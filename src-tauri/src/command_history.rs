use crate::{ssh::LiveSession, AppState};
use serde::{Deserialize, Serialize};
use std::{collections::VecDeque, fs, io::Write, path::PathBuf, sync::{Mutex, atomic::{AtomicBool, AtomicU64, Ordering}}, time::{SystemTime, UNIX_EPOCH}};
use tauri::{AppHandle, Emitter, Manager, State};
use uuid::Uuid;

const LIMIT: usize = 10000;
const BYTE_LIMIT: usize = 8 * 1024 * 1024;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: String, pub host_key: String, pub hostname: String, pub port: u16, pub username: String,
    pub session_name: String, pub source: String, pub agent_name: Option<String>, pub command: String,
    pub created_at: u64, pub redacted: bool, pub temporary: bool,
}
#[derive(Clone)]
struct Context { host_key: String, hostname: String, port: u16, username: String, name: String, temporary: bool }
#[derive(Default, Serialize, Deserialize)]
struct Saved { entries: VecDeque<Entry> }
pub struct History {
    path: PathBuf, entries: Mutex<VecDeque<Entry>>, contexts: Mutex<VecDeque<(String, Context)>>,
    file_lock: Mutex<()>, revision: AtomicU64, saving: AtomicBool, error: Mutex<String>, load_failed: bool,
}

impl History {
    pub fn open(directory: PathBuf) -> Self {
        let path = directory.join("command-history.json");
        let result = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice::<Saved>(&bytes).map_err(|_| "执行历史文件无法读取，原文件已保留，本次新增记录暂存于内存。".to_string()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Saved::default()),
            Err(_) => Err("执行历史文件读取失败，本次新增记录暂存于内存。".into()),
        };
        let (entries, error, load_failed) = match result { Ok(saved) => (saved.entries.into_iter().filter(|entry| !entry.temporary).collect(), String::new(), false), Err(error) => (VecDeque::new(), error, true) };
        Self { path, entries: Mutex::new(entries), contexts: Mutex::new(VecDeque::new()), file_lock: Mutex::new(()), revision: AtomicU64::new(0), saving: AtomicBool::new(false), error: Mutex::new(error), load_failed }
    }
    pub fn register(&self, session_id: &str, live: &LiveSession) {
        let profile = &live.profile;
        let temporary = live.is_temporary;
        let context = Context { host_key: if temporary { format!("temporary:{}", profile.id) } else { format!("{}:{}", profile.hostname.to_lowercase(), profile.port) },
            hostname: profile.hostname.clone(), port: profile.port, username: if profile.username.starts_with("b64>>") { "堡垒机会话".into() } else { profile.username.clone() }, name: profile.name.clone(), temporary };
        if let Ok(mut contexts) = self.contexts.lock() {
            contexts.retain(|(id, _)| id != session_id); contexts.push_back((session_id.into(), context));
            while contexts.len() > 1000 { contexts.pop_front(); }
        }
    }
    pub fn record(&self, app: &AppHandle, session_id: &str, id: String, command: &str, agent_name: Option<&str>) {
        let command = command.trim();
        if command.is_empty() || command.len() > 64 * 1024 || command.contains('\0') { return; }
        let context = self.contexts.lock().ok().and_then(|c| c.iter().find(|(id, _)| id == session_id).map(|(_, c)| c.clone()));
        let Some(context) = context else { return; };
        let redacted = sensitive(command);
        let command = if redacted { "[包含敏感参数，命令内容未保存]".into() } else { command.chars().filter(|c| !c.is_control() || matches!(*c, '\n' | '\t')).collect() };
        let entry = Entry { id, host_key: context.host_key, hostname: context.hostname, port: context.port, username: context.username,
            session_name: context.name, source: if agent_name.is_some() { "ai" } else { "human" }.into(), agent_name: agent_name.map(|name| name.chars().filter(|c| !c.is_control()).take(100).collect()),
            command, created_at: SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64, redacted, temporary: context.temporary };
        if let Ok(mut entries) = self.entries.lock() {
            if entries.iter().any(|e| e.id == entry.id) { return; }
            entries.push_back(entry);
            let mut bytes = entries.iter().map(|e| e.command.len()).sum::<usize>();
            while entries.len() > LIMIT || bytes > BYTE_LIMIT { if let Some(old) = entries.pop_front() { bytes = bytes.saturating_sub(old.command.len()); } else { break; } }
        }
        self.revision.fetch_add(1, Ordering::Release);
        let _ = app.emit_to("main", "orbitshell:command-history-changed", ());
        if !self.load_failed && !self.saving.swap(true, Ordering::AcqRel) {
            let app = app.clone();
            tauri::async_runtime::spawn_blocking(move || loop {
                let state = app.state::<AppState>();
                let before = state.history.revision.load(Ordering::Acquire);
                let result = state.history.flush();
                if let Ok(mut error) = state.history.error.lock() { *error = result.err().unwrap_or_default(); }
                let _ = app.emit_to("main", "orbitshell:command-history-changed", ());
                state.history.saving.store(false, Ordering::Release);
                if state.history.revision.load(Ordering::Acquire) == before || state.history.saving.swap(true, Ordering::AcqRel) { break; }
            });
        }
    }
    pub fn flush(&self) -> Result<(), String> {
        if self.load_failed { return Err("执行历史原文件不可读，未覆盖原文件".into()); }
        let _file = self.file_lock.lock().map_err(|_| "执行历史暂不可写")?;
        let saved = Saved { entries: self.entries.lock().map_err(|_| "执行历史不可用")?.iter().filter(|e| !e.temporary).cloned().collect() };
        let directory = self.path.parent().ok_or("执行历史目录无效")?;
        let result = (|| -> std::io::Result<()> {
            let mut file = tempfile::NamedTempFile::new_in(directory)?;
            serde_json::to_writer(&mut file, &saved).map_err(std::io::Error::other)?; file.flush()?; file.as_file().sync_all()?;
            file.persist(&self.path).map_err(|e| e.error)?; Ok(())
        })();
        result.map_err(|_| "执行历史保存失败，新增记录仍在内存中，可稍后重试。".into())
    }
}

fn sensitive(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    let compact: String = lower.chars().filter(|c| !c.is_whitespace() && !matches!(c, '\'' | '"')).collect();
    ["ssh://", "b64>>", "sshpass ", "--password", "--passwd", "--token", "--secret", "password=", "passwd=", "token=", "secret=", "api_key=", "apikey=", "authorization:", "bearer ", "private key", "x-api-key", "api-key:", "cookie:"].iter().any(|p| lower.contains(p))
        || ["password", "passwd", "token", "secret", "api_key", "apikey", "access_key", "accesskey", "private_key"].iter()
            .any(|key| ["=", ":"].iter().any(|separator| compact.contains(&format!("{key}{separator}"))))
        || lower.split_whitespace().any(|word| word.split_once("://").is_some_and(|(_, rest)| rest.split('/').next().is_some_and(|authority| authority.contains('@'))))
        || ((lower.contains("mysql ") || lower.contains("redis-cli ")) && lower.contains(" -p"))
        || (lower.contains("curl ") && (lower.contains(" -u") || lower.contains(" --user")))
        || (lower.contains("docker ") && lower.contains("login") && lower.contains(" -p"))
        || lower.contains("-asplaintext")
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Host { key: String, hostname: String, port: u16, name: String, count: usize, temporary: bool }
#[derive(Serialize)]
pub struct Snapshot { entries: Vec<Entry>, hosts: Vec<Host>, total: usize, count: usize, warning: String }
#[tauri::command]
pub fn command_history(host_key: Option<String>, query: Option<String>, source: Option<String>, offset: Option<usize>, state: State<'_, AppState>) -> Result<Snapshot, String> {
    let entries = state.history.entries.lock().map_err(|_| "执行历史不可用")?;
    let mut hosts = std::collections::BTreeMap::<String, Host>::new();
    for entry in entries.iter().rev() {
        let host = hosts.entry(entry.host_key.clone()).or_insert_with(|| Host { key: entry.host_key.clone(), hostname: entry.hostname.clone(), port: entry.port, name: entry.session_name.clone(), count: 0, temporary: entry.temporary });
        host.count += 1;
    }
    let query = query.unwrap_or_default().trim().to_lowercase();
    let selected: Vec<_> = entries.iter().rev().filter(|entry| {
        host_key.as_ref().is_none_or(|key| key.is_empty() || *key == entry.host_key)
            && source.as_ref().is_none_or(|source| source.is_empty() || *source == entry.source)
            && (query.is_empty() || [&entry.command, &entry.hostname, &entry.username, &entry.session_name, entry.agent_name.as_ref().unwrap_or(&entry.source)].iter().any(|value| value.to_lowercase().contains(&query)))
    }).collect();
    Ok(Snapshot { total: selected.len(), count: entries.len(), entries: selected.into_iter().skip(offset.unwrap_or(0)).take(100).cloned().collect(), hosts: hosts.into_values().collect(), warning: state.history.error.lock().map_err(|_| "执行历史状态不可用")?.clone() })
}
#[tauri::command]
pub fn command_history_record(session_id: String, id: String, command: String, app: AppHandle) -> Result<(), String> {
    Uuid::parse_str(&id).map_err(|_| "命令记录标识无效")?;
    app.state::<AppState>().history.record(&app, &session_id, id, &command, None); Ok(())
}
#[tauri::command]
pub fn command_history_retry(app: AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let result = state.history.flush();
    if let Ok(mut error) = state.history.error.lock() { *error = result.clone().err().unwrap_or_default(); }
    let _ = app.emit_to("main", "orbitshell:command-history-changed", ()); result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_common_cli_environment_and_json_credentials() {
        for command in [
            "AWS_SECRET_ACCESS_KEY=example program", "export API_KEY = 'example'", "curl --data '{\"password\":\"example\"}' https://example.test",
            "docker login registry.example.test -pexample", "curl -uuser:example https://example.test", "redis-cli -p example",
            "ConvertTo-SecureString 'example' -AsPlainText -Force", "curl -H 'Authorization: Bearer example' https://example.test",
        ] { assert!(sensitive(command), "credential not redacted: {command}"); }
        for command in ["ls -la", "git status", "printf 'hello'", "cat /srv/demo/app.conf"] { assert!(!sensitive(command)); }
    }

    fn entry(temporary: bool) -> Entry {
        Entry { id: Uuid::new_v4().to_string(), host_key: "192.0.2.20:22".into(), hostname: "192.0.2.20".into(), port: 22,
            username: "demo".into(), session_name: "Demo".into(), source: "human".into(), agent_name: None,
            command: "ls".into(), created_at: 1, redacted: false, temporary }
    }

    #[test]
    fn temporary_history_never_reaches_disk_and_latest_snapshot_wins() {
        let directory = tempfile::tempdir().unwrap();
        let history = History::open(directory.path().to_owned());
        history.entries.lock().unwrap().extend([entry(false), entry(true)]);
        history.flush().unwrap();
        let reopened = History::open(directory.path().to_owned());
        assert_eq!(reopened.entries.lock().unwrap().len(), 1);
        history.entries.lock().unwrap().push_back(entry(false));
        history.flush().unwrap();
        assert_eq!(History::open(directory.path().to_owned()).entries.lock().unwrap().len(), 2);
    }

    #[test]
    fn unreadable_history_is_preserved() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("command-history.json");
        fs::write(&path, b"invalid original file").unwrap();
        let history = History::open(directory.path().to_owned());
        history.entries.lock().unwrap().push_back(entry(false));
        assert!(history.flush().is_err());
        assert_eq!(fs::read(path).unwrap(), b"invalid original file");
    }
}
