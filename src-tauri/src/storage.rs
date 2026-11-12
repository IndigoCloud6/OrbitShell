use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, io::Write, path::PathBuf, sync::Mutex};
use uuid::Uuid;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConnectionProfile {
    pub id: String,
    pub name: String,
    pub group: String,
    pub hostname: String,
    pub port: u16,
    pub username: String,
    pub auth_method: String,
    pub private_key_path: Option<String>,
}

impl ConnectionProfile {
    pub fn validate(&mut self) -> Result<(), String> {
        self.name = self.name.trim().to_owned();
        self.group = self.group.trim().to_owned();
        self.hostname = self.hostname.trim().trim_matches(['[', ']']).to_owned();
        self.username = self.username.trim().to_owned();
        if self.id.is_empty() { self.id = Uuid::new_v4().to_string(); }
        Uuid::parse_str(&self.id).map_err(|_| "连接标识无效")?;
        if self.name.is_empty() || self.name.len() > 160 { return Err("请填写有效的连接名称".into()); }
        if self.group.is_empty() { self.group = "未分组".into(); }
        if self.group.len() > 120 { return Err("分组名称过长".into()); }
        if self.hostname.is_empty() || self.hostname.len() > 253 || self.hostname.chars().any(char::is_whitespace) {
            return Err("请填写有效的服务器地址".into());
        }
        if self.port == 0 || self.username.is_empty() || self.username.len() > 4096 {
            return Err("请填写有效的端口和用户名".into());
        }
        if !matches!(self.auth_method.as_str(), "password" | "privateKey") {
            return Err("不支持的认证方式".into());
        }
        if self.auth_method == "privateKey" && self.private_key_path.as_deref().unwrap_or("").trim().is_empty() {
            return Err("请选择私钥文件".into());
        }
        if self.auth_method == "password" { self.private_key_path = None; }
        Ok(())
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Store {
    pub connections: Vec<ConnectionProfile>,
    pub known_hosts: HashMap<String, String>,
    #[serde(default)]
    pub mcp_token: String,
    #[serde(default)]
    pub quick_commands: Vec<crate::quick_commands::QuickCommand>,
    #[serde(default)]
    pub quick_command_groups: Vec<String>,
    #[serde(default)]
    pub desktop: crate::desktop::DesktopPreferences,
}

pub struct Storage {
    pub path: PathBuf,
    pub store: Mutex<Store>,
}

impl Storage {
    pub fn open(directory: PathBuf) -> anyhow::Result<Self> {
        fs::create_dir_all(&directory)?;
        let path = directory.join("workspace.json");
        let store = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Store::default(),
            Err(error) => return Err(error.into()),
        };
        Ok(Self { path, store: Mutex::new(store) })
    }

    // Replace the file only after the complete new document has reached disk.
    pub fn persist(&self, store: &Store) -> anyhow::Result<()> {
        let directory = self.path.parent().ok_or_else(|| anyhow::anyhow!("存储目录无效"))?;
        let mut file = tempfile::NamedTempFile::new_in(directory)?;
        file.write_all(&serde_json::to_vec_pretty(store)?)?;
        file.as_file().sync_all()?;
        file.persist(&self.path)?;
        Ok(())
    }
}
