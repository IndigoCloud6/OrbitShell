pub use orbitshell_ssh::ConnectionProfile;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
};
use zeroize::Zeroizing;

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Store {
    pub connections: Vec<ConnectionProfile>,
    pub known_hosts: HashMap<String, String>,
    #[serde(default)]
    pub mcp_token: String,
    #[serde(default)]
    pub mcp_auto_start: bool,
    #[serde(default)]
    pub quick_commands: Vec<crate::quick_commands::QuickCommand>,
    #[serde(default)]
    pub quick_command_groups: Vec<String>,
    #[serde(default)]
    pub desktop: crate::desktop::DesktopPreferences,
    #[serde(default)]
    pub recent_connections: Vec<crate::recent::RecentStamp>,
    #[serde(default)]
    pub gpui_workspace_migrated: bool,
    #[serde(default)]
    pub gpui_linux_quick_commands_seeded: bool,
    #[serde(default)]
    pub gpui_linux_quick_commands_version: u64,
}

pub struct Storage {
    pub path: PathBuf,
    store: Mutex<Store>,
}

pub struct LockedStore<'a> {
    store: MutexGuard<'a, Store>,
    _file_lock: fs::File,
}

impl Deref for LockedStore<'_> {
    type Target = Store;

    fn deref(&self) -> &Store {
        &self.store
    }
}

impl DerefMut for LockedStore<'_> {
    fn deref_mut(&mut self) -> &mut Store {
        &mut self.store
    }
}

impl Storage {
    pub fn open(directory: PathBuf) -> anyhow::Result<Self> {
        fs::create_dir_all(&directory)?;
        let path = directory.join("workspace.json");
        let store = read_store(&path)?;
        Ok(Self {
            path,
            store: Mutex::new(store),
        })
    }

    pub fn lock(&self) -> anyhow::Result<LockedStore<'_>> {
        let mut store = self
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("配置暂不可用"))?;
        let file_lock = orbitshell_ssh::lock_workspace(&self.path)?;
        // GPUI can update connections while this process is open.
        *store = read_store(&self.path)?;
        Ok(LockedStore {
            store,
            _file_lock: file_lock,
        })
    }

    // Replace the file only after the complete new document has reached disk.
    pub fn persist(&self, store: &Store) -> anyhow::Result<()> {
        orbitshell_ssh::write_workspace(&self.path, store)
    }
}

fn read_store(path: &Path) -> anyhow::Result<Store> {
    match fs::read(path) {
        Ok(bytes) => Ok(serde_json::from_slice(&Zeroizing::new(bytes))?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Store::default()),
        Err(error) => Err(error.into()),
    }
}
