use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, Weak, atomic::Ordering},
};

use anyhow::{Context, Result, anyhow, bail};
use orbitshell_ssh::{
    ConnectionProfile, credentials, lock_workspace,
    channel_diagnostics::{ChannelDiagnostics, ChannelSnapshot},
    quick_command::{QuickCommand, group_name},
    remote_editor::{EditorStore, OpenedFile},
    server_metrics,
    session::{
        self, ConnectRequest, LiveSession, SessionCommand, SessionEvent, SessionEvents,
        SessionHooks,
    },
    sftp::{self, RemoteDirectory},
    write_workspace,
};
use serde::{Deserialize, Serialize};
use tokio::{
    runtime::Runtime,
    sync::{mpsc, watch},
    task::JoinHandle,
};
use zeroize::Zeroizing;

use crate::{mcp_terminal::TerminalControl, theme::UiTheme};

#[path = "linux_quick_commands.rs"]
mod linux_quick_commands;

const INPUT_CHUNK: usize = 16 * 1024;

#[derive(Default, Deserialize)]
#[serde(default)]
struct WorkspaceData {
    connections: Vec<ConnectionProfile>,
    known_hosts: HashMap<String, String>,
}

#[derive(Default, Deserialize, Serialize)]
pub(crate) struct SavedLayout {
    pub connections: Option<f32>,
    pub files: Option<f32>,
    pub ui_theme: Option<UiTheme>,
}

struct Workspace {
    path: PathBuf,
    previous_gpui: PathBuf,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SshDiagnosticReport {
    session_id: String,
    saved_at: String,
    #[serde(flatten)]
    snapshot: ChannelSnapshot,
}

fn read_workspace(path: &Path) -> Result<WorkspaceData> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&Zeroizing::new(bytes))
            .map_err(|_| anyhow!("连接配置或主机指纹文件格式无效")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(WorkspaceData::default()),
        Err(error) => Err(error).context("无法读取连接配置或主机指纹文件"),
    }
}

pub(crate) const UNGROUPED_COMMANDS: &str = "未分组";

#[derive(Default)]
pub(crate) struct QuickCommands {
    pub commands: Vec<QuickCommand>,
    pub groups: Vec<String>,
}

pub(crate) enum ConnectionGroupChange {
    Move(HashSet<String>),
    Rename(String),
}

fn read_quick_commands(document: &serde_json::Value) -> Result<QuickCommands, String> {
    let commands: Vec<QuickCommand> = match document.get("quick_commands") {
        Some(items) => serde_json::from_value(items.clone()).map_err(|_| "快捷指令配置格式无效")?,
        None => Vec::new(),
    };
    let saved_groups: Vec<String> = match document.get("quick_command_groups") {
        Some(items) => {
            serde_json::from_value(items.clone()).map_err(|_| "指令分组配置格式无效")?
        }
        None => Vec::new(),
    };
    let mut groups = Vec::new();
    for name in saved_groups
        .iter()
        .chain(commands.iter().map(|command| &command.group))
    {
        if name != UNGROUPED_COMMANDS && !groups.contains(name) {
            groups.push(name.clone());
        }
    }
    Ok(QuickCommands { commands, groups })
}

fn trusted_fingerprint(
    current: &WorkspaceData,
    previous: &WorkspaceData,
    address: &str,
) -> Result<Option<String>> {
    let current = current.known_hosts.get(address);
    let previous = previous.known_hosts.get(address);
    if let (Some(left), Some(right)) = (current, previous)
        && left != right
    {
        bail!("已保存的主机指纹存在冲突，请先核实服务器身份");
    }
    Ok(current.or(previous).cloned())
}

impl Workspace {
    fn load_document(&self) -> Result<serde_json::Value> {
        let document: serde_json::Value = match fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&Zeroizing::new(bytes))
                .map_err(|_| anyhow!("连接配置文件格式无效"))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                serde_json::json!({"connections": [], "known_hosts": {}})
            }
            Err(error) => return Err(error).context("无法读取连接配置"),
        };
        if !document
            .get("connections")
            .is_some_and(|value| value.is_array())
            || !document
                .get("known_hosts")
                .is_some_and(|value| value.is_object())
        {
            bail!("连接配置文件格式无效");
        }
        Ok(document)
    }

    fn migrate_gpui_workspace(&self) -> Result<()> {
        if !self
            .previous_gpui
            .try_exists()
            .context("无法访问先前的 GPUI 配置")?
        {
            return Ok(());
        }
        let _lock = lock_workspace(&self.path).context("无法锁定连接配置")?;
        let mut document = self.load_document()?;
        if document["gpui_workspace_migrated"].as_bool() == Some(true) {
            return Ok(());
        }
        let previous: WorkspaceData = read_workspace(&self.previous_gpui)?;
        let connections = document["connections"]
            .as_array_mut()
            .expect("workspace connections");
        for profile in previous.connections {
            if !connections
                .iter()
                .any(|entry| entry["id"].as_str() == Some(profile.id.as_str()))
            {
                connections.push(serde_json::to_value(profile)?);
            }
        }
        let known_hosts = document["known_hosts"]
            .as_object_mut()
            .expect("workspace known hosts");
        for (address, fingerprint) in previous.known_hosts {
            known_hosts
                .entry(address)
                .or_insert_with(|| fingerprint.into());
        }
        document["gpui_workspace_migrated"] = true.into();
        write_workspace(&self.path, &document).context("无法合并 GPUI 连接配置")
    }

    fn with_profile<T>(
        &self,
        profile: &ConnectionProfile,
        action: impl FnOnce(&ConnectionProfile) -> T,
    ) -> Result<T> {
        let _lock = lock_workspace(&self.path).context("无法锁定连接配置")?;
        let data = read_workspace(&self.path)?;
        let current = data
            .connections
            .iter()
            .find(|current| current.id == profile.id)
            .ok_or_else(|| anyhow!("连接配置已被删除，请重新打开 GPUI"))?;
        if !credentials::same_identity(current, profile) {
            bail!("连接配置已被其他窗口修改，请重新打开 GPUI");
        }
        Ok(action(current))
    }
}

impl SessionHooks for Workspace {
    fn known_host(&self, address: &str) -> Result<Option<String>> {
        trusted_fingerprint(
            &read_workspace(&self.path)?,
            &read_workspace(&self.previous_gpui)?,
            address,
        )
    }

    fn save_host(&self, address: String, fingerprint: String) -> Result<()> {
        let _lock = lock_workspace(&self.path).context("无法锁定主机指纹")?;
        if self
            .known_host(&address)?
            .is_some_and(|saved| saved != fingerprint)
        {
            bail!("另一会话保存了不同的服务器指纹，请重新核实");
        }
        let mut document = self.load_document()?;
        document["known_hosts"][address] = fingerprint.into();
        write_workspace(&self.path, &document).context("主机指纹未保存")?;
        Ok(())
    }

    fn remember_credential(&self, profile: &ConnectionProfile, secret: &str) -> Result<(), String> {
        self.with_profile(profile, |current| credentials::save(current, secret))
            .map_err(|error| format!("{error:#}"))?
    }
}

struct SessionScope {
    workspace: Arc<Workspace>,
    control: Arc<TerminalControl>,
}

impl SessionHooks for SessionScope {
    fn known_host(&self, address: &str) -> Result<Option<String>> {
        self.workspace.known_host(address)
    }

    fn save_host(&self, address: String, fingerprint: String) -> Result<()> {
        self.workspace.save_host(address, fingerprint)
    }

    fn remember_credential(&self, profile: &ConnectionProfile, secret: &str) -> Result<(), String> {
        self.workspace.remember_credential(profile, secret)
    }

    fn allow_input(&self, _: &str) -> bool {
        self.control.binding().is_none()
    }

    fn owns_input(&self, owner: &str, _: &str) -> bool {
        self.control
            .binding()
            .is_some_and(|binding| binding.id == owner)
    }
}

struct SessionRecord {
    live: Weak<LiveSession>,
    control: Arc<TerminalControl>,
}

pub struct Backend {
    runtime: Runtime,
    workspace: Arc<Workspace>,
    sessions: Arc<Mutex<HashMap<String, SessionRecord>>>,
    control_changed: watch::Sender<()>,
    editor: Arc<EditorStore>,
}

#[derive(Clone)]
pub struct McpCatalog {
    workspace: Arc<Workspace>,
    sessions: Arc<Mutex<HashMap<String, SessionRecord>>>,
    control_changed: watch::Sender<()>,
}

impl McpCatalog {
    pub(crate) fn diagnostics_path(&self) -> PathBuf {
        self.workspace
            .path
            .with_file_name("diagnostics")
            .join("mcp-diagnostics.jsonl")
    }

    pub fn profiles(&self) -> Result<Vec<ConnectionProfile>> {
        Ok(read_workspace(&self.workspace.path)?.connections)
    }

    pub fn sessions(&self, agent: &str) -> Result<Vec<serde_json::Value>> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| anyhow!("会话列表暂不可用"))?;
        sessions.retain(|_, record| {
            record
                .live
                .upgrade()
                .is_some_and(|live| !live.cancel.is_cancelled())
        });
        Ok(sessions
            .iter()
            .filter_map(|(id, record)| {
                let live = record.live.upgrade()?;
                let binding = record.control.binding();
                let temporary = live.is_temporary;
                Some(serde_json::json!({
                    "sessionId": id,
                    "connectionId": (!temporary).then_some(live.profile.id.as_str()),
                    "name": if temporary { "堡垒机临时会话" } else { &live.profile.name },
                    "hostname": (!temporary).then_some(live.profile.hostname.as_str()),
                    "port": (!temporary).then_some(live.profile.port),
                    "temporary": temporary,
                    "connected": live.connected.load(Ordering::Acquire),
                    "control": {"ownerAgentId": binding.as_ref().map(|owner| &owner.agent),
                        "terminalId": binding.as_ref().filter(|owner| owner.agent == agent).map(|owner| &owner.id)},
                }))
            })
            .collect())
    }

    pub fn session(&self, id: &str) -> Result<(Arc<LiveSession>, Arc<TerminalControl>)> {
        let sessions = self
            .sessions
            .lock()
            .map_err(|_| anyhow!("会话列表暂不可用"))?;
        let record = sessions.get(id).ok_or_else(|| anyhow!("未找到 SSH 会话"))?;
        let live = record
            .live
            .upgrade()
            .filter(|live| !live.cancel.is_cancelled())
            .ok_or_else(|| anyhow!("SSH 会话已断开"))?;
        Ok((live, record.control.clone()))
    }

    pub fn control_changed(&self) {
        self.control_changed.send_replace(());
    }

    pub fn token(&self) -> Result<Zeroizing<String>> {
        let path = &self.workspace.path;
        let _lock = lock_workspace(path).context("无法锁定 MCP 配置")?;
        let mut document = self.workspace.load_document()?;
        if let Some(token) = document
            .get("mcp_token")
            .and_then(|value| value.as_str())
            .filter(|token| !token.is_empty())
        {
            return Ok(Zeroizing::new(token.to_owned()));
        }
        let token = Zeroizing::new(format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        ));
        document["mcp_token"] = token.as_str().into();
        write_workspace(path, &document).context("无法保存 MCP 访问令牌")?;
        Ok(token)
    }
}

pub struct TerminalConnection {
    pub id: String,
    pub live: Arc<LiveSession>,
    pub control: Arc<TerminalControl>,
    pub input: mpsc::Sender<Zeroizing<Vec<u8>>>,
    pub size: watch::Sender<(u32, u32)>,
}

impl Drop for TerminalConnection {
    fn drop(&mut self) {
        self.live.cancel.cancel();
    }
}

impl Backend {
    fn layout_path(&self) -> PathBuf {
        self.workspace.path.with_file_name("gpui-layout.json")
    }

    pub(crate) fn layout(&self) -> Result<SavedLayout> {
        match fs::read(self.layout_path()) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes).unwrap_or_default()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(SavedLayout::default())
            }
            Err(error) => Err(error).context("无法读取外观与布局设置"),
        }
    }

    pub(crate) fn save_layout(&self, layout: SavedLayout) -> Result<()> {
        let path = self.layout_path();
        let _lock = lock_workspace(&path).context("无法锁定布局配置")?;
        let existing = self.layout()?;
        let updated = SavedLayout {
            connections: layout.connections.or(existing.connections),
            files: layout.files.or(existing.files),
            ui_theme: layout.ui_theme.or(existing.ui_theme),
        };
        write_workspace(&path, &updated).context("无法保存外观与布局设置")
    }

    pub fn close_to_tray(&self) -> Result<bool> {
        Ok(self.workspace.load_document()?["desktop"]["closeToTray"]
            .as_bool()
            .unwrap_or(true))
    }

    pub fn set_close_to_tray(&self, enabled: bool) -> Result<()> {
        let path = &self.workspace.path;
        let _lock = lock_workspace(path).context("无法锁定工作区配置")?;
        let mut document = self.workspace.load_document()?;
        let preferences = document
            .as_object_mut()
            .context("工作区配置格式无效")?
            .entry("desktop")
            .or_insert_with(|| serde_json::json!({}))
            .as_object_mut()
            .context("窗口行为配置格式无效")?;
        preferences.insert("closeToTray".into(), enabled.into());
        write_workspace(path, &document).context("无法保存窗口关闭行为")
    }

    pub fn mcp_auto_start(&self) -> Result<bool> {
        Ok(self.workspace.load_document()?["mcp_auto_start"]
            .as_bool()
            .unwrap_or(false))
    }

    pub fn set_mcp_auto_start(&self, enabled: bool) -> Result<()> {
        let path = &self.workspace.path;
        let _lock = lock_workspace(path).context("无法锁定工作区配置")?;
        let mut document = self.workspace.load_document()?;
        document["mcp_auto_start"] = enabled.into();
        write_workspace(path, &document).context("无法保存 MCP 自动开启设置")
    }

    pub fn history_path(&self) -> PathBuf {
        self.workspace.path.with_file_name("command-history.json")
    }

    pub fn editor(&self) -> Arc<EditorStore> {
        self.editor.clone()
    }

    pub fn open_remote_text(
        &self,
        live: Arc<LiveSession>,
        path: String,
    ) -> JoinHandle<Result<OpenedFile, String>> {
        let editor = self.editor.clone();
        let workspace = self.workspace.clone();
        self.runtime.spawn(async move {
            let address = format!("[{}]:{}", live.profile.hostname, live.profile.port);
            let fingerprint = workspace
                .known_host(&address)
                .map_err(|error| format!("{error:#}"))?
                .ok_or("服务器身份未确认，请重新建立连接")?;
            let base = workspace.path.with_file_name("file-snapshots");
            editor.open(live, path, &fingerprint, &base).await
        })
    }

    pub fn runtime_handle(&self) -> tokio::runtime::Handle {
        self.runtime.handle().clone()
    }

    pub fn mcp_catalog(&self) -> McpCatalog {
        McpCatalog {
            workspace: self.workspace.clone(),
            sessions: self.sessions.clone(),
            control_changed: self.control_changed.clone(),
        }
    }

    pub fn control_updates(&self) -> watch::Receiver<()> {
        self.control_changed.subscribe()
    }

    pub fn open() -> Result<Self> {
        let data = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .ok_or_else(|| anyhow!("无法定位 Windows 应用数据目录"))?;
        Self::open_in(data)
    }

    pub(crate) fn open_in(data: PathBuf) -> Result<Self> {
        let directory = data.join("com.indigocloud.orbitshell");
        fs::create_dir_all(&directory).context("无法创建工作区配置目录")?;
        let workspace = Arc::new(Workspace {
            path: directory.join("workspace.json"),
            previous_gpui: data
                .join("com.indigocloud.orbitshell.gpui")
                .join("workspace.json"),
        });
        workspace.migrate_gpui_workspace()?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("orbitshell-ssh")
            .build()
            .context("无法启动 SSH 运行时")?;
        Ok(Self {
            runtime,
            workspace,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            control_changed: watch::channel(()).0,
            editor: Arc::new(EditorStore::default()),
        })
    }

    pub fn profiles(&self) -> Result<Vec<ConnectionProfile>> {
        Ok(read_workspace(&self.workspace.path)?.connections)
    }

    pub(crate) fn quick_commands(&self) -> Result<QuickCommands, String> {
        let document = self
            .workspace
            .load_document()
            .map_err(|_| "无法读取工作区配置")?;
        if linux_quick_commands::installed_version(&document) < linux_quick_commands::VERSION {
            return self.update_quick_commands(|_| Ok(()));
        }
        read_quick_commands(&document)
    }

    pub(crate) fn save_quick_command(
        &self,
        command: QuickCommand,
        editing: bool,
    ) -> Result<QuickCommands, String> {
        let command = command.validate()?;
        self.update_quick_commands(move |collection| {
            if command.group != UNGROUPED_COMMANDS && !collection.groups.contains(&command.group) {
                if collection.groups.len() >= 100 {
                    return Err("最多保存 100 个指令分组".into());
                }
                collection.groups.push(command.group.clone());
            }
            if editing {
                let existing = collection
                    .commands
                    .iter_mut()
                    .find(|entry| entry.id == command.id)
                    .ok_or("指令已被删除")?;
                *existing = command;
            } else {
                if collection.commands.len() >= 500 {
                    return Err("最多保存 500 个快捷指令".into());
                }
                collection.commands.push(command);
            }
            Ok(())
        })
    }

    pub(crate) fn delete_quick_command(&self, id: &str) -> Result<QuickCommands, String> {
        self.update_quick_commands(|collection| {
            collection.commands.retain(|entry| entry.id != id);
            Ok(())
        })
    }

    pub(crate) fn save_quick_command_group(
        &self,
        name: &str,
        previous: Option<&str>,
    ) -> Result<QuickCommands, String> {
        let name = group_name(name)?;
        if name == UNGROUPED_COMMANDS {
            return Err("未分组是保留名称，请使用其他名称".into());
        }
        self.update_quick_commands(move |collection| {
            if collection.groups.contains(&name) && previous != Some(name.as_str()) {
                return Err("该分组已存在".into());
            }
            if let Some(previous) = previous {
                let group = collection
                    .groups
                    .iter_mut()
                    .find(|group| group.as_str() == previous)
                    .ok_or("分组已被删除")?;
                *group = name.clone();
                for command in &mut collection.commands {
                    if command.group == previous {
                        command.group = name.clone();
                    }
                }
            } else {
                if collection.groups.len() >= 100 {
                    return Err("最多保存 100 个指令分组".into());
                }
                collection.groups.push(name);
            }
            Ok(())
        })
    }

    pub(crate) fn delete_quick_command_group(&self, name: &str) -> Result<QuickCommands, String> {
        if name == UNGROUPED_COMMANDS {
            return Err("未分组不能删除".into());
        }
        self.update_quick_commands(|collection| {
            collection.groups.retain(|group| group != name);
            for command in &mut collection.commands {
                if command.group == name {
                    command.group = UNGROUPED_COMMANDS.into();
                }
            }
            Ok(())
        })
    }

    fn update_quick_commands(
        &self,
        update: impl FnOnce(&mut QuickCommands) -> Result<(), String>,
    ) -> Result<QuickCommands, String> {
        let path = &self.workspace.path;
        let _lock = lock_workspace(path).map_err(|_| "无法锁定工作区配置，请重试")?;
        let mut document = self
            .workspace
            .load_document()
            .map_err(|_| "无法读取工作区配置")?;
        let mut collection = read_quick_commands(&document)?;
        linux_quick_commands::seed_pending(&mut collection, &mut document)?;
        update(&mut collection)?;
        document["quick_commands"] =
            serde_json::to_value(&collection.commands).map_err(|_| "无法生成快捷指令配置")?;
        document["quick_command_groups"] =
            serde_json::to_value(&collection.groups).map_err(|_| "无法生成指令分组配置")?;
        write_workspace(path, &document).map_err(|_| "无法保存快捷指令配置，请重试")?;
        Ok(collection)
    }

    pub(crate) fn change_connection_group(
        &self,
        change: &ConnectionGroupChange,
        target: &str,
    ) -> Result<(String, HashSet<String>)> {
        let target = ConnectionProfile::group_name(target).map_err(|error| anyhow!(error))?;
        let path = &self.workspace.path;
        let _lock = lock_workspace(path).context("无法锁定连接配置")?;
        let mut document = self.workspace.load_document()?;
        let connections = document["connections"]
            .as_array_mut()
            .expect("workspace connections");
        let group_matches = |entry: &serde_json::Value, name: &str| {
            entry["group"]
                .as_str()
                .is_some_and(|group| group.trim() == name)
        };
        if let ConnectionGroupChange::Rename(previous) = change {
            if previous == "未分组" || target == "未分组" {
                bail!("请选择其他名称，未分组是保留分组");
            }
            if previous != target && connections.iter().any(|entry| group_matches(entry, target))
            {
                bail!("已有同名分组，请使用其他名称");
            }
        }

        let mut updated = HashSet::new();
        for entry in connections {
            let Some(id) = entry["id"].as_str() else {
                continue;
            };
            let selected = match change {
                ConnectionGroupChange::Move(ids) => ids.contains(id),
                ConnectionGroupChange::Rename(previous) => group_matches(entry, previous),
            };
            if selected {
                updated.insert(id.to_owned());
                entry["group"] = target.into();
            }
        }
        if updated.is_empty() {
            bail!("没有找到要更新的连接，请重新选择");
        }
        if let ConnectionGroupChange::Move(ids) = change
            && &updated != ids
        {
            bail!("部分连接已被删除，请重新选择");
        }
        write_workspace(path, &document).context("分组未保存，请重试")?;
        Ok((target.to_owned(), updated))
    }

    pub fn save_profile(
        &self,
        profile: &ConnectionProfile,
        previous: Option<&ConnectionProfile>,
    ) -> Result<Option<String>> {
        let path = &self.workspace.path;
        let _lock = lock_workspace(path).context("无法锁定连接配置")?;
        let identity_changed =
            previous.is_some_and(|previous| !credentials::same_identity(previous, profile));
        // Preserve settings and fields that the GPUI editor does not use.
        let mut document = self.workspace.load_document()?;
        let connections = document["connections"]
            .as_array_mut()
            .expect("workspace connections");
        if let Some(previous) = previous {
            let entry = connections
                .iter_mut()
                .find(|entry| {
                    entry.get("id").and_then(|id| id.as_str()) == Some(previous.id.as_str())
                })
                .ok_or_else(|| anyhow!("连接配置已被删除，请重新打开 GPUI"))?;
            let current: ConnectionProfile =
                serde_json::from_value(entry.clone()).map_err(|_| anyhow!("连接配置格式无效"))?;
            if &current != previous {
                bail!("连接配置已被其他窗口修改，请重新打开 GPUI");
            }
            *entry = serde_json::to_value(profile)?;
            if identity_changed
                && let Some(recent) = document
                    .get_mut("recent_connections")
                    .and_then(|entries| entries.as_array_mut())
            {
                recent.retain(|entry| {
                    entry.get("profileId").and_then(|id| id.as_str()) != Some(profile.id.as_str())
                });
            }
        } else {
            connections.push(serde_json::to_value(profile)?);
        }
        write_workspace(path, &document).context("连接配置未保存")?;
        Ok(if identity_changed {
            credentials::forget(&profile.id)
                .err()
                .map(|error| format!("连接已保存，但旧凭据未清除。{error}"))
        } else {
            None
        })
    }

    pub fn has_saved_credential(
        &self,
        profile: ConnectionProfile,
    ) -> JoinHandle<Result<bool, String>> {
        let workspace = self.workspace.clone();
        self.runtime.spawn(async move {
            workspace
                .with_profile(&profile, |current| {
                    credentials::load(current).map(|secret| secret.is_some())
                })
                .map_err(|error| format!("{error:#}"))?
        })
    }

    pub fn connect(
        &self,
        profile: ConnectionProfile,
        temporary: bool,
        mut secret: Zeroizing<String>,
        use_saved_credential: bool,
        remember_credential: bool,
        size: (u32, u32),
    ) -> (TerminalConnection, mpsc::UnboundedReceiver<SessionEvent>) {
        let (live, commands) = LiveSession::new(profile.clone(), temporary);
        if temporary {
            live.diagnostics.get_or_init(Default::default);
        }
        let control = Arc::new(TerminalControl::new(self.control_changed.clone()));
        let session_id = uuid::Uuid::new_v4().to_string();
        let id = session_id.clone();
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.retain(|_, record| {
                record
                    .live
                    .upgrade()
                    .is_some_and(|live| !live.cancel.is_cancelled())
            });
            sessions.insert(
                session_id.clone(),
                SessionRecord {
                    live: Arc::downgrade(&live),
                    control: control.clone(),
                },
            );
        }
        let (send, receive) = mpsc::unbounded_channel();
        let events =
            SessionEvents::new(move |event| send.send(event).map_err(|_| anyhow!("窗口已关闭")));
        let (input, inputs) = mpsc::channel(128);
        let (size_sender, sizes) = watch::channel(size);
        let workspace = Arc::clone(&self.workspace);
        let scope = Arc::new(SessionScope {
            workspace: workspace.clone(),
            control: control.clone(),
        });
        let running = Arc::clone(&live);
        self.runtime.spawn(async move {
            let saved = if temporary {
                Ok(Ok(None))
            } else {
                workspace.with_profile(&profile, |current| {
                    if use_saved_credential {
                        credentials::load(current)
                    } else {
                        Ok(None)
                    }
                })
            };
            let failure = match saved {
                Ok(Ok(Some(saved))) => {
                    secret = saved;
                    None
                }
                Ok(Ok(None)) if use_saved_credential => {
                    Some(("已保存凭据不可用，请重新输入".into(), true))
                }
                Ok(Ok(None)) => None,
                Ok(Err(message)) => Some((message, true)),
                Err(error) => Some((format!("{error:#}"), false)),
            };
            if let Some((message, authentication)) = failure {
                let _ = events.send(SessionEvent::Error {
                    message,
                    authentication,
                });
                running.cancel.cancel();
                return;
            }
            let request = ConnectRequest {
                session_id,
                profile_id: profile.id,
                profile: None,
                credential: std::mem::take(&mut *secret),
                use_saved_credential: false,
                remember_credential: remember_credential && !temporary,
                columns: size.0,
                rows: size.1,
            };
            session::run_session(scope, running, request, events, commands, sizes).await;
        });
        self.runtime
            .spawn(forward_input(live.clone(), inputs));
        (
            TerminalConnection {
                id,
                live,
                control,
                input,
                size: size_sender,
            },
            receive,
        )
    }

    pub(crate) fn server_metrics(
        &self,
        live: Arc<LiveSession>,
        inventory: bool,
    ) -> JoinHandle<Result<server_metrics::Sample>> {
        self.runtime.spawn(async move { server_metrics::sample(&live, inventory).await })
    }

    pub(crate) fn save_ssh_diagnostics(
        &self,
        session_id: String,
        diagnostics: Arc<ChannelDiagnostics>,
    ) -> JoinHandle<Result<PathBuf>> {
        let directory = self.workspace.path.with_file_name("diagnostics");
        self.runtime.spawn_blocking(move || {
            let snapshot = diagnostics.snapshot();
            let now = chrono::Local::now();
            let path = directory.join(format!(
                "bastion-{}-{}.json",
                now.format("%Y%m%d-%H%M%S"),
                uuid::Uuid::new_v4().simple(),
            ));
            fs::create_dir_all(&directory).context("无法创建诊断目录")?;
            let report = SshDiagnosticReport {
                session_id,
                saved_at: now.to_rfc3339(),
                snapshot,
            };
            write_workspace(&path, &report).context("无法写入 SSH 诊断文件")?;
            Ok(path)
        })
    }

    pub fn list(
        &self,
        live: Arc<LiveSession>,
        path: String,
    ) -> JoinHandle<Result<RemoteDirectory, String>> {
        self.runtime.spawn(async move {
            sftp::list(&live, &path)
                .await
                .map_err(|error| format!("{error:#}"))
        })
    }
}

async fn forward_input(
    live: Arc<LiveSession>,
    mut input: mpsc::Receiver<Zeroizing<Vec<u8>>>,
) {
    let _: Result<(), mpsc::error::SendError<SessionCommand>> = tokio::select! {
        _ = live.cancel.cancelled() => Ok(()),
        result = async {
            while let Some(data) = input.recv().await {
                for chunk in data.chunks(INPUT_CHUNK) {
                    live.commands.send(SessionCommand::Input { data: chunk.to_vec(), history: None }).await?;
                }
            }
            Ok(())
        } => result,
    };
}
