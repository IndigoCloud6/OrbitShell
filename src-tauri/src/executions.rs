use crate::{storage::ConnectionProfile, AppState};
use serde::Serialize;
use std::{sync::Mutex, time::{Instant, SystemTime, UNIX_EPOCH}};
use tauri::{AppHandle, Emitter, Manager, State};
use uuid::Uuid;

pub const OUTPUT_LIMIT: usize = 512 * 1024;
const RECORD_LIMIT: usize = 50;
const RUNNING_LIMIT: usize = 16;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionRecord {
    pub id: String,
    pub session_id: String,
    pub session_name: String,
    pub group: String,
    pub hostname: String,
    pub port: u16,
    pub username: String,
    pub tool: String,
    pub command: Option<String>,
    pub status: String,
    pub started_at: u64,
    pub elapsed_ms: u64,
    pub exit_code: Option<u32>,
    pub message: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputChunk {
    pub stream: String,
    pub data: Vec<u8>,
}

struct StoredChunk { offset: usize, stream: &'static str, data: Vec<u8> }
struct StoredExecution {
    record: ExecutionRecord,
    started: Instant,
    chunks: Vec<StoredChunk>,
    bytes: usize,
}

#[derive(Default)]
struct Records { revision: u64, entries: Vec<StoredExecution> }

#[derive(Default)]
pub struct ExecutionStore { records: Mutex<Records> }

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionSnapshot { revision: u64, records: Vec<ExecutionRecord> }

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionOutput { chunks: Vec<OutputChunk>, cursor: usize, has_more: bool }

impl ExecutionStore {
    fn changed(app: &AppHandle, revision: u64, output_id: Option<&str>) {
        // Notify only; terminal/exec byte streams never enter React render state.
        let _ = app.emit_to("main", "orbitshell:execution-changed", serde_json::json!({"revision": revision, "outputId": output_id}));
    }

    fn begin(&self, app: &AppHandle, session_id: String, profile: &ConnectionProfile, tool: &str, command: Option<String>) -> Result<String, String> {
        let mut records = self.records.lock().map_err(|_| "执行记录暂不可用")?;
        if records.entries.iter().filter(|entry| entry.record.status == "running").count() >= RUNNING_LIMIT {
            return Err("同时执行的 AI 任务过多，请等待现有任务完成".into());
        }
        while records.entries.len() >= RECORD_LIMIT {
            let Some(index) = records.entries.iter().position(|entry| entry.record.status != "running") else { break; };
            records.entries.remove(index);
        }
        let id = Uuid::new_v4().to_string();
        records.entries.push(StoredExecution {
            record: ExecutionRecord {
                id: id.clone(), session_id, session_name: profile.name.clone(), group: profile.group.clone(),
                hostname: profile.hostname.clone(), port: profile.port, username: profile.username.clone(),
                tool: tool.into(), command, status: "running".into(),
                started_at: SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64,
                elapsed_ms: 0, exit_code: None, message: String::new(),
            },
            started: Instant::now(), chunks: Vec::new(), bytes: 0,
        });
        records.revision += 1;
        Self::changed(app, records.revision, None);
        Ok(id)
    }

    fn append(&self, app: &AppHandle, id: &str, stream: &'static str, data: &[u8]) -> usize {
        let Ok(mut records) = self.records.lock() else { return 0; };
        let Some(entry) = records.entries.iter_mut().find(|entry| entry.record.id == id) else { return 0; };
        let count = data.len().min(OUTPUT_LIMIT.saturating_sub(entry.bytes));
        if count == 0 { return 0; }
        if entry.chunks.len() >= 8192 && entry.chunks.last().is_some_and(|chunk| chunk.stream != stream) { return 0; }
        if let Some(last) = entry.chunks.last_mut().filter(|chunk| chunk.stream == stream && chunk.data.len() + count <= 16 * 1024) {
            last.data.extend_from_slice(&data[..count]);
        } else {
            entry.chunks.push(StoredChunk { offset: entry.bytes, stream, data: data[..count].to_vec() });
        }
        entry.bytes += count;
        records.revision += 1;
        Self::changed(app, records.revision, Some(id));
        count
    }

    fn finish(&self, app: &AppHandle, id: &str, status: &str, exit_code: Option<u32>, message: &str) {
        let Ok(mut records) = self.records.lock() else { return; };
        let Some(entry) = records.entries.iter_mut().find(|entry| entry.record.id == id) else { return; };
        if entry.record.status != "running" { return; }
        entry.record.status = status.into();
        entry.record.exit_code = exit_code;
        entry.record.elapsed_ms = entry.started.elapsed().as_millis() as u64;
        entry.record.message = message.into();
        records.revision += 1;
        Self::changed(app, records.revision, None);
    }

    fn snapshot(&self) -> Result<ExecutionSnapshot, String> {
        let records = self.records.lock().map_err(|_| "执行记录暂不可用")?;
        Ok(ExecutionSnapshot { revision: records.revision, records: records.entries.iter().rev().map(|entry| {
            let mut record = entry.record.clone();
            if record.status == "running" { record.elapsed_ms = entry.started.elapsed().as_millis() as u64; }
            record
        }).collect() })
    }

    fn output(&self, id: &str, cursor: usize) -> Result<ExecutionOutput, String> {
        let records = self.records.lock().map_err(|_| "执行记录暂不可用")?;
        let entry = records.entries.iter().find(|entry| entry.record.id == id).ok_or("该执行记录已被清理")?;
        let start = cursor.min(entry.bytes);
        let end = (start + 64 * 1024).min(entry.bytes);
        let chunks = entry.chunks.iter().filter_map(|chunk| {
            let from = start.saturating_sub(chunk.offset).min(chunk.data.len());
            let to = end.saturating_sub(chunk.offset).min(chunk.data.len());
            (from < to).then(|| OutputChunk { stream: chunk.stream.into(), data: chunk.data[from..to].to_vec() })
        }).collect();
        Ok(ExecutionOutput { chunks, cursor: end, has_more: end < entry.bytes })
    }
}

pub struct ExecutionTracker { app: AppHandle, id: String, finished: bool }
impl ExecutionTracker {
    pub fn begin(app: AppHandle, session_id: String, profile: &ConnectionProfile, tool: &str, command: Option<String>) -> Result<Self, String> {
        let id = app.state::<AppState>().executions.begin(&app, session_id, profile, tool, command)?;
        Ok(Self { app, id, finished: false })
    }
    pub fn append(&self, stream: &'static str, data: &[u8]) -> usize {
        self.app.state::<AppState>().executions.append(&self.app, &self.id, stream, data)
    }
    pub fn finish(&mut self, status: &str, exit_code: Option<u32>, message: &str) {
        self.app.state::<AppState>().executions.finish(&self.app, &self.id, status, exit_code, message);
        self.finished = true;
    }
}
impl Drop for ExecutionTracker {
    fn drop(&mut self) {
        if !self.finished { self.finish("cancelled", None, "调用已中断，远端进程状态以服务器为准"); }
    }
}

#[tauri::command]
pub fn execution_list(state: State<'_, AppState>) -> Result<ExecutionSnapshot, String> { state.executions.snapshot() }

#[tauri::command]
pub fn execution_output(id: String, cursor: usize, state: State<'_, AppState>) -> Result<ExecutionOutput, String> {
    state.executions.output(&id, cursor)
}

#[tauri::command]
pub fn execution_clear(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let mut records = state.executions.records.lock().map_err(|_| "执行记录暂不可用")?;
    records.entries.retain(|entry| entry.record.status == "running");
    records.revision += 1;
    ExecutionStore::changed(&app, records.revision, None);
    Ok(())
}
