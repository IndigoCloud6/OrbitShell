use crate::AppState;
pub use orbitshell_ssh::remote_editor::EditorStore;
use orbitshell_ssh::remote_editor::{OpenedFile, RemoteText, SaveResult, SnapshotInfo};
use tauri::State;

#[tauri::command]
pub async fn editor_open(
    session_id: String,
    path: String,
    state: State<'_, AppState>,
) -> Result<OpenedFile, String> {
    let live = state.session(&session_id)?;
    let address = format!("[{}]:{}", live.profile.hostname, live.profile.port);
    let fingerprint = state
        .storage
        .lock()
        .map_err(|_| "配置暂不可用")?
        .known_hosts
        .get(&address)
        .cloned()
        .ok_or("服务器身份未确认，请重新建立连接")?;
    let root = state
        .storage
        .path
        .parent()
        .ok_or("本地快照目录不可用")?
        .join("file-snapshots");
    state.editor.open(live, path, &fingerprint, &root).await
}

#[tauri::command]
pub async fn editor_read(id: String, state: State<'_, AppState>) -> Result<RemoteText, String> {
    state.editor.read(&id).await
}

#[tauri::command]
pub async fn editor_save(
    id: String,
    content: String,
    revision: String,
    restoring: bool,
    state: State<'_, AppState>,
) -> Result<SaveResult, String> {
    state.editor.save(&id, &content, &revision, restoring).await
}

#[tauri::command]
pub async fn editor_snapshots(
    id: String,
    state: State<'_, AppState>,
) -> Result<Vec<SnapshotInfo>, String> {
    state.editor.snapshots(&id).await
}

#[tauri::command]
pub async fn editor_snapshot_read(
    id: String,
    snapshot_id: String,
    state: State<'_, AppState>,
) -> Result<RemoteText, String> {
    state.editor.read_snapshot(&id, &snapshot_id).await
}

#[tauri::command]
pub fn editor_close(id: String, state: State<'_, AppState>) -> Result<(), String> {
    state.editor.close(&id)
}
