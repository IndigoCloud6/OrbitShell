mod ssh;
mod storage;
mod credentials;
mod launch;
mod mcp;
mod executions;
mod quick_commands;

use anyhow::{anyhow, Context};
use russh_sftp::protocol::OpenFlags;
use serde::Serialize;
use ssh::{ConnectRequest, LiveSession, SessionCommand, SessionEvent, SessionEvents, OutputBuffer};
use std::{collections::HashMap, sync::{Arc, Mutex, atomic::AtomicBool}, time::Duration};
use storage::{ConnectionProfile, Storage};
use tauri::{ipc::Channel, AppHandle, Manager, State};
use tokio::{io::{AsyncReadExt, AsyncWriteExt}, sync::{mpsc, OnceCell}, time::timeout};
use tokio_util::sync::CancellationToken;

pub struct AppState {
    storage: Storage,
    sessions: Mutex<HashMap<String, Arc<LiveSession>>>,
    launches: Mutex<Vec<launch::PendingLaunch>>,
    mcp_enabled: AtomicBool,
    mcp_runtime: tokio::sync::Mutex<Option<mcp::RunningMcp>>,
    executions: executions::ExecutionStore,
}

impl AppState {
    fn session(&self, id: &str) -> Result<Arc<LiveSession>, String> {
        self.sessions.lock().map_err(|_| "会话状态不可用")?
            .get(id).cloned().ok_or_else(|| "连接已关闭，请重新连接".into())
    }
}

#[tauri::command]
fn list_connections(state: State<'_, AppState>) -> Result<Vec<ConnectionProfile>, String> {
    Ok(state.storage.store.lock().map_err(|_| "配置暂不可用")?.connections.clone())
}

#[tauri::command]
fn save_connection(mut profile: ConnectionProfile, state: State<'_, AppState>) -> Result<ConnectionProfile, String> {
    profile.validate()?;
    let mut store = state.storage.store.lock().map_err(|_| "配置暂不可用")?;
    if let Some(existing) = store.connections.iter().find(|entry| entry.id == profile.id) {
        if !credentials::same_identity(existing, &profile) { credentials::forget(&profile.id)?; }
    }
    let mut next = store.clone();
    if let Some(existing) = next.connections.iter_mut().find(|entry| entry.id == profile.id) {
        *existing = profile.clone();
    } else {
        next.connections.push(profile.clone());
    }
    state.storage.persist(&next).map_err(|error| format!("保存失败：{error}"))?;
    *store = next;
    Ok(profile)
}

#[tauri::command]
fn delete_connection(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let mut store = state.storage.store.lock().map_err(|_| "配置暂不可用")?;
    credentials::forget(&id)?;
    let mut next = store.clone();
    next.connections.retain(|profile| profile.id != id);
    state.storage.persist(&next).map_err(|error| format!("保存失败：{error}"))?;
    *store = next;
    Ok(())
}

#[derive(Serialize)]
struct CredentialStatus { available: bool, saved: bool }

#[tauri::command]
fn credential_status(profile_id: Option<String>, state: State<'_, AppState>) -> Result<CredentialStatus, String> {
    let Some(profile_id) = profile_id else { return Ok(CredentialStatus {available: credentials::AVAILABLE, saved: false}); };
    let store = state.storage.store.lock().map_err(|_| "配置暂不可用")?;
    let profile = store.connections.iter().find(|profile| profile.id == profile_id).ok_or("连接配置不存在")?;
    Ok(CredentialStatus { available: credentials::AVAILABLE, saved: credentials::load(profile)?.is_some() })
}

#[tauri::command]
fn save_connection_credential(profile_id: String, credential: String, state: State<'_, AppState>) -> Result<(), String> {
    let secret = zeroize::Zeroizing::new(credential);
    let store = state.storage.store.lock().map_err(|_| "配置暂不可用")?;
    let profile = store.connections.iter().find(|profile| profile.id == profile_id).ok_or("连接配置不存在")?;
    credentials::save(profile, secret.as_str())
}

#[tauri::command]
fn forget_connection_credential(profile_id: String, state: State<'_, AppState>) -> Result<(), String> {
    let _store = state.storage.store.lock().map_err(|_| "配置暂不可用")?;
    credentials::forget(&profile_id)
}

pub(crate) fn remember_session_credential(app: &AppHandle, profile: &ConnectionProfile, secret: &str) -> Result<(), String> {
    let state = app.state::<AppState>();
    let store = state.storage.store.lock().map_err(|_| "配置暂不可用")?;
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
        None => state.storage.store.lock().map_err(|_| "配置暂不可用")?.connections
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
    let (sender, receiver) = mpsc::channel(128);
    let live = Arc::new(LiveSession {
        profile: profile.clone(), is_temporary: request.profile.is_some(),
        connected: AtomicBool::new(false), output: Mutex::new(OutputBuffer::new()),
        commands: sender, cancel: CancellationToken::new(), trust: Mutex::new(None),
        verification_error: Mutex::new(None), connection: OnceCell::new(), sftp: OnceCell::new(),
    });
    let mut sessions = state.sessions.lock().map_err(|_| "会话状态不可用")?;
    if sessions.contains_key(&request.session_id) { return Err("该会话已存在".into()); }
    events.announce(&profile).map_err(|error| error.to_string())?;
    sessions.insert(request.session_id.clone(), live.clone());
    drop(sessions);
    let id = request.session_id.clone();
    tauri::async_runtime::spawn(ssh::run_session(app.clone(), live, request, profile, events, receiver));
    Ok(id)
}

#[tauri::command]
async fn ssh_write(session_id: String, data: Vec<u8>, state: State<'_, AppState>) -> Result<(), String> {
    if data.len() > 1024 * 1024 { return Err("单次输入内容过大，请分批粘贴".into()); }
    state.session(&session_id)?.commands.send(SessionCommand::Input(data)).await.map_err(|_| "连接已关闭".into())
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
    let answer = live.trust.lock().map_err(|_| "会话状态不可用")?.take().ok_or("确认已过期，请重新连接")?;
    answer.send(accept).map_err(|_| "确认已过期，请重新连接".into())
}

#[tauri::command]
fn ssh_disconnect(session_id: String, state: State<'_, AppState>) -> Result<(), String> {
    if let Ok(session) = state.session(&session_id) { session.cancel.cancel(); }
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RemoteEntry { name: String, path: String, is_directory: bool, is_symlink: bool, size: u64 }

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RemoteDirectory { path: String, entries: Vec<RemoteEntry> }

#[tauri::command]
async fn sftp_list(session_id: String, path: String, state: State<'_, AppState>) -> Result<RemoteDirectory, String> {
    let live = state.session(&session_id)?;
    let result = tokio::select! {
        _ = live.cancel.cancelled() => Err(anyhow!("连接已关闭")),
        result = async {
            let sftp = live.sftp().await?;
            timeout(Duration::from_secs(30), async {
                let path = sftp.canonicalize(if path.is_empty() { "." } else { &path }).await?;
                let mut entries: Vec<_> = sftp.read_dir(&path).await?.map(|entry| {
                    let meta = entry.metadata();
                    RemoteEntry { name: entry.file_name(), path: entry.path(), is_directory: meta.is_dir(), is_symlink: meta.is_symlink(), size: meta.len() }
                }).collect();
                entries.sort_by(|left, right| right.is_directory.cmp(&left.is_directory).then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase())));
                Ok::<_, anyhow::Error>(RemoteDirectory { path, entries })
            }).await.context("读取目录超时，请重试")?
        } => result,
    };
    result.map_err(|error| format!("{error:#}"))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TransferResult { name: String, bytes: u64 }

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TransferProgress { name: String, bytes: u64, total: Option<u64> }

async fn copy_with_progress<R, W>(source: &mut R, destination: &mut W, name: &str, total: Option<u64>, progress: &Channel<TransferProgress>) -> std::io::Result<u64>
where R: tokio::io::AsyncRead + Unpin, W: tokio::io::AsyncWrite + Unpin {
    let mut buffer = vec![0u8; 64 * 1024];
    let mut bytes = 0u64;
    let mut last = std::time::Instant::now();
    let report = |bytes| { let _ = progress.send(TransferProgress {name: name.to_owned(), bytes, total}); };
    report(0);
    loop {
        let count = source.read(&mut buffer).await?;
        if count == 0 { break; }
        destination.write_all(&buffer[..count]).await?;
        bytes += count as u64;
        if last.elapsed() >= Duration::from_millis(100) { report(bytes); last = std::time::Instant::now(); }
    }
    report(bytes);
    Ok(bytes)
}

#[tauri::command]
async fn sftp_download(session_id: String, path: String, progress: Channel<TransferProgress>, state: State<'_, AppState>) -> Result<Option<TransferResult>, String> {
    let live = state.session(&session_id)?;
    let name = path.rsplit('/').next().filter(|name| !name.is_empty()).ok_or("文件路径无效")?.to_owned();
    let Some(destination) = rfd::AsyncFileDialog::new().set_title("下载远程文件").set_file_name(&name).save_file().await else { return Ok(None); };
    let result = tokio::select! {
        _ = live.cancel.cancelled() => Err(anyhow!("连接已关闭，下载已停止")),
        result = async {
            let sftp = live.sftp().await?;
            let mut source = sftp.open(&path).await?;
            let total = source.metadata().await.ok().map(|metadata| metadata.len());
            let parent = destination.path().parent().ok_or_else(|| anyhow!("保存目录无效"))?;
            let temporary = tempfile::NamedTempFile::new_in(parent)?;
            let mut output = tokio::fs::File::from_std(temporary.reopen()?);
            let bytes = copy_with_progress(&mut source, &mut output, &name, total, &progress).await?;
            output.flush().await?;
            output.sync_all().await?;
            drop(output);
            source.close().await?;
            temporary.persist(destination.path())?;
            Ok::<_, anyhow::Error>(Some(TransferResult { name, bytes }))
        } => result,
    };
    result.map_err(|error| format!("下载失败：{error:#}"))
}

#[tauri::command]
async fn sftp_upload(session_id: String, directory: String, progress: Channel<TransferProgress>, state: State<'_, AppState>) -> Result<Option<TransferResult>, String> {
    let live = state.session(&session_id)?;
    let Some(source) = rfd::AsyncFileDialog::new().set_title("上传文件到远程目录").pick_file().await else { return Ok(None); };
    let name = source.path().file_name().ok_or("文件名无效")?.to_string_lossy().into_owned();
    let result = tokio::select! {
        _ = live.cancel.cancelled() => Err(anyhow!("连接已关闭，上传已停止")),
        result = async {
            let sftp = live.sftp().await?;
            let directory = sftp.canonicalize(directory).await?;
            let path = format!("{}/{name}", directory.trim_end_matches('/'));
            let mut input = tokio::fs::File::open(source.path()).await?;
            let total = Some(input.metadata().await?.len());
            // Exclusive create preserves existing remote files.
            let mut output = sftp.open_with_flags(&path, OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::EXCLUDE).await
                .context("无法创建远程文件；若存在同名文件，请先重命名")?;
            let result = async {
                let bytes = copy_with_progress(&mut input, &mut output, &name, total, &progress).await?;
                output.flush().await?;
                output.close().await?;
                Ok::<_, anyhow::Error>(bytes)
            }.await;
            match result {
                Ok(bytes) => Ok(Some(TransferResult { name, bytes })),
                Err(error) => { let _ = sftp.remove_file(path).await; Err(error) }
            }
        } => result,
    };
    result.map_err(|error| format!("上传失败：{error:#}"))
}

pub fn run() {
    use tauri_plugin_deep_link::DeepLinkExt;
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, arguments, _working_directory| {
            launch::ingest_arguments(app, arguments);
        }))
        .plugin(tauri_plugin_deep_link::init())
        .setup(|app| {
            let storage = Storage::open(app.path().app_data_dir()?)?;
            {
                let mut store = storage.store.lock().map_err(|_| anyhow!("配置暂不可用"))?;
                if store.mcp_token.is_empty() {
                    store.mcp_token = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
                    storage.persist(&store)?;
                }
            }
            app.manage(AppState { storage, sessions: Mutex::new(HashMap::new()), launches: Mutex::new(Vec::new()),
                mcp_enabled: AtomicBool::new(false), mcp_runtime: tokio::sync::Mutex::new(None), executions: executions::ExecutionStore::default() });
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
            if matches!(event, tauri::WindowEvent::CloseRequested { .. } | tauri::WindowEvent::Destroyed) {
                let state = window.state::<AppState>();
                state.mcp_enabled.store(false, std::sync::atomic::Ordering::Release);
                if let Ok(sessions) = state.sessions.lock() {
                    for session in sessions.values() { session.cancel.cancel(); }
                }
                if let Ok(mut runtime) = state.mcp_runtime.try_lock() {
                    if let Some(server) = runtime.take() { server.stop(); }
                };
            }
        })
        .invoke_handler(tauri::generate_handler![list_connections, save_connection, delete_connection, pick_private_key,
            credential_status, save_connection_credential, forget_connection_credential,
            ssh_connect, ssh_write, ssh_resize, ssh_acknowledge, ssh_trust_host, ssh_disconnect,
            sftp_list, sftp_download, sftp_upload, mcp::mcp_status, mcp::mcp_set_enabled,
            executions::execution_list, executions::execution_output, executions::execution_clear,
            quick_commands::list_quick_commands, quick_commands::save_quick_command, quick_commands::delete_quick_command,
            quick_commands::save_quick_command_group, quick_commands::delete_quick_command_group,
            launch::open_external_link, launch::take_launch_requests, launch::register_ssh_protocol])
        .run(tauri::generate_context!())
        .expect("OrbitShell could not start");
}
