use crate::{ssh::LiveSession, AppState};
use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use russh::ChannelMsg;
use russh_sftp::{client::{RawSftpSession, SftpSession}, protocol::{FileAttributes, OpenFlags, Packet, StatusCode}};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, fs, io::Write, path::PathBuf, sync::{Arc, Mutex, Weak}, time::{Duration, SystemTime, UNIX_EPOCH}};
use tauri::State;
use tokio::{io::{AsyncReadExt, AsyncWriteExt}, time::timeout};
use uuid::Uuid;

const MAX_BYTES: usize = 2 * 1024 * 1024;
const MAX_FILES: usize = 20;
const MAX_SNAPSHOTS: usize = 20;
const POSIX_RENAME: &str = "posix-rename@openssh.com";

pub struct EditorStore {
    files: Mutex<HashMap<String, Arc<EditorFile>>>,
    // Serialize commits to the same remote identity across sessions and aliases.
    locks: Mutex<HashMap<PathBuf, Weak<tokio::sync::Mutex<()>>>>,
}
impl Default for EditorStore {
    fn default() -> Self { Self { files: Mutex::new(HashMap::new()), locks: Mutex::new(HashMap::new()) } }
}
impl EditorStore {
    fn get(&self, id: &str) -> Result<Arc<EditorFile>> {
        self.files.lock().map_err(|_| anyhow!("编辑器状态不可用"))?.get(id).cloned().ok_or_else(|| anyhow!("文件标签已关闭，请重新打开"))
    }
    fn lock_for(&self, path: &PathBuf) -> Result<Arc<tokio::sync::Mutex<()>>> {
        let mut locks = self.locks.lock().map_err(|_| anyhow!("文件保存状态不可用"))?;
        locks.retain(|_, value| value.strong_count() > 0);
        if let Some(lock) = locks.get(path).and_then(Weak::upgrade) { return Ok(lock); }
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        locks.insert(path.clone(), Arc::downgrade(&lock));
        Ok(lock)
    }
    pub fn clear_renderer_files(&self) {
        if let Ok(mut files) = self.files.lock() { files.clear(); }
    }
}
struct EditorFile { live: Arc<LiveSession>, path: String, snapshots: PathBuf, writes: Arc<tokio::sync::Mutex<()>> }

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteText { content: String, revision: String, bytes: usize, bom: bool, line_ending: String }
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenedFile { id: String, path: String, document: RemoteText, warning: Option<String> }
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotInfo { id: String, created_at: u64, bytes: usize, reason: String }
#[derive(Serialize, Deserialize)]
struct DiskSnapshot { info: SnapshotInfo, data: String }
#[derive(Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum SaveResult {
    Saved { document: RemoteText, snapshot: SnapshotInfo },
    Conflict { current: RemoteText },
}

fn digest(bytes: &[u8]) -> String { format!("{:x}", Sha256::digest(bytes)) }
fn decode(bytes: &[u8]) -> Result<RemoteText> {
    if bytes.len() > MAX_BYTES { bail!("在线编辑仅支持不超过 2 MiB 的文本文件，请下载后编辑"); }
    let bom = bytes.starts_with(&[0xef, 0xbb, 0xbf]);
    let text = std::str::from_utf8(if bom { &bytes[3..] } else { bytes })
        .context("文件不是有效的 UTF-8 文本，请下载并使用支持该编码的编辑器打开")?;
    if text.chars().any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t')) { bail!("此文件包含二进制控制字符，不支持在线编辑"); }
    let content = text.replace("\r\n", "\n");
    if content.contains('\r') || (text.contains("\r\n") && text.replace("\r\n", "").contains('\n')) {
        bail!("文件包含旧式或混合换行，为避免改变原格式，请下载后编辑");
    }
    Ok(RemoteText { content, revision: digest(bytes), bytes: bytes.len(), bom, line_ending: if text.contains("\r\n") { "CRLF" } else { "LF" }.into() })
}
fn encode(content: &str, format: &RemoteText) -> Result<Vec<u8>> {
    if content.len() > MAX_BYTES || content.contains('\r') { bail!("内容超过 2 MiB 或包含不支持的换行，请缩小文件后重试"); }
    let text = if format.line_ending == "CRLF" { content.replace('\n', "\r\n") } else { content.to_owned() };
    let mut bytes = Vec::with_capacity(text.len() + 3);
    if format.bom { bytes.extend_from_slice(&[0xef, 0xbb, 0xbf]); }
    bytes.extend_from_slice(text.as_bytes());
    decode(&bytes)?;
    Ok(bytes)
}
async fn read_remote(file: &EditorFile) -> Result<(Vec<u8>, FileAttributes)> {
    let sftp = file.live.sftp().await?;
    // Keep a previously resolved target from being redirected through a new symlink.
    if sftp.canonicalize(file.path.clone()).await? != file.path { bail!("远程文件的实际路径已改变，请重新打开文件"); }
    let attributes = sftp.metadata(file.path.clone()).await?;
    if !attributes.is_regular() { bail!("仅支持编辑普通文本文件"); }
    if attributes.len() > MAX_BYTES as u64 { bail!("在线编辑仅支持不超过 2 MiB 的文本文件"); }
    let mut input = sftp.open(file.path.clone()).await?;
    let before = input.metadata().await?;
    if !before.is_regular() { bail!("仅支持编辑普通文本文件"); }
    if before.len() > MAX_BYTES as u64 { bail!("在线编辑仅支持不超过 2 MiB 的文本文件"); }
    let mut bytes = Vec::new();
    (&mut input).take(MAX_BYTES as u64 + 1).read_to_end(&mut bytes).await?;
    let after = input.metadata().await?;
    input.close().await?;
    if before.size != after.size || before.mtime != after.mtime { bail!("远程文件正在被修改，请稍后重新读取"); }
    decode(&bytes)?;
    Ok((bytes, after))
}
async fn bounded_read(file: &EditorFile) -> Result<(Vec<u8>, FileAttributes)> {
    tokio::select! {
        _ = file.live.cancel.cancelled() => bail!("会话已断开，编辑内容仍保留在当前标签中"),
        result = timeout(Duration::from_secs(90), read_remote(file)) => result.context("读取远程文件超时，请重试")?,
    }
}

fn list_local(directory: &PathBuf) -> Result<Vec<SnapshotInfo>> {
    if !directory.exists() { return Ok(Vec::new()); }
    let mut items = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if entry.path().extension().and_then(|s| s.to_str()) != Some("json") || entry.metadata()?.len() > (MAX_BYTES * 2) as u64 { continue; }
        let Ok(snapshot) = serde_json::from_slice::<DiskSnapshot>(&fs::read(entry.path())?) else { continue; };
        if Uuid::parse_str(&snapshot.info.id).is_ok() { items.push(snapshot.info); }
    }
    items.sort_by(|a, b| b.created_at.cmp(&a.created_at).then_with(|| b.id.cmp(&a.id)));
    Ok(items)
}
async fn snapshot(file: &EditorFile, bytes: Vec<u8>, reason: &str) -> Result<SnapshotInfo> {
    let directory = file.snapshots.clone();
    let reason = reason.to_owned();
    tokio::task::spawn_blocking(move || write_snapshot(&directory, &bytes, &reason)).await.context("本地快照任务未完成")?
}
fn write_snapshot(directory: &PathBuf, bytes: &[u8], reason: &str) -> Result<SnapshotInfo> {
        decode(bytes)?;
        fs::create_dir_all(&directory)?;
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
        }
        let info = SnapshotInfo { id: Uuid::new_v4().to_string(), created_at: SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64, bytes: bytes.len(), reason: reason.into() };
        let disk = DiskSnapshot { info: info.clone(), data: STANDARD.encode(bytes) };
        let mut temporary = tempfile::NamedTempFile::new_in(&directory)?;
        temporary.write_all(&serde_json::to_vec(&disk)?)?;
        temporary.as_file().sync_all()?;
        temporary.persist_noclobber(directory.join(format!("{}.json", info.id)))?;
        // Only prune files created by this feature, within this file's hash directory.
        // Failure to prune must not turn a successfully durable snapshot into a failed save.
        if let Ok(items) = list_local(&directory) {
            for old in items.into_iter().filter(|old| old.id != info.id).skip(MAX_SNAPSHOTS - 1) {
                let _ = fs::remove_file(directory.join(format!("{}.json", old.id)));
            }
        }
        Ok(info)
}

// A save uses its own short-lived SFTP control channel for the OpenSSH atomic
// replacement extension; terminal and shared transfer channels remain untouched.
async fn rename_session(file: &EditorFile) -> Result<RawSftpSession> {
    let connection = file.live.connection.get().ok_or_else(|| anyhow!("会话已断开"))?;
    let mut channel = connection.channel_open_session().await?;
    channel.request_subsystem(true, "sftp").await?;
    loop {
        match channel.wait().await {
            Some(ChannelMsg::Success) => break,
            Some(ChannelMsg::Failure | ChannelMsg::Close) | None => bail!("服务器未接受文件保存通道"),
            _ => {}
        }
    }
    let raw = RawSftpSession::new(channel.into_stream());
    let version = raw.init().await?;
    if version.extensions.get(POSIX_RENAME).map(String::as_str) != Some("1") {
        let _ = raw.close_session();
        bail!("服务器不支持安全原子替换（posix-rename），原文件未修改。请下载后编辑");
    }
    Ok(raw)
}
struct RemoteTemporary { sftp: Arc<SftpSession>, path: String, armed: bool }
impl Drop for RemoteTemporary {
    fn drop(&mut self) {
        if self.armed {
            let sftp = self.sftp.clone(); let path = self.path.clone();
            tauri::async_runtime::spawn(async move { let _ = sftp.remove_file(path).await; });
        }
    }
}
async fn commit(file: &EditorFile, content: &str, revision: &str, restoring: bool) -> Result<SaveResult> {
    let (original, attributes) = bounded_read(file).await?;
    let current = decode(&original)?;
    if current.revision != revision { return Ok(SaveResult::Conflict { current }); }
    let bytes = encode(content, &current)?;
    let raw = timeout(Duration::from_secs(25), rename_session(file)).await.context("建立文件保存通道超时")??;
    let result = async {
        let saved_snapshot = snapshot(file, original, if restoring { "restore" } else { "save" }).await
            .context("本地快照创建失败，远程文件尚未写入")?;
        let sftp = file.live.sftp().await?;
        let parent = file.path.rsplit_once('/').map(|(p, _)| p).unwrap_or("");
        let mut temporary = RemoteTemporary { sftp: sftp.clone(), path: format!("{parent}/.orbitshell-{}.tmp", Uuid::new_v4()), armed: false };
        let mut output = sftp.open_with_flags_and_attributes(temporary.path.clone(), OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::EXCLUDE,
            FileAttributes { permissions: Some(0o600), ..FileAttributes::empty() }).await?;
        temporary.armed = true;
        output.write_all(&bytes).await?;
        output.flush().await?;
        output.sync_all().await?;
        let owner = output.metadata().await?;
        if owner.uid != attributes.uid || owner.gid != attributes.gid {
            output.set_metadata(FileAttributes { uid: attributes.uid, gid: attributes.gid, ..FileAttributes::empty() }).await
                .context("无法保留原文件的属主或属组，已停止保存")?;
        }
        output.set_metadata(FileAttributes { permissions: attributes.permissions, ..FileAttributes::empty() }).await
            .context("无法保留原文件权限，已停止保存")?;
        output.close().await?;
        let (latest, latest_attributes) = bounded_read(file).await?;
        let latest_text = decode(&latest)?;
        if latest_text.revision != revision || latest_attributes.permissions != attributes.permissions || latest_attributes.uid != attributes.uid || latest_attributes.gid != attributes.gid {
            return Ok(SaveResult::Conflict { current: latest_text });
        }
        if file.live.cancel.is_cancelled() { bail!("会话已断开，文件尚未提交"); }
        let mut payload = Vec::new();
        for path in [&temporary.path, &file.path] {
            payload.extend_from_slice(&(path.len() as u32).to_be_bytes());
            payload.extend_from_slice(path.as_bytes());
        }
        match raw.extended(POSIX_RENAME, payload).await? {
            Packet::Status(status) if status.status_code == StatusCode::Ok => {},
            _ => bail!("远程文件替换未成功，原文件快照已保留，请重新读取后确认"),
        }
        temporary.armed = false;
        Ok(SaveResult::Saved { document: decode(&bytes)?, snapshot: saved_snapshot })
    }.await;
    let _ = raw.close_session();
    result
}

#[tauri::command]
pub async fn editor_open(session_id: String, path: String, state: State<'_, AppState>) -> Result<OpenedFile, String> {
    let result = async {
        if path.is_empty() || path.len() > 8192 || path.contains('\0') { bail!("文件路径无效"); }
        if state.editor.files.lock().map_err(|_| anyhow!("编辑器状态不可用"))?.len() >= MAX_FILES { bail!("最多同时打开 20 个远程文件，请先关闭部分标签"); }
        let live = state.session(&session_id).map_err(|e| anyhow!(e))?;
        let sftp = live.sftp().await?;
        let path = timeout(Duration::from_secs(30), sftp.canonicalize(path)).await.context("解析文件路径超时")??;
        let address = format!("[{}]:{}", live.profile.hostname, live.profile.port);
        let fingerprint = state.storage.store.lock().map_err(|_| anyhow!("配置暂不可用"))?.known_hosts.get(&address).cloned()
            .ok_or_else(|| anyhow!("服务器身份未确认，请重新建立连接"))?;
        let identity = serde_json::to_vec(&(live.profile.hostname.to_lowercase(), live.profile.port, &live.profile.username, fingerprint, &path))?;
        let root = state.storage.path.parent().ok_or_else(|| anyhow!("本地快照目录不可用"))?.join("file-snapshots").join(digest(&identity));
        let writes = state.editor.lock_for(&root)?;
        let file = Arc::new(EditorFile { live, path: path.clone(), snapshots: root, writes });
        let (bytes, _) = bounded_read(&file).await?;
        let document = decode(&bytes)?;
        let _guard = file.writes.lock().await;
        let directory = file.snapshots.clone();
        let initial = tokio::task::spawn_blocking(move || list_local(&directory)).await.context("读取初始快照状态未完成")?;
        let warning = match initial {
            Ok(items) if !items.is_empty() => None,
            Ok(_) if snapshot(&file, bytes, "open").await.is_ok() => None,
            _ => Some("初始快照未记录；保存时会再次尝试，快照失败将停止保存。".into()),
        };
        let id = Uuid::new_v4().to_string();
        let mut files = state.editor.files.lock().map_err(|_| anyhow!("编辑器状态不可用"))?;
        if files.len() >= MAX_FILES { bail!("最多同时打开 20 个远程文件"); }
        files.insert(id.clone(), file.clone());
        Ok::<_, anyhow::Error>(OpenedFile { id, path, document, warning })
    }.await;
    result.map_err(|e| format!("打开文件失败：{e:#}"))
}
#[tauri::command]
pub async fn editor_read(id: String, state: State<'_, AppState>) -> Result<RemoteText, String> {
    let file = state.editor.get(&id).map_err(|e| e.to_string())?;
    bounded_read(&file).await.and_then(|(bytes, _)| decode(&bytes)).map_err(|e| format!("读取文件失败：{e:#}"))
}
#[tauri::command]
pub async fn editor_save(id: String, content: String, revision: String, restoring: bool, state: State<'_, AppState>) -> Result<SaveResult, String> {
    let file = state.editor.get(&id).map_err(|e| e.to_string())?;
    let _guard = file.writes.lock().await;
    // A cancelled/expired write can have an uncertain acknowledgement. The next
    // attempt always rereads and compares the remote version rather than retrying.
    tokio::select! {
        _ = file.live.cancel.cancelled() => Err("会话已断开，保存未确认。编辑内容和已有快照仍保留，请重新读取远程版本后确认。".into()),
        result = timeout(Duration::from_secs(120), commit(&file, &content, &revision, restoring)) =>
            result.map_err(|_| "保存超时，远程结果未确认。请保留当前编辑并重新对比远程文件。".to_owned())?
                .map_err(|e| format!("保存未完成：{e:#}")),
    }
}
#[tauri::command]
pub async fn editor_snapshots(id: String, state: State<'_, AppState>) -> Result<Vec<SnapshotInfo>, String> {
    let file = state.editor.get(&id).map_err(|e| e.to_string())?;
    let directory = file.snapshots.clone();
    tokio::task::spawn_blocking(move || list_local(&directory)).await.map_err(|_| "快照读取任务未完成".to_owned())?
        .map_err(|e| format!("读取快照失败：{e:#}"))
}
#[tauri::command]
pub async fn editor_snapshot_read(id: String, snapshot_id: String, state: State<'_, AppState>) -> Result<RemoteText, String> {
    Uuid::parse_str(&snapshot_id).map_err(|_| "快照标识无效")?;
    let file = state.editor.get(&id).map_err(|e| e.to_string())?;
    let path = file.snapshots.join(format!("{snapshot_id}.json"));
    tokio::task::spawn_blocking(move || {
        if fs::metadata(&path)?.len() > (MAX_BYTES * 2) as u64 { bail!("快照文件无效"); }
        let disk: DiskSnapshot = serde_json::from_slice(&fs::read(path)?)?;
        let bytes = STANDARD.decode(disk.data)?;
        decode(&bytes)
    }).await.map_err(|_| "快照读取任务未完成".to_owned())?.map_err(|e| format!("读取快照失败：{e:#}"))
}
#[tauri::command]
pub fn editor_close(id: String, state: State<'_, AppState>) -> Result<(), String> {
    state.editor.files.lock().map_err(|_| "编辑器状态不可用")?.remove(&id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_bom_crlf_survives_edit_and_snapshot() {
        let original = b"\xef\xbb\xbf[server]\r\ntimeout=30\r\n";
        let text = decode(original).unwrap();
        assert!(text.bom);
        assert_eq!(text.line_ending, "CRLF");
        assert_eq!(text.content, "[server]\ntimeout=30\n");
        assert_eq!(encode(&text.content, &text).unwrap(), original);
        assert_eq!(encode("timeout=60\n", &text).unwrap(), b"\xef\xbb\xbftimeout=60\r\n");
        let dir = tempfile::tempdir().unwrap();
        let item = write_snapshot(&dir.path().to_path_buf(), original, "save").unwrap();
        let disk: DiskSnapshot = serde_json::from_slice(&fs::read(dir.path().join(format!("{}.json", item.id))).unwrap()).unwrap();
        assert_eq!(STANDARD.decode(disk.data).unwrap(), original);
    }
    #[test]
    fn reject_binary_non_utf8_mixed_and_oversize_before_saving() {
        for bytes in [b"binary\0data".as_slice(), b"\xff\xfe", b"a\r\nb\nc", b"a\rb"] { assert!(decode(bytes).is_err()); }
        assert!(decode(&vec![b'a'; MAX_BYTES + 1]).is_err());
        let format = decode(b"a\r\n").unwrap();
        assert!(encode(&"\n".repeat(MAX_BYTES), &format).is_err());
    }
    #[test]
    fn content_version_detects_same_length_changes() {
        assert_ne!(decode(b"value=1").unwrap().revision, decode(b"value=2").unwrap().revision);
        assert_ne!(decode(b"a\n").unwrap().revision, decode(b"a\r\n").unwrap().revision);
    }
    #[test]
    fn pruning_keeps_the_just_written_backup_even_with_tied_timestamps() {
        let dir = tempfile::tempdir().unwrap(); let path = dir.path().to_path_buf();
        for _ in 0..MAX_SNAPSHOTS {
            let info = SnapshotInfo {id: Uuid::new_v4().to_string(), created_at: u64::MAX, bytes: 3, reason: "save".into()};
            let disk = DiskSnapshot {info: info.clone(), data: STANDARD.encode(b"old")};
            fs::write(path.join(format!("{}.json", info.id)), serde_json::to_vec(&disk).unwrap()).unwrap();
        }
        let latest = write_snapshot(&path, b"current", "save").unwrap();
        assert_eq!(list_local(&path).unwrap().len(), MAX_SNAPSHOTS);
        assert!(path.join(format!("{}.json", latest.id)).exists());
    }
    #[test]
    fn snapshot_failure_does_not_report_success_or_leave_partial_backup() {
        let dir = tempfile::tempdir().unwrap(); let path = dir.path().join("not-a-directory");
        fs::write(&path, b"existing").unwrap();
        assert!(write_snapshot(&path, b"data", "save").is_err());
        assert_eq!(fs::read(&path).unwrap(), b"existing");
        let empty = dir.path().join("empty");
        assert!(write_snapshot(&empty, b"invalid\0", "save").is_err());
        assert!(!empty.exists());
    }
    #[test]
    fn same_target_lock_is_shared_and_unrelated_targets_are_independent() {
        let store = EditorStore::default();
        let a = store.lock_for(&PathBuf::from("same-target")).unwrap();
        let b = store.lock_for(&PathBuf::from("same-target")).unwrap();
        let c = store.lock_for(&PathBuf::from("other-target")).unwrap();
        assert!(Arc::ptr_eq(&a, &b)); assert!(!Arc::ptr_eq(&a, &c));
    }
}
