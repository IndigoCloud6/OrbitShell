use crate::{credentials, ssh::LiveSession, storage::ConnectionProfile, AppState};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Mutex, time::{SystemTime, UNIX_EPOCH}};
use tauri::{AppHandle, Emitter, Manager, State};

const LIMIT: usize = 50;

// Only a saved profile reference and its successful connection time reach disk.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentStamp { pub profile_id: String, pub last_used_at: u64 }

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentConnection {
    id: String,
    profile_id: Option<String>,
    session_id: Option<String>,
    name: String,
    group: String,
    hostname: String,
    port: u16,
    username: String,
    last_used_at: u64,
    temporary: bool,
    requires_new_link: bool,
}

#[derive(Default)]
struct RuntimeHistory { entries: Vec<RecentConnection>, warning: Option<String> }
#[derive(Default)]
pub struct RecentState { runtime: Mutex<RuntimeHistory> }

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentSnapshot { records: Vec<RecentConnection>, warning: Option<String> }

fn saved_entry(profile: &ConnectionProfile, time: u64, session_id: Option<String>) -> RecentConnection {
    RecentConnection { id: format!("profile:{}", profile.id), profile_id: Some(profile.id.clone()), session_id,
        name: profile.name.clone(), group: profile.group.clone(), hostname: profile.hostname.clone(), port: profile.port,
        username: if profile.username.starts_with("b64>>") { String::new() } else { profile.username.clone() },
        last_used_at: time, temporary: false, requires_new_link: false }
}

pub fn notify(app: &AppHandle) { let _ = app.emit_to("main", "orbitshell:recent-connections-changed", ()); }

pub fn forget_profile(state: &AppState, profile_id: &str) {
    if let Ok(mut runtime) = state.recent.runtime.lock() {
        runtime.entries.retain(|entry| entry.profile_id.as_deref() != Some(profile_id));
    }
}

pub fn record_connected(app: &AppHandle, session: &LiveSession, session_id: &str) {
    let state = app.state::<AppState>();
    let time = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis().min(u64::MAX as u128) as u64;
    let profile = &session.profile;
    let result = (|| -> Result<(), String> {
        if session.is_temporary {
            // Authentication links are never retained, even in the history's in-memory entries.
            let requires_new_link = profile.group != "快速连接" || profile.username.starts_with("b64>>");
            let entry = RecentConnection { id: format!("temporary:{session_id}"), profile_id: None, session_id: Some(session_id.into()),
                name: if requires_new_link { if profile.username.starts_with("b64>>") { "堡垒机临时会话".into() } else { "临时会话".into() } } else { profile.name.clone() },
                group: "临时连接".into(), hostname: profile.hostname.clone(), port: profile.port,
                username: if requires_new_link { String::new() } else { profile.username.clone() }, last_used_at: time, temporary: true, requires_new_link };
            let mut runtime = state.recent.runtime.lock().map_err(|_| "最近会话状态不可用")?;
            // Keep only the most recent quick connection for each ordinary target.
            if !requires_new_link {
                runtime.entries.retain(|existing| !existing.temporary || existing.requires_new_link ||
                    existing.hostname != entry.hostname || existing.port != entry.port || existing.username != entry.username);
            }
            runtime.entries.insert(0, entry);
            runtime.entries.truncate(LIMIT);
            return Ok(());
        }

        let mut store = state.storage.store.lock().map_err(|_| "最近会话未能保存到本机")?;
        let Some(current) = store.connections.iter().find(|current| current.id == profile.id && credentials::same_identity(current, profile)) else { return Ok(()); };
        let entry = saved_entry(current, time, Some(session_id.into()));
        let mut next = store.clone();
        next.recent_connections.retain(|stamp| stamp.profile_id != profile.id);
        next.recent_connections.push(RecentStamp { profile_id: profile.id.clone(), last_used_at: time });
        next.recent_connections.sort_by(|a, b| b.last_used_at.cmp(&a.last_used_at));
        next.recent_connections.truncate(LIMIT);
        let warning = if state.storage.persist(&next).is_ok() { *store = next; None } else {
            Some("最近会话未能保存到本机，本次运行仍可查看。".into())
        };
        let mut runtime = state.recent.runtime.lock().map_err(|_| "最近会话状态不可用")?;
        runtime.entries.retain(|previous| previous.id != entry.id);
        runtime.entries.insert(0, entry);
        runtime.entries.truncate(LIMIT);
        runtime.warning = warning;
        Ok(())
    })();
    if let Err(message) = result {
        if let Ok(mut runtime) = state.recent.runtime.lock() { runtime.warning = Some(message); }
    }
    // A history write failure must never turn an authenticated SSH session into a failure.
    notify(app);
}

#[tauri::command]
pub fn list_recent_connections(state: State<'_, AppState>) -> Result<RecentSnapshot, String> {
    let store = state.storage.store.lock().map_err(|_| "最近会话暂不可用")?;
    let runtime = state.recent.runtime.lock().map_err(|_| "最近会话暂不可用")?;
    let mut entries = HashMap::new();
    for stamp in &store.recent_connections {
        if let Some(profile) = store.connections.iter().find(|profile| profile.id == stamp.profile_id) {
            let entry = saved_entry(profile, stamp.last_used_at, None);
            entries.insert(entry.id.clone(), entry);
        }
    }
    for entry in &runtime.entries {
        let current = if let Some(profile_id) = &entry.profile_id {
            let Some(profile) = store.connections.iter().find(|profile| &profile.id == profile_id) else { continue; };
            saved_entry(profile, entry.last_used_at, entry.session_id.clone())
        } else { entry.clone() };
        if entries.get(&current.id).is_none_or(|previous: &RecentConnection| previous.last_used_at <= current.last_used_at) {
            entries.insert(current.id.clone(), current);
        }
    }
    let mut records: Vec<_> = entries.into_values().collect();
    records.sort_by(|a, b| b.last_used_at.cmp(&a.last_used_at).then_with(|| a.id.cmp(&b.id)));
    records.truncate(LIMIT);
    Ok(RecentSnapshot { records, warning: runtime.warning.clone() })
}
