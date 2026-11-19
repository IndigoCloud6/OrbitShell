use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use orbitshell_ssh::{ai_shell::AiShell, command_guard::InputGuard, command_history, session::LiveSession};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct Binding {
    pub id: String,
    pub agent: String,
    pub name: String,
    pub cancel: CancellationToken,
    pub client: CancellationToken,
    pub service: CancellationToken,
    pub input: Arc<tokio::sync::Mutex<InputGuard>>,
}

impl Binding {
    fn active(&self) -> bool {
        !self.cancel.is_cancelled() && !self.client.is_cancelled() && !self.service.is_cancelled()
    }
}

#[derive(Clone)]
pub struct AgentInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub client: CancellationToken,
}

impl AgentInfo {
    pub fn connected(&self) -> bool {
        !self.client.is_cancelled()
    }

    pub fn binding(&self, live: &LiveSession) -> Binding {
        Binding {
            id: uuid::Uuid::new_v4().to_string(),
            agent: self.id.clone(),
            name: self.name.clone(),
            cancel: live.cancel.child_token(),
            client: self.client.clone(),
            service: self.client.clone(),
            input: Arc::new(tokio::sync::Mutex::default()),
        }
    }
}

pub struct Dedicated {
    pub id: String,
    pub session_id: String,
    pub session_name: String,
    pub agent: AgentInfo,
    pub shell: Arc<AiShell>,
    pub input: tokio::sync::Mutex<InputGuard>,
}

#[derive(Clone)]
pub struct Execution {
    pub session: String,
    pub agent: String,
    pub command: String,
}

#[derive(Default)]
struct Entries {
    agents: Vec<AgentInfo>,
    terminals: Vec<Arc<Dedicated>>,
    execution: VecDeque<Execution>,
}

pub struct AiWorkspace {
    entries: Mutex<Entries>,
    active: Mutex<Option<String>>,
    changed: watch::Sender<()>,
    history_path: Option<PathBuf>,
    history_error: Mutex<Option<&'static str>>,
}

impl Default for AiWorkspace {
    fn default() -> Self {
        Self {
            entries: Mutex::default(),
            active: Mutex::default(),
            changed: watch::channel(()).0,
            history_path: None,
            history_error: Mutex::default(),
        }
    }
}

impl AiWorkspace {
    pub fn open(history_path: Option<PathBuf>) -> Self {
        let mut workspace = Self { history_path, ..Self::default() };
        if let Some(path) = &workspace.history_path {
            match command_history::read(path) {
                Ok(records) => workspace.entries.get_mut().unwrap().execution.extend(records.into_iter().rev().filter(|entry| entry.source == "ai")
                    .take(100).map(|entry| Execution { session: entry.session_name, agent: entry.agent_name.unwrap_or_else(|| "AI".into()), command: entry.command }).collect::<Vec<_>>().into_iter().rev()),
                Err(_) => *workspace.history_error.get_mut().unwrap() = Some("历史文件无法读取，当前记录暂存于内存"),
            }
        }
        workspace
    }

    pub fn history_error(&self) -> Option<&'static str> {
        *self.history_error.lock().ok()?
    }

    pub fn subscribe(&self) -> watch::Receiver<()> {
        self.changed.subscribe()
    }

    pub fn notify(&self) {
        self.changed.send_replace(());
    }

    pub fn agent_connected(&self, info: AgentInfo) {
        if let Ok(mut entries) = self.entries.lock() {
            entries
                .agents
                .retain(|agent| agent.id != info.id && agent.connected());
            entries.agents.push(info);
        }
        self.notify();
    }

    pub fn agents(&self) -> Vec<AgentInfo> {
        self.entries
            .lock()
            .map(|entries| entries.agents.clone())
            .unwrap_or_default()
    }

    pub fn terminals(&self) -> Vec<Arc<Dedicated>> {
        self.entries
            .lock()
            .map(|entries| entries.terminals.clone())
            .unwrap_or_default()
    }

    pub fn active(&self) -> Option<String> {
        self.active.lock().ok()?.clone()
    }

    pub fn set_active(&self, id: Option<&str>) {
        if let Ok(mut active) = self.active.lock()
            && active.as_deref() != id
        {
            *active = id.map(str::to_owned);
        }
    }

    pub fn ensure_dedicated(
        &self,
        id: &str,
        live: Arc<LiveSession>,
        agent: AgentInfo,
        service: CancellationToken,
    ) -> Result<Arc<Dedicated>, &'static str> {
        let mut entries = self.entries.lock().map_err(|_| "AI 工作区暂不可用")?;
        if !agent.connected() || live.cancel.is_cancelled() {
            return Err("MCP 或 SSH 会话已断开");
        }
        if let Some(terminal) = entries.terminals.iter().find(|terminal| {
            terminal.session_id == id
                && terminal.agent.id == agent.id
                && !terminal.shell.cancel.is_cancelled()
        }) {
            return Ok(terminal.clone());
        }
        if entries.terminals.len() >= 32 {
            let index = entries
                .terminals
                .iter()
                .position(|terminal| terminal.shell.cancel.is_cancelled())
                .ok_or("独立 AI 终端数量已达到上限")?;
            entries.terminals.remove(index);
        }
        let terminal = Arc::new(Dedicated {
            id: uuid::Uuid::new_v4().to_string(),
            session_id: id.into(),
            session_name: if live.is_temporary {
                "堡垒机临时会话".into()
            } else {
                live.profile.name.clone()
            },
            shell: AiShell::start(live, self.changed.clone(), service, agent.client.clone()),
            agent,
            input: tokio::sync::Mutex::default(),
        });
        entries.terminals.push(terminal.clone());
        self.notify();
        Ok(terminal)
    }

    pub fn record(self: &Arc<Self>, live: &LiveSession, agent: &str, command: String) {
        let profile = &live.profile;
        let persisted = self.history_path.as_ref().filter(|_| !live.is_temporary).map(|path| (path.clone(), command_history::Entry {
            id: uuid::Uuid::new_v4().to_string(), host_key: format!("{}:{}", profile.hostname.to_lowercase(), profile.port),
            hostname: profile.hostname.clone(), port: profile.port, username: profile.username.clone(), session_name: profile.name.clone(),
            source: "ai".into(), agent_name: Some(agent.into()), redacted: command.starts_with("[包含敏感参数"), command: command.clone(),
            created_at: SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64, temporary: false,
        }));
        if let Ok(mut entries) = self.entries.lock() {
            if entries.execution.len() == 100 {
                entries.execution.pop_front();
            }
            entries.execution.push_back(Execution {
                session: if live.is_temporary {
                    "堡垒机临时会话".into()
                } else {
                    live.profile.name.clone()
                },
                agent: agent.into(),
                command,
            });
        }
        self.notify();
        if let Some((path, entry)) = persisted {
            let workspace = self.clone();
            tokio::task::spawn_blocking(move || {
                let warning = command_history::merge(&path, [entry]).err().map(|_| "执行历史保存失败，当前记录仍保留在内存中");
                if let Ok(mut error) = workspace.history_error.lock() { *error = warning; }
                workspace.notify();
            });
        }
    }

    pub fn executions(&self) -> Vec<Execution> {
        self.entries
            .lock()
            .map(|entries| entries.execution.iter().rev().cloned().collect())
            .unwrap_or_default()
    }
}

pub struct TerminalControl {
    owner: Mutex<Option<Binding>>,
    changed: watch::Sender<()>,
}

impl TerminalControl {
    pub fn new(changed: watch::Sender<()>) -> Self {
        Self {
            owner: Mutex::new(None),
            changed,
        }
    }

    pub fn binding(&self) -> Option<Binding> {
        self.owner
            .lock()
            .ok()?
            .as_ref()
            .filter(|owner| owner.active())
            .cloned()
    }

    pub fn claim(&self, binding: Binding) -> Result<Binding, &'static str> {
        let mut owner = self.owner.lock().map_err(|_| "终端接管状态暂不可用")?;
        if !binding.active() {
            return Err("MCP 会话已关闭");
        }
        if let Some(current) = owner.as_ref().filter(|owner| owner.active()) {
            return if current.agent == binding.agent {
                Ok(current.clone())
            } else {
                Err("此终端已由另一个 MCP 客户端接管")
            };
        }
        *owner = Some(binding.clone());
        self.changed.send_replace(());
        Ok(binding)
    }

    pub fn release(&self, id: &str) -> bool {
        let Ok(mut owner) = self.owner.lock() else {
            return false;
        };
        if owner.as_ref().is_none_or(|owner| owner.id != id) {
            return false;
        }
        if let Some(owner) = owner.take() {
            owner.cancel.cancel();
        }
        self.changed.send_replace(());
        true
    }

    pub fn read<T>(
        &self,
        id: &str,
        read: impl FnOnce() -> Result<T, &'static str>,
    ) -> Result<T, &'static str> {
        let owner = self.owner.lock().map_err(|_| "终端接管状态暂不可用")?;
        if owner
            .as_ref()
            .is_none_or(|owner| owner.id != id || !owner.active())
        {
            return Err("终端接管已结束");
        }
        read()
    }
}
