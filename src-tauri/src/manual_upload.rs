//! Desktop uploads: a saved destination is replaced only after an explicit UI decision.
//! MCP keeps using sftp_transfer::upload's exclusive-create path.
use crate::{file_transfers::{Direction, Task}, sftp_transfer::{self, Outcome, TransferProgress}, ssh::LiveSession, AppState, TransferResult};
use anyhow::{anyhow, bail, Context, Result};
use russh::{ChannelMsg, client};
use russh_sftp::{client::{RawSftpSession, SftpSession}, protocol::{Packet, StatusCode, FileAttributes}};
use serde::Serialize;
use std::{collections::HashMap, path::{Path, PathBuf}, sync::{Arc, Mutex}, time::Duration};
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

struct RemoteTemporary { sftp: Arc<SftpSession>, path: String, armed: bool }
impl Drop for RemoteTemporary {
    fn drop(&mut self) {
        if self.armed {
            let sftp = self.sftp.clone(); let path = self.path.clone();
            tauri::async_runtime::spawn(async move { let _ = timeout(Duration::from_secs(5), sftp.remove_file(path)).await; });
        }
    }
}

async fn rename_channel(connection: &client::Handle<crate::ssh::SshHandler>) -> Result<RawSftpSession> {
    let mut channel = connection.channel_open_session().await?;
    channel.request_subsystem(true, "sftp").await?;
    loop {
        match channel.wait().await {
            Some(ChannelMsg::Success) => break,
            Some(ChannelMsg::Failure | ChannelMsg::Close) | None => bail!("服务器未接受文件替换通道"),
            _ => {}
        }
    }
    let raw = RawSftpSession::new(channel.into_stream());
    let version = raw.init().await?;
    if version.extensions.get("posix-rename@openssh.com").map(String::as_str) != Some("1") {
        let _ = raw.close_session();
        bail!("服务器不支持安全原子覆盖，原文件未修改；请取消或更换上传文件名");
    }
    Ok(raw)
}

async fn upload(app: &AppHandle, live: &LiveSession, task: &Task, source: &Path, directory: &str, name: &str, events: &Channel<UploadEvent>) -> Result<Outcome> {
    let cancel = CancellationToken::new();
    let report = |event: TransferProgress| { task.progress(app, event.clone()); let _ = events.send(UploadEvent::Progress { progress: event }); };
    let sftp = live.sftp().await?;
    let directory = timeout(Duration::from_secs(60), sftp.canonicalize(directory)).await.context("读取远程目录超时")??;
    let target = format!("{}/{name}", directory.trim_end_matches('/'));
    let before = match timeout(Duration::from_secs(60), sftp.symlink_metadata(&target)).await.context("读取目标文件超时")? {
        Ok(metadata) => metadata,
        Err(russh_sftp::client::error::Error::Status(status)) if status.status_code == StatusCode::NoSuchFile => {
            match sftp_transfer::upload(live, source, &directory, name, None, &report, &cancel).await {
                Ok(outcome) => return Ok(outcome),
                Err(error) if error.is::<sftp_transfer::UploadCreateRejected>() => {
                    // A file may have appeared after the first lookup. Still ask
                    // before replacing it, without retrying a write or a timeout.
                    match timeout(Duration::from_secs(60), sftp.symlink_metadata(&target)).await {
                        Ok(Ok(metadata)) => metadata,
                        _ => return Err(error),
                    }
                },
                Err(error) => return Err(error),
            }
        },
        Err(error) => return Err(error.into()),
    };
    if !before.is_regular() || before.is_symlink() { bail!("同名目标不是普通文件，不能覆盖目录或符号链接"); }
    if !confirm(app, live, task, name, &target, events).await? { bail!("用户取消覆盖"); }
    let connection = live.connection.get().ok_or_else(|| anyhow!("会话已断开"))?;
    let raw = timeout(Duration::from_secs(25), rename_channel(connection)).await.context("建立文件替换通道超时")??;
    let result = async {
        let temporary_name = format!(".orbitshell-upload-{}.tmp", Uuid::new_v4());
        let mut temporary = RemoteTemporary { sftp: sftp.clone(), path: format!("{}/{temporary_name}", directory.trim_end_matches('/')), armed: false };
        let uploaded = sftp_transfer::upload(live, source, &directory, &temporary_name, Some(0o600), |mut event| { event.name = name.into(); report(event); }, &cancel).await?;
        temporary.armed = true;
        // Match the existing owner's permissions before committing the complete file.
        let owner = sftp.symlink_metadata(&temporary.path).await?;
        let ownership_changed = owner.uid != before.uid || owner.gid != before.gid;
        if ownership_changed && (before.uid.is_none() || before.gid.is_none()) { bail!("无法确认原文件的属主与属组，已停止覆盖"); }
        sftp.set_metadata(&temporary.path, FileAttributes { uid: if ownership_changed { before.uid } else { None },
            gid: if ownership_changed { before.gid } else { None }, permissions: before.permissions, ..FileAttributes::empty() }).await
            .context("无法保留原文件的权限或属主，已停止覆盖")?;
        let latest = sftp.symlink_metadata(&target).await?;
        if !latest.is_regular() || latest.is_symlink() || latest.size != before.size || latest.mtime != before.mtime ||
            latest.permissions != before.permissions || latest.uid != before.uid || latest.gid != before.gid {
            bail!("目标文件在确认后发生变化，已停止覆盖，请刷新目录后重新确认");
        }
        if live.cancel.is_cancelled() { bail!("会话已断开，未提交覆盖文件"); }
        task.message(app, "正在提交覆盖文件");
        let mut payload = Vec::new();
        for path in [&temporary.path, &target] { payload.extend_from_slice(&(path.len() as u32).to_be_bytes()); payload.extend_from_slice(path.as_bytes()); }
        match timeout(Duration::from_secs(60), raw.extended("posix-rename@openssh.com", payload)).await.context("文件覆盖结果未确认，请核对远程文件后再决定是否重试")?? {
            Packet::Status(status) if status.status_code == StatusCode::Ok => {},
            _ => bail!("远程文件覆盖未成功，请核对目标文件"),
        }
        temporary.armed = false;
        Ok(Outcome { name: name.into(), bytes: uploaded.bytes, local_path: source.into(), remote_path: target })
    }.await;
    let _ = raw.close_session();
    result
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
