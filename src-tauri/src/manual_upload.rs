//! Desktop uploads: a saved destination is replaced only after an explicit UI decision.
//! MCP keeps using sftp_transfer::upload's exclusive-create path.
use crate::{file_transfers::{Direction, Task}, sftp_transfer::{self, Outcome, TransferProgress}, ssh::LiveSession, AppState, TransferResult};
use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use std::{collections::HashMap, path::{Path, PathBuf}, sync::Mutex, time::Duration};
use tauri::{AppHandle, Manager, State, ipc::Channel};
use tokio::{sync::oneshot, time::timeout};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Default)]
pub struct Confirmations(Mutex<HashMap<String, oneshot::Sender<bool>>>);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Conflict { token: String, name: String, remote_path: String, session_name: String }
#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum UploadEvent { Progress { progress: TransferProgress }, Conflict { conflict: Conflict } }

struct PendingDecision { app: AppHandle, token: String }
impl Drop for PendingDecision {
    fn drop(&mut self) {
        if let Ok(mut pending) = self.app.state::<AppState>().upload_confirmations.0.lock() { pending.remove(&self.token); }
    }
}

#[tauri::command]
pub fn sftp_confirm_upload(token: String, overwrite: bool, state: State<'_, AppState>) -> Result<(), String> {
    let answer = state.upload_confirmations.0.lock().map_err(|_| "上传确认状态不可用")?
        .remove(&token).ok_or("此次覆盖确认已结束，请重新发起上传")?;
    answer.send(overwrite).map_err(|_| "此次上传已结束，未执行覆盖".into())
}

async fn confirm(app: &AppHandle, live: &LiveSession, task: &Task, name: &str, path: &str, events: &Channel<UploadEvent>) -> Result<bool> {
    let token = Uuid::new_v4().to_string();
    let (send, receive) = oneshot::channel();
    app.state::<AppState>().upload_confirmations.0.lock().map_err(|_| anyhow!("上传确认状态不可用"))?.insert(token.clone(), send);
    let _pending = PendingDecision { app: app.clone(), token: token.clone() };
    task.message(app, "等待确认是否覆盖同名远程文件");
    events.send(UploadEvent::Conflict { conflict: Conflict { token, name: name.into(), remote_path: path.into(), session_name: live.profile.name.clone() } })?;
    tokio::select! {
        _ = live.cancel.cancelled() => bail!("所属 SSH 会话已断开"),
        answer = timeout(Duration::from_secs(900), receive) => {
            let answer = answer.context("等待覆盖确认超时，未覆盖原文件")?.context("覆盖确认已关闭")?;
            if !answer { task.cancel(); }
            Ok(answer)
        }
    }
}

async fn upload(app: &AppHandle, live: &LiveSession, task: &Task, source: &Path, directory: &str, name: &str, events: &Channel<UploadEvent>) -> Result<Outcome> {
    let cancel = CancellationToken::new();
    sftp_transfer::upload_with_confirmation(
        live, source, directory, name,
        |event| { task.progress(app, event.clone()); let _ = events.send(UploadEvent::Progress { progress: event }); },
        |target| async move { confirm(app, live, task, name, &target, events).await },
        || task.message(app, "正在提交覆盖文件"),
        &cancel,
    ).await
}
#[tauri::command]
pub async fn sftp_upload(session_id: String, transfer_id: String, directory: String, local_path: Option<String>, progress: Channel<UploadEvent>, app: AppHandle) -> Result<Option<TransferResult>, String> {
    let live = app.state::<AppState>().session(&session_id)?;
    let source = if let Some(path) = local_path { PathBuf::from(path) } else {
        let Some(file) = rfd::AsyncFileDialog::new().set_title("上传文件到远程目录").pick_file().await else { return Ok(None); };
        file.path().to_owned()
    };
    if !source.is_absolute() || !tokio::fs::metadata(&source).await.map_err(|error| format!("无法读取本机文件：{error}"))?.is_file() { return Err("只支持拖入或选择本机普通文件，暂不支持上传目录".into()); }
    let name = source.file_name().and_then(|name| name.to_str()).filter(|name| !name.chars().any(|c| c.is_control() || matches!(c, '/' | '\\'))).ok_or("文件名无效")?.to_owned();
    let task = app.state::<AppState>().transfers.manual(&app, transfer_id, &live, &session_id, Direction::Upload,
        name.clone(), source.to_string_lossy().into_owned(), format!("{}/{name}", directory.trim_end_matches('/')))?;
    let result = tokio::select! {
        _ = live.cancel.cancelled() => Err(anyhow!("所属 SSH 会话已断开")),
        result = upload(&app, &live, &task, &source, &directory, &name, &progress) => result,
    };
    app.state::<AppState>().transfers.finish(&app, &task, &live, &result);
    if task.cancelled() { return Ok(None); }
    result.map(|outcome| Some(TransferResult { name: outcome.name, bytes: outcome.bytes })).map_err(|error| format!("上传失败：{error:#}"))
}
