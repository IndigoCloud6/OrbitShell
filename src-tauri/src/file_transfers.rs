use crate::{sftp_transfer::{self, Outcome, TransferProgress}, ssh::LiveSession, AppState};
use serde::{Deserialize, Serialize};
use std::{fs, io::Write, path::{Path, PathBuf}, sync::{Arc, Mutex, atomic::Ordering}, time::{Instant, SystemTime, UNIX_EPOCH}};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const HISTORY_LIMIT: usize = 1000;
const HISTORY_BYTES: u64 = 8 * 1024 * 1024;
fn now() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64 }
fn notify(app: &AppHandle) { let _ = app.emit_to("main", "orbitshell:file-transfers-changed", ()); }

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction { Upload, Download }

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transfer {
    pub id: String, pub agent_id: Option<String>, pub agent_name: Option<String>,
    pub session_id: String, pub session_name: String, pub hostname: String, pub port: u16,
    pub temporary: bool, pub name: String, pub direction: Direction, pub status: String, pub message: String,
    pub bytes: u64, pub total: Option<u64>, pub speed: f64,
    pub local_path: String, pub remote_path: String, pub created_at: u64,
    pub finished_at: Option<u64>, pub elapsed_ms: u64,
    #[serde(skip_deserializing)]
    pub restored: bool,
}
impl Transfer { fn active(&self) -> bool { self.status == "running" } }

#[derive(Serialize, Deserialize)]
struct History { version: u32, records: Vec<Transfer> }
#[derive(Serialize)]
pub struct Snapshot { records: Vec<Transfer>, warning: Option<String> }

pub struct Task { info: Mutex<Transfer>, cancel: CancellationToken, started: Instant, sample: Mutex<(Instant, u64)> }
impl Task {
    pub fn cancel(&self) { self.cancel.cancel(); }
    pub fn cancelled(&self) -> bool { self.cancel.is_cancelled() }
    pub fn message(&self, app: &AppHandle, message: &str) { self.patch(app, |info| info.message = message.into()); }
    fn new(info: Transfer) -> Self { Self { info: Mutex::new(info), cancel: CancellationToken::new(), started: Instant::now(), sample: Mutex::new((Instant::now(), 0)) } }
    fn snapshot(&self) -> Result<Transfer, String> { self.info.lock().map(|info| info.clone()).map_err(|_| "文件传输状态不可用".into()) }
    fn patch(&self, app: &AppHandle, update: impl FnOnce(&mut Transfer)) {
        if let Ok(mut info) = self.info.lock() { update(&mut info); }
        notify(app);
    }
    pub fn progress(&self, app: &AppHandle, progress: TransferProgress) {
        let speed = if let Ok(mut sample) = self.sample.lock() {
            let speed = progress.bytes.saturating_sub(sample.1) as f64 / sample.0.elapsed().as_secs_f64().max(0.05);
            *sample = (Instant::now(), progress.bytes); speed
        } else { 0.0 };
        self.patch(app, |info| { if !info.active() { return; } info.name = progress.name; info.bytes = progress.bytes; info.total = progress.total; info.speed = speed; info.message = "传输中".into(); });
    }
}

pub struct Transfers {
    entries: Mutex<Vec<Arc<Task>>>, path: PathBuf, disk: Mutex<()>,
    warning: Mutex<Option<String>>, unreadable: bool,
}
impl Transfers {
    pub fn open(root: PathBuf) -> Self {
        let path = root.join("transfer-history.json");
        let loaded = (|| -> anyhow::Result<Vec<Transfer>> {
            match fs::metadata(&path) {
                Ok(metadata) if metadata.len() > HISTORY_BYTES => anyhow::bail!("history too large"),
                Ok(_) => {},
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
                Err(error) => return Err(error.into()),
            }
            let history: History = serde_json::from_slice(&fs::read(&path)?)?;
            if history.version != 1 { anyhow::bail!("unsupported history version"); }
            Ok(history.records)
        })();
        let unreadable = loaded.is_err();
        let mut records = loaded.unwrap_or_default();
        records.retain(|info| !info.temporary);
        if records.len() > HISTORY_LIMIT { records.drain(..records.len() - HISTORY_LIMIT); }
        for info in &mut records {
            info.restored = true;
            if info.active() {
                info.status = "error".into(); info.finished_at = None; info.elapsed_ms = 0;
                info.message = "上次运行中断，结果未确认；请核对目标文件后再决定是否重试".into();
            }
            info.speed = 0.0;
        }
        Self { entries: Mutex::new(records.into_iter().map(|info| Arc::new(Task::new(info))).collect()), path, disk: Mutex::new(()),
            warning: Mutex::new(unreadable.then(|| "本机传输记录无法读取，已保留原文件；本次记录暂存于内存".into())), unreadable }
    }
    fn records(&self) -> Result<Vec<Transfer>, String> {
        self.entries.lock().map_err(|_| "文件传输状态不可用")?.iter().map(|task| task.snapshot()).collect()
    }
    pub fn snapshot(&self) -> Result<Snapshot, String> {
        Ok(Snapshot { records: self.records()?, warning: self.warning.lock().map_err(|_| "传输记录状态不可用")?.clone() })
    }
    pub fn get(&self, agent: &str, id: &str) -> Result<Transfer, String> {
        self.records()?.into_iter().find(|info| info.id == id && info.agent_id.as_deref() == Some(agent))
            .ok_or_else(|| "未找到当前 Agent 的传输任务；记录可能已经清理或属于另一 MCP 客户端".into())
    }
    pub fn stop_mcp(&self) {
        if let Ok(entries) = self.entries.lock() {
            for task in entries.iter() {
                if task.snapshot().is_ok_and(|info| info.agent_id.is_some() && info.active()) { task.cancel.cancel(); }
            }
        }
    }
    fn save(&self, app: &AppHandle) {
        if self.unreadable { notify(app); return; }
        let saved = (|| -> anyhow::Result<()> {
            // Serialize writes; take the latest state after acquiring the disk lock.
            let _disk = self.disk.lock().map_err(|_| anyhow::anyhow!("history unavailable"))?;
            let mut records = self.records().map_err(anyhow::Error::msg)?;
            records.retain(|info| !info.temporary);
            for info in &mut records { info.speed = 0.0; }
            let data = serde_json::to_vec(&History { version: 1, records })?;
            if data.len() as u64 > HISTORY_BYTES { anyhow::bail!("history too large"); }
            let parent = self.path.parent().ok_or_else(|| anyhow::anyhow!("history directory unavailable"))?;
            fs::create_dir_all(parent)?;
            let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
            temporary.write_all(&data)?; temporary.as_file_mut().sync_all()?;
            temporary.persist(&self.path).map_err(|error| error.error)?;
            Ok(())
        })();
        if let Ok(mut warning) = self.warning.lock() {
            *warning = saved.err().map(|_| "传输记录保存失败，已保留本次内存记录；原记录文件未被主动清空".into());
        }
        notify(app);
    }
    fn register(&self, app: &AppHandle, id: String, live: &LiveSession, session_id: &str, direction: Direction,
        agent: Option<(String, String)>, name: String, local_path: String, remote_path: String) -> Result<Arc<Task>, String> {
        let task = Arc::new(Task::new(Transfer { id, agent_id: agent.as_ref().map(|agent| agent.0.clone()), agent_name: agent.map(|agent| agent.1),
            session_id: session_id.into(), session_name: live.profile.name.clone(), hostname: live.profile.hostname.clone(), port: live.profile.port,
            temporary: live.is_temporary, name, direction, status: "running".into(), message: "准备传输".into(), bytes: 0, total: None, speed: 0.0,
            local_path, remote_path, created_at: now(), finished_at: None, elapsed_ms: 0, restored: false }));
        {
            let mut entries = self.entries.lock().map_err(|_| "文件传输状态不可用")?;
            let info = task.snapshot()?;
            if entries.iter().any(|entry| entry.snapshot().is_ok_and(|old| old.id == info.id)) { return Err("传输任务标识重复，请勿重发同一任务".into()); }
            if info.agent_id.is_some() && entries.iter().filter(|entry| entry.snapshot().is_ok_and(|old| old.agent_id.is_some() && old.active())).count() >= 4 {
                return Err("同时进行的 MCP 文件传输已达到 4 个，请等待现有任务结束".into());
            }
            if entries.iter().filter(|entry| entry.snapshot().map_or(true, |old| old.active())).count() >= 32 { return Err("正在进行的文件传输过多，请稍后重试".into()); }
            while entries.len() >= HISTORY_LIMIT {
                let removable = entries.iter().position(|entry| entry.snapshot().is_ok_and(|info| !info.active()));
                if let Some(index) = removable { entries.remove(index); } else { break; }
            }
            entries.push(task.clone());
        }
        self.save(app); Ok(task)
    }
    pub fn manual(&self, app: &AppHandle, id: String, live: &LiveSession, session_id: &str, direction: Direction,
        name: String, local_path: String, remote_path: String) -> Result<Arc<Task>, String> {
        Uuid::parse_str(&id).map_err(|_| "传输任务标识无效")?;
        self.register(app, id, live, session_id, direction, None, name, local_path, remote_path)
    }
    pub fn finish(&self, app: &AppHandle, task: &Task, live: &LiveSession, result: &anyhow::Result<Outcome>) {
        task.patch(app, |info| {
            info.speed = 0.0; info.finished_at = Some(now()); info.elapsed_ms = task.started.elapsed().as_millis() as u64;
            match result {
                Ok(outcome) => {
                    info.name = outcome.name.clone(); info.bytes = outcome.bytes; info.total = Some(outcome.bytes);
                    info.local_path = outcome.local_path.to_string_lossy().into_owned(); info.remote_path = outcome.remote_path.clone();
                    info.status = "done".into(); info.message = "已完成".into();
                },
                Err(_) if task.cancel.is_cancelled() || live.cancel.is_cancelled() => {
                    info.status = "cancelled".into();
                    info.message = if live.cancel.is_cancelled() { "所属 SSH 会话已断开，传输已停止" } else if info.agent_id.is_some() { "MCP 已关闭，传输已停止" } else { "用户已取消传输" }.into();
                },
                Err(error) => { info.status = "error".into(); info.message = format!("传输失败：{error:#}"); },
            }
        });
        self.save(app);
    }
    pub fn finish_drag(&self, app: &AppHandle, task: &Task, result: &Result<Option<u64>, String>) {
        task.patch(app, |info| {
            info.speed = 0.0; info.finished_at = Some(now()); info.elapsed_ms = task.started.elapsed().as_millis() as u64;
            match result {
                Ok(Some(bytes)) => { info.status = "done".into(); info.bytes = *bytes; info.total = Some(*bytes); info.message = "文件已交给系统拖放目标保存".into(); },
                Ok(None) => { info.status = "cancelled".into(); info.message = "已取消拖出下载".into(); },
                Err(error) => { info.status = "error".into(); info.message = error.clone(); },
            }
        });
        self.save(app);
    }
    pub fn start(&self, app: AppHandle, agent_id: &str, session_id: &str, direction: Direction,
        local_path: String, remote_path: String) -> Result<Transfer, String> {
        validate_remote_path(&remote_path, matches!(direction, Direction::Download))?;
        let local = validate_local_path(&local_path)?;
        let state = app.state::<AppState>();
        if !state.mcp_enabled.load(Ordering::Acquire) || state.desktop.exiting.load(Ordering::Acquire) { return Err("MCP 已关闭或程序正在退出，未发起传输".into()); }
        let epoch = state.mcp_epoch.load(Ordering::Acquire);
        let live = state.session(session_id)?;
        if !live.connected.load(Ordering::Acquire) || live.cancel.is_cancelled() { return Err("SSH 尚未连接，请等待认证完成".into()); }
        let agent_name = state.ai.agent_name(agent_id)?;
        let name = match direction {
            Direction::Upload => local.file_name().and_then(|name| name.to_str()).ok_or("本机文件名无效")?.to_owned(),
            Direction::Download => remote_path.rsplit('/').next().ok_or("远程文件路径无效")?.to_owned(),
        };
        let intended_remote = match direction {
            Direction::Upload => format!("{}/{}", remote_path.trim_end_matches('/'), name),
            Direction::Download => remote_path.clone(),
        };
        let task = self.register(&app, Uuid::new_v4().to_string(), &live, session_id, direction,
            Some((agent_id.into(), agent_name)), name.clone(), local_path, intended_remote)?;
        let initial = task.snapshot()?;
        drop(state);
        tauri::async_runtime::spawn(async move {
            let state = app.state::<AppState>();
            if !state.mcp_enabled.load(Ordering::Acquire) || state.mcp_epoch.load(Ordering::Acquire) != epoch { task.cancel.cancel(); }
            let result = async {
                if task.cancel.is_cancelled() || live.cancel.is_cancelled() { anyhow::bail!("传输已停止"); }
                let local = prepare_local_path(&local, direction).await?;
                let progress = |event| task.progress(&app, event);
                match direction {
                    Direction::Upload => sftp_transfer::upload(&live, &local, &remote_path, &name, None, progress, &task.cancel).await,
                    Direction::Download => sftp_transfer::download(&live, &remote_path, &local, false, progress, &task.cancel).await,
                }
            }.await;
            app.state::<AppState>().transfers.finish(&app, &task, &live, &result);
        });
        Ok(initial)
    }
}

fn validate_remote_path(path: &str, file: bool) -> Result<(), String> {
    if !path.starts_with('/') || path.len() > 4096 || path.chars().any(char::is_control) {
        return Err("请提供 SFTP 中的绝对路径，不支持相对路径、控制字符或超过 4096 字节的路径".into());
    }
    if file && path.rsplit('/').next().is_none_or(|name| name.is_empty() || matches!(name, "." | "..")) { return Err("下载路径必须指向具体文件".into()); }
    Ok(())
}

fn validate_local_path(value: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(value);
    if !path.is_absolute() || value.len() > 32767 || value.chars().any(char::is_control) || path.file_name().is_none() {
        return Err("请提供 OrbitShell 所在电脑上的完整文件路径，不支持相对路径或控制字符".into());
    }
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};
        if !matches!(path.components().next(), Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))) {
            return Err("本机路径只支持本地磁盘文件，不支持设备或网络共享路径".into());
        }
        if path.components().any(|part| matches!(part, Component::Normal(name) if name.to_string_lossy().contains(':'))) { return Err("不支持 Windows 备用数据流路径".into()); }
    }
    Ok(path)
}

async fn prepare_local_path(path: &Path, direction: Direction) -> anyhow::Result<PathBuf> {
    let prepared = match direction {
        Direction::Upload => tokio::fs::canonicalize(path).await?,
        Direction::Download => {
            match tokio::fs::symlink_metadata(path).await {
                Ok(_) => anyhow::bail!("本机已存在同名目标，请选择新的文件名；未覆盖原文件"),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
                Err(error) => return Err(error.into()),
            }
            let parent = tokio::fs::canonicalize(path.parent().ok_or_else(|| anyhow::anyhow!("本机保存目录无效"))?).await?;
            if !tokio::fs::metadata(&parent).await?.is_dir() { anyhow::bail!("本机保存位置不是目录"); }
            parent.join(path.file_name().ok_or_else(|| anyhow::anyhow!("本机保存文件名无效"))?)
        }
    };
    // Recheck resolved paths so junctions cannot redirect local-only access to network/device paths.
    validate_local_path(prepared.to_str().ok_or_else(|| anyhow::anyhow!("本机文件路径编码无效"))?).map_err(anyhow::Error::msg)?;
    Ok(prepared)
}

#[tauri::command]
pub fn file_transfer_list(state: State<'_, AppState>) -> Result<Snapshot, String> { state.transfers.snapshot() }
