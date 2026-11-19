mod ssh;
mod storage;
mod launch;
mod mcp;
mod executions;
mod ai_terminals;
mod quick_commands;
mod remote_editor;
mod desktop;
mod recent;
mod command_history;
mod command_guard;
mod clipboard;
mod sftp_transfer;
mod file_transfers;
mod manual_upload;
mod file_drag;

use anyhow::{anyhow, Context};
use orbitshell_ssh::{credentials, sftp::RemoteDirectory};
use sftp_transfer::TransferProgress;
use serde::Serialize;
use ssh::{ConnectRequest, LiveSession, SessionCommand, SessionEvent, SessionEvents};
use std::{collections::HashMap, sync::{Arc, Mutex, atomic::{AtomicBool, AtomicU64}}};
use storage::{ConnectionProfile, Storage};
use tauri::{ipc::Channel, AppHandle, Manager, State};
use tokio_util::sync::CancellationToken;

pub struct AppState {
    storage: Storage,
    sessions: Mutex<HashMap<String, Arc<LiveSession>>>,
    launches: Mutex<Vec<launch::PendingLaunch>>,
    mcp_enabled: AtomicBool,
    mcp_epoch: AtomicU64,
    mcp_runtime: tokio::sync::Mutex<Option<mcp::RunningMcp>>,
    mcp_startup_error: Mutex<Option<String>>,
    history: command_history::History,
    executions: executions::ExecutionStore,
    ai: ai_terminals::AiTerminals,
    editor: remote_editor::EditorStore,
    desktop: desktop::DesktopState,
    recent: recent::RecentState,
    transfers: file_transfers::Transfers,
    upload_confirmations: manual_upload::Confirmations,
}

impl AppState {
    fn session(&self, id: &str) -> Result<Arc<LiveSession>, String> {
        self.sessions.lock().map_err(|_| "会话状态不可用")?
            .get(id).cloned().ok_or_else(|| "连接已关闭，请重新连接".into())
    }
}

#[tauri::command]
fn list_connections(state: State<'_, AppState>) -> Result<Vec<ConnectionProfile>, String> {
    Ok(state.storage.lock().map_err(|_| "配置暂不可用")?.connections.clone())
}

#[tauri::command]
fn save_connection(mut profile: ConnectionProfile, state: State<'_, AppState>, app: AppHandle) -> Result<ConnectionProfile, String> {
    profile.validate()?;
    let mut store = state.storage.lock().map_err(|_| "配置暂不可用")?;
    let identity_changed = store.connections.iter().find(|entry| entry.id == profile.id)
        .is_some_and(|existing| !credentials::same_identity(existing, &profile));
    if identity_changed { credentials::forget(&profile.id)?; }
    let mut next = store.clone();
    if identity_changed { next.recent_connections.retain(|stamp| stamp.profile_id != profile.id); }
    if let Some(existing) = next.connections.iter_mut().find(|entry| entry.id == profile.id) {
        *existing = profile.clone();
    } else {
        next.connections.push(profile.clone());
    }
    state.storage.persist(&next).map_err(|error| format!("保存失败：{error}"))?;
    *store = next;
    if identity_changed { recent::forget_profile(&state, &profile.id); }
    drop(store);
    recent::notify(&app);
    Ok(profile)
}

#[tauri::command]
fn delete_connection(id: String, state: State<'_, AppState>, app: AppHandle) -> Result<(), String> {
    let mut store = state.storage.lock().map_err(|_| "配置暂不可用")?;
    credentials::forget(&id)?;
    let mut next = store.clone();
    next.connections.retain(|profile| profile.id != id);
    next.recent_connections.retain(|stamp| stamp.profile_id != id);
    state.storage.persist(&next).map_err(|error| format!("保存失败：{error}"))?;
    *store = next;
    recent::forget_profile(&state, &id);
    drop(store);
    recent::notify(&app);
    Ok(())
}

#[derive(Serialize)]
struct CredentialStatus { available: bool, saved: bool }

#[tauri::command]
fn credential_status(profile_id: Option<String>, state: State<'_, AppState>) -> Result<CredentialStatus, String> {
    let Some(profile_id) = profile_id else { return Ok(CredentialStatus {available: credentials::AVAILABLE, saved: false}); };
    let store = state.storage.lock().map_err(|_| "配置暂不可用")?;
    let profile = store.connections.iter().find(|profile| profile.id == profile_id).ok_or("连接配置不存在")?;
    Ok(CredentialStatus { available: credentials::AVAILABLE, saved: credentials::load(profile)?.is_some() })
}

#[tauri::command]
fn save_connection_credential(profile_id: String, credential: String, state: State<'_, AppState>) -> Result<(), String> {
    let secret = zeroize::Zeroizing::new(credential);
    let store = state.storage.lock().map_err(|_| "配置暂不可用")?;
    let profile = store.connections.iter().find(|profile| profile.id == profile_id).ok_or("连接配置不存在")?;
    credentials::save(profile, secret.as_str())
}

#[tauri::command]
fn forget_connection_credential(profile_id: String, state: State<'_, AppState>) -> Result<(), String> {
    let _store = state.storage.lock().map_err(|_| "配置暂不可用")?;
    credentials::forget(&profile_id)
}

pub(crate) fn remember_session_credential(app: &AppHandle, profile: &ConnectionProfile, secret: &str) -> Result<(), String> {
    let state = app.state::<AppState>();
    let store = state.storage.lock().map_err(|_| "配置暂不可用")?;
    let current = store.connections.iter().find(|entry| entry.id == profile.id)
        .ok_or("连接配置已删除，未保存密码")?;
    if !credentials::same_identity(current, profile) { return Err("连接配置已改变，未保存密码，请重新确认".into()); }
    credentials::save(current, secret)
}

#[tauri::command]
async fn pick_private_key() -> Result<Option<String>, String> {
    Ok(rfd::AsyncFileDialog::new().set_title("选择 SSH 私钥（OpenSSH 或 PEM 格式）")
        .pick_file().await.map(|file| file.path().to_string_lossy().into_owned()))
}

#[tauri::command]
fn ssh_connect(request: ConnectRequest, events: Channel<SessionEvent>, app: AppHandle) -> Result<(), String> {
    uuid::Uuid::parse_str(&request.session_id).map_err(|_| "会话标识无效")?;
    start_session(app, request, SessionEvents::channel(events))?;
    Ok(())
}

pub(crate) fn start_session(app: AppHandle, mut request: ConnectRequest, events: SessionEvents) -> Result<String, String> {
    uuid::Uuid::parse_str(&request.session_id).map_err(|_| "会话标识无效")?;
    let state = app.state::<AppState>();
    let mut profile = match &request.profile {
        Some(profile) => profile.clone(),
        None => state.storage.lock().map_err(|_| "配置暂不可用")?.connections
            .iter().find(|profile| profile.id == request.profile_id).cloned().ok_or("连接配置不存在")?,
    };
    profile.validate()?;
    if request.profile.is_some() && (request.use_saved_credential || request.remember_credential) {
        return Err("临时会话不能读取或保存连接密码".into());
    }
    if request.use_saved_credential {
        let mut secret = credentials::load(&profile)?.ok_or("未找到这台服务器的已保存密码，请重新输入")?;
        use zeroize::Zeroize;
        request.credential.zeroize();
        request.credential = std::mem::take(&mut *secret);
    }
    let (live, receiver) = LiveSession::new(profile.clone(), request.profile.is_some());
    let mut sessions = state.sessions.lock().map_err(|_| "会话状态不可用")?;
    if sessions.contains_key(&request.session_id) { return Err("该会话已存在".into()); }
    events.announce(&profile).map_err(|error| error.to_string())?;
    sessions.insert(request.session_id.clone(), live.clone());
    drop(sessions);
    let id = request.session_id.clone();
    tauri::async_runtime::spawn(ssh::run_session(app, live, request, events, receiver));
    Ok(id)
}

#[tauri::command]
async fn ssh_write(session_id: String, data: Vec<u8>, history_command: Option<String>, history_id: Option<String>, state: State<'_, AppState>) -> Result<(), String> {
    if data.len() > 1024 * 1024 { return Err("单次输入内容过大，请分批粘贴".into()); }
    if state.ai.taken_over(&session_id) { return Err("AI_SESSION_OWNED: 当前终端由 AI 接管，请先交回人工".into()); }
    let history = match (history_id, history_command) {
        (Some(id), Some(command)) if command.len() <= 64 * 1024 && uuid::Uuid::parse_str(&id).is_ok() => Some((id, command)),
        _ => None,
    };
    state.session(&session_id)?.commands.send(SessionCommand::Input { data, history }).await.map_err(|_| "连接已关闭".into())
}

#[tauri::command]
async fn ssh_resize(session_id: String, columns: u32, rows: u32, state: State<'_, AppState>) -> Result<(), String> {
    state.session(&session_id)?.commands.send(SessionCommand::Resize(columns, rows)).await.map_err(|_| "连接已关闭".into())
}

#[tauri::command]
async fn ssh_acknowledge(session_id: String, bytes: usize, state: State<'_, AppState>) -> Result<(), String> {
    state.session(&session_id)?.commands.send(SessionCommand::Acknowledge(bytes)).await.map_err(|_| "连接已关闭".into())
}

#[tauri::command]
fn ssh_trust_host(session_id: String, accept: bool, state: State<'_, AppState>) -> Result<(), String> {
    let live = state.session(&session_id)?;
    live.trust_host(accept)
}

#[tauri::command]
fn ssh_disconnect(session_id: String, state: State<'_, AppState>) -> Result<(), String> {
    if let Ok(session) = state.session(&session_id) { session.cancel.cancel(); }
    Ok(())
}

#[tauri::command]
async fn sftp_list(session_id: String, path: String, state: State<'_, AppState>) -> Result<RemoteDirectory, String> {
    let live = state.session(&session_id)?;
    orbitshell_ssh::sftp::list(&live, &path).await.map_err(|error| format!("{error:#}"))
}

#[tauri::command]
async fn sftp_delete_entry(
    session_id: String,
    directory: String,
    path: String,
    is_directory: bool,
    is_symlink: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let live = state.session(&session_id)?;
    orbitshell_ssh::sftp::delete_entry(&live, &directory, &path, is_directory, is_symlink).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TransferResult { name: String, bytes: u64 }

#[tauri::command]
async fn sftp_download(session_id: String, transfer_id: String, path: String, progress: Channel<TransferProgress>, app: AppHandle, state: State<'_, AppState>) -> Result<Option<TransferResult>, String> {
    let live = state.session(&session_id)?;
    let name = path.rsplit('/').next().filter(|name| !name.is_empty()).ok_or("文件路径无效")?.to_owned();
    let Some(destination) = rfd::AsyncFileDialog::new().set_title("下载远程文件").set_file_name(&name).save_file().await else { return Ok(None); };
    let task = state.transfers.manual(&app, transfer_id, &live, &session_id, file_transfers::Direction::Download,
        name, destination.path().to_string_lossy().into_owned(), path.clone())?;
    let cancel = CancellationToken::new();
    let result = sftp_transfer::download(&live, &path, destination.path(), true, |event| {
        task.progress(&app, event.clone()); let _ = progress.send(event);
    }, &cancel).await;
    state.transfers.finish(&app, &task, &live, &result);
    result.map(|outcome| Some(TransferResult { name: outcome.name, bytes: outcome.bytes }))
        .map_err(|error| format!("下载失败：{error:#}"))
}

pub fn run() {
    use tauri_plugin_deep_link::DeepLinkExt;
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, arguments, _working_directory| {
            launch::ingest_arguments(app, arguments);
        }))
        .plugin(tauri_plugin_deep_link::init())
        .on_page_load(|webview, payload| {
            if webview.label() == "main" && matches!(payload.event(), tauri::webview::PageLoadEvent::Started) {
                desktop::renderer_loading(webview.app_handle());
            }
        })
        .setup(|app| {
            let storage = Storage::open(app.path().app_data_dir()?)?;
            let history = command_history::History::open(app.path().app_data_dir()?);
            let mcp_auto_start = storage.lock().map_err(|_| anyhow!("配置暂不可用"))?.mcp_auto_start;
            let close_to_tray = {
                let mut store = storage.lock().map_err(|_| anyhow!("配置暂不可用"))?;
                if store.mcp_token.is_empty() {
                    store.mcp_token = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
                    storage.persist(&store)?;
                }
                store.desktop.close_to_tray
            };
            app.manage(AppState { storage, sessions: Mutex::new(HashMap::new()), launches: Mutex::new(Vec::new()),
                mcp_enabled: AtomicBool::new(false), mcp_epoch: AtomicU64::new(0), mcp_runtime: tokio::sync::Mutex::new(None), mcp_startup_error: Mutex::new(None), history, executions: executions::ExecutionStore::default(), ai: ai_terminals::AiTerminals::default(), editor: remote_editor::EditorStore::default(), desktop: desktop::DesktopState::new(close_to_tray), recent: recent::RecentState::default(), transfers: file_transfers::Transfers::open(app.path().app_data_dir()?), upload_confirmations: manual_upload::Confirmations::default() });
            if mcp_auto_start {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let epoch = handle.state::<AppState>().mcp_epoch.load(std::sync::atomic::Ordering::Acquire);
                    if let Err(error) = mcp::mcp_set_enabled(true, handle.clone()).await {
                        let state = handle.state::<AppState>();
                        if let Ok(mut status) = state.mcp_startup_error.lock() { if state.mcp_epoch.load(std::sync::atomic::Ordering::Acquire) == epoch.wrapping_add(1) { *status = Some(error); } }
                        mcp::notify_status(&handle);
                    }
                });
            }
            desktop::install(app.handle());
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            launch::ingest_arguments(app.handle(), std::env::args().skip(1).collect());
            #[cfg(target_os = "macos")]
            if let Some(urls) = app.deep_link().get_current()? {
                for url in urls {
                    if let Ok(launch) = launch::parse_link(url.as_str(), "preserve", None) { let _ = launch::enqueue(app.handle(), launch); }
                }
            }
            let handle = app.handle().clone();
            app.deep_link().on_open_url(move |event| {
                for url in event.urls() {
                    match launch::parse_link(url.as_str(), "preserve", None).and_then(|launch| launch::enqueue(&handle, launch)) {
                        Ok(()) => {},
                        Err(error) => { use tauri::Emitter; let _ = handle.emit_to("main", "orbitshell:launch-error", error); }
                    }
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() != "main" { return; }
            match event {
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    if !window.state::<AppState>().desktop.exiting.load(std::sync::atomic::Ordering::Acquire) {
                        api.prevent_close();
                        let _ = desktop::close_main(window.app_handle());
                    }
                },
                tauri::WindowEvent::Destroyed => desktop::shutdown(window.app_handle()),
                _ => {},
            }
        })
        .invoke_handler(tauri::generate_handler![list_connections, save_connection, delete_connection, pick_private_key,
            clipboard::clipboard_read_text, clipboard::clipboard_write_text,
            credential_status, save_connection_credential, forget_connection_credential,
            ssh_connect, ssh_write, ssh_resize, ssh_acknowledge, ssh_trust_host, ssh_disconnect,
            sftp_list, sftp_delete_entry, sftp_download, manual_upload::sftp_upload, manual_upload::sftp_confirm_upload,
            file_drag::sftp_drag_download, mcp::mcp_status, mcp::mcp_set_enabled,
            file_transfers::file_transfer_list,
            remote_editor::editor_open, remote_editor::editor_read, remote_editor::editor_save, remote_editor::editor_close,
            remote_editor::editor_snapshots, remote_editor::editor_snapshot_read,
            desktop::desktop_status, desktop::desktop_set_close_to_tray, desktop::desktop_close_window, desktop::desktop_hide_window,
            desktop::desktop_request_exit, desktop::desktop_confirm_exit, desktop::desktop_cancel_exit,
            desktop::desktop_register_exit_guard, desktop::desktop_unregister_exit_guard,
            executions::execution_list, executions::execution_output, executions::execution_clear,
            ai_terminals::ai_workspace, ai_terminals::ai_terminal_output, ai_terminals::ai_terminal_resize,
            ai_terminals::ai_release_session,
            ai_terminals::ai_take_over_session,
            ai_terminals::ai_set_active_session,
            mcp::mcp_set_auto_start, command_history::command_history, command_history::command_history_record, command_history::command_history_retry,
            quick_commands::list_quick_commands, quick_commands::save_quick_command, quick_commands::delete_quick_command,
            quick_commands::save_quick_command_group, quick_commands::delete_quick_command_group,
            recent::list_recent_connections, launch::open_quick_connection,
            launch::open_external_link, launch::take_launch_requests, launch::register_ssh_protocol])
        .run(tauri::generate_context!())
        .expect("OrbitShell could not start");
}
