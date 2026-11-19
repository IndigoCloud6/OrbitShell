use std::{collections::{HashSet, VecDeque}, path::Path};

use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: String,
    pub host_key: String,
    pub hostname: String,
    pub port: u16,
    pub username: String,
    pub session_name: String,
    pub source: String,
    pub agent_name: Option<String>,
    pub command: String,
    pub created_at: u64,
    pub redacted: bool,
    pub temporary: bool,
}

#[derive(Default, Serialize, Deserialize)]
struct Saved { entries: VecDeque<Entry> }

pub fn read(path: &Path) -> anyhow::Result<VecDeque<Entry>> {
    let saved: Saved = match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Saved::default(),
        Err(error) => return Err(error.into()),
    };
    Ok(saved.entries.into_iter().filter(|entry| !entry.temporary).collect())
}

pub fn merge(path: &Path, entries: impl IntoIterator<Item = Entry>) -> anyhow::Result<()> {
    let _lock = crate::lock_workspace(path)?;
    let mut saved = read(path)?;
    let mut ids: HashSet<_> = saved.iter().map(|entry| entry.id.clone()).collect();
    saved.extend(entries.into_iter().filter(|entry| !entry.temporary && ids.insert(entry.id.clone())));
    saved.make_contiguous().sort_by_key(|entry| entry.created_at);
    let mut bytes: usize = saved.iter().map(|entry| entry.command.len()).sum();
    while saved.len() > 10000 || bytes > 8 * 1024 * 1024 {
        if let Some(old) = saved.pop_front() { bytes -= old.command.len(); }
    }
    crate::write_workspace(path, &Saved { entries: saved })
}
