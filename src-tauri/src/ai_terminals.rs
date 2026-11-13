use crate::{ssh::{LiveSession, OutputBuffer, SessionCommand}, AppState};
use russh::{client, ChannelMsg, Pty};
use serde::Serialize;
use std::{sync::{Arc, Mutex, atomic::{AtomicU64, Ordering}}, time::{Duration, Instant, SystemTime, UNIX_EPOCH}};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::{sync::{mpsc, oneshot}, time::timeout};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use zeroize::Zeroizing;

const TERMINAL_LIMIT: usize = 32;
const CAPTURE_LIMIT: usize = 512 * 1024;

fn now() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64 }
fn label(value: &str) -> String { value.chars().filter(|c| !c.is_control()).take(100).collect() }

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Agent {
    pub id: String, pub name: String, pub version: String, pub connected: bool,
    pub connected_at: u64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalInfo {
    pub mode: String,
    pub id: String, pub agent_id: String, pub agent_name: String, pub session_id: String,
    pub session_name: String, pub hostname: String, pub port: u16, pub username: String,
    pub status: String, pub message: String, pub command: Option<String>,
    pub started_at: Option<u64>, pub elapsed_ms: u64, pub exit_code: Option<u32>,
    pub updated_at: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot { revision: u64, agents: Vec<Agent>, terminals: Vec<TerminalInfo> }

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalOutput { pub data: Vec<u8>, pub cursor: u64, pub truncated: bool }

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandResult {
    pub terminal_id: String, pub stdout: String, pub stderr: String, pub exit_code: Option<u32>,
    pub elapsed_ms: u64, pub running: bool, pub truncated: bool, pub note: String,
}

enum Input {
    Execute { command: String, result: oneshot::Sender<Result<CommandResult, String>> },
    Literal { data: Zeroizing<Vec<u8>>, display_command: Option<String>, result: oneshot::Sender<Result<(), String>> },
    Resize(u32, u32),
}

pub struct AiTerminal {
    info: Mutex<TerminalInfo>, output: Mutex<OutputBuffer>, input: mpsc::Sender<Input>,
    cancel: CancellationToken,
    access: Mutex<()>,
    source: Option<Arc<LiveSession>>,
}

impl AiTerminal {
    fn revoke(&self) {
        let _access = self.access.lock().unwrap_or_else(|error| error.into_inner());
        self.cancel.cancel();
    }
    pub fn read_for_agent(&self, cursor: Option<u64>, max_bytes: usize) -> Result<TerminalOutput, String> {
        // Linearize MCP reads with release: a resolved Arc is not a continuing read capability.
        let _access = self.access.lock().map_err(|_| "终端访问状态不可用")?;
        if let Some(source) = &self.source {
            if self.cancel.is_cancelled() || source.cancel.is_cancelled() { return Err("原标签接管已结束，不能继续读取人工终端。".into()); }
        }
        self.read(cursor, max_bytes)
    }
    pub fn info(&self) -> Result<TerminalInfo, String> { self.info.lock().map(|v| v.clone()).map_err(|_| "AI 终端状态不可用".into()) }
    fn patch(&self, app: &AppHandle, change: impl FnOnce(&mut TerminalInfo)) {
        if let Ok(mut info) = self.info.lock() { change(&mut info); info.updated_at = now(); }
        app.state::<AppState>().ai.notify(app, None);
    }
    fn append(&self, app: &AppHandle, data: &[u8]) {
        if data.is_empty() { return; }
        if let Ok(mut output) = self.output.lock() { output.append(data); }
        if let Ok(info) = self.info.lock() { app.state::<AppState>().ai.notify(app, Some(&info.id)); }
    }
    pub fn read(&self, cursor: Option<u64>, max_bytes: usize) -> Result<TerminalOutput, String> {
        let output = if let Some(source) = &self.source { source.output.lock() } else { self.output.lock() }.map_err(|_| "AI 终端输出不可用")?;
        let first = output.end - output.bytes.len() as u64;
        let requested = cursor.unwrap_or(first);
        let start = requested.clamp(first, output.end);
        let data: Vec<_> = output.bytes.iter().skip((start - first) as usize).take(max_bytes.clamp(1, 65536)).copied().collect();
        Ok(TerminalOutput { cursor: start + data.len() as u64, data, truncated: requested < first })
    }
    pub async fn execute(&self, command: String, seconds: u64) -> Result<CommandResult, String> {
        if self.cancel.is_cancelled() { return Err("AI 终端已关闭，命令未发送".into()); }
        let (tx, rx) = oneshot::channel();
        self.input.try_send(Input::Execute { command, result: tx }).map_err(|_| "AI 终端输入队列繁忙或已关闭")?;
        match timeout(Duration::from_secs(seconds.clamp(1, 300)), rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("AI 终端已关闭".into()),
            Err(_) => {
                let info = self.info()?;
                if self.cancel.is_cancelled() || matches!(info.status.as_str(), "closed" | "error") { return Err(info.message); }
                Ok(CommandResult { terminal_id: info.id, stdout: String::new(), stderr: String::new(), exit_code: None,
                    elapsed_ms: info.started_at.map(|start| now().saturating_sub(start)).unwrap_or(0), running: true, truncated: false,
                    note: "等待时间已到，命令仍在 AI 终端中排队或运行；使用 read_output 继续读取，不要重复执行。".into() })
            },
        }
    }
    pub async fn input(&self, data: Vec<u8>, display_command: Option<String>) -> Result<(), String> {
        if self.cancel.is_cancelled() { return Err("AI 终端已关闭，输入未发送".into()); }
        let (tx, rx) = oneshot::channel();
        self.input.try_send(Input::Literal { data: Zeroizing::new(data), display_command, result: tx }).map_err(|_| "AI 终端输入队列繁忙或已关闭")?;
        timeout(Duration::from_secs(20), rx).await.map_err(|_| "终端尚未就绪，输入仍在队列中，请先读取输出，不要重复发送")?
            .map_err(|_| "AI 终端已关闭")?
    }
}

#[derive(Default)]
struct Entries { agents: Vec<Agent>, terminals: Vec<Arc<AiTerminal>> }
#[derive(Default)]
pub struct AiTerminals { entries: Mutex<Entries>, revision: AtomicU64, active_session: Mutex<Option<String>> }

impl AiTerminals {
    pub fn active_session(&self) -> Option<String> { self.active_session.lock().ok().and_then(|id| id.clone()) }
    fn notify(&self, app: &AppHandle, output_id: Option<&str>) {
        let revision = self.revision.fetch_add(1, Ordering::Relaxed) + 1;
        let _ = app.emit_to("main", "orbitshell:ai-changed", serde_json::json!({"revision": revision, "outputId": output_id}));
    }
    pub fn connect_agent(&self, app: &AppHandle, id: &str, name: &str, version: &str) -> Result<(), String> {
        let mut entries = self.entries.lock().map_err(|_| "Agent 状态不可用")?;
        if let Some(agent) = entries.agents.iter_mut().find(|a| a.id == id) { agent.connected = true; }
        else {
            if entries.agents.len() >= 64 {
                let removable = entries.agents.iter().position(|a| !a.connected && !entries.terminals.iter().any(|t| t.info().is_ok_and(|i| i.agent_id == a.id)));
                if let Some(index) = removable { entries.agents.remove(index); }
                else { return Err("MCP 客户端会话数量达到上限，请关闭不用的连接".into()); }
            }
            let name = label(name);
            entries.agents.push(Agent { id: id.into(), name: if name.is_empty() { "MCP Agent".into() } else { name }, version: label(version), connected: true, connected_at: now() });
        }
        drop(entries); self.notify(app, None); Ok(())
    }
    pub fn disconnect_agent(&self, app: &AppHandle, id: &str) {
        if let Ok(mut entries) = self.entries.lock() {
            if let Some(agent) = entries.agents.iter_mut().find(|a| a.id == id) { agent.connected = false; }
            for terminal in &entries.terminals {
                if terminal.source.is_some() && terminal.info().is_ok_and(|i| i.agent_id == id) { terminal.revoke(); }
            }
        }
        self.notify(app, None);
    }
    pub fn stop(&self, app: &AppHandle) {
        if let Ok(mut entries) = self.entries.lock() {
            for agent in &mut entries.agents { agent.connected = false; }
            for terminal in &entries.terminals { terminal.revoke(); }
        }
        self.notify(app, None);
    }
    pub fn terminal(&self, id: &str) -> Result<Arc<AiTerminal>, String> {
        self.entries.lock().map_err(|_| "AI 终端状态不可用")?.terminals.iter()
            .find(|t| t.info().is_ok_and(|i| i.id == id)).cloned().ok_or_else(|| "AI 终端已被清理".into())
    }
    pub fn existing(&self, agent_id: &str, session_id: &str) -> Result<Arc<AiTerminal>, String> {
        let entries = self.entries.lock().map_err(|_| "AI 终端状态不可用")?;
        let matches: Vec<_> = entries.terminals.iter().filter(|t| t.info().is_ok_and(|i| i.agent_id == agent_id && i.session_id == session_id)).collect();
        matches.iter().rev().find(|t| t.source.is_some() && !t.cancel.is_cancelled()).or_else(|| matches.last()).map(|t| Arc::clone(t))
            .ok_or_else(|| "该 Agent 尚未选择终端。要操作已有标签，请调用 take_over_session，或 send_input(action=takeover, data=空字符串)，并指定 list_sessions 中完整的 sessionId；不能把另开的独立 Shell 当作接管。".into())
    }
    pub fn resolve(&self, app: &AppHandle, agent_id: &str, session_id: &str, terminal_id: Option<&str>, mode: Option<&str>, writing: bool) -> Result<Arc<AiTerminal>, String> {
        let terminal = if let Some(id) = terminal_id {
            let terminal = self.terminal(id)?;
            let info = terminal.info()?;
            if info.agent_id != agent_id { return Err("终端属于另一个 MCP 客户端会话；重新连接后必须重新接管，不能按 Agent 名称继承控制权。".into()); }
            if info.session_id != session_id || mode.is_some_and(|mode| mode != info.mode) { return Err("terminal_id、session_id 或 mode 不匹配，未发送输入。".into()); }
            terminal
        } else if mode == Some("takeover") {
            // Choosing a mode never implicitly takes control of an existing terminal.
            let terminal = self.existing(agent_id, session_id)?;
            if terminal.source.is_none() { return Err("尚未接管原标签。请先调用 take_over_session 或 send_input(action=takeover, data=空字符串)；mode=takeover 不会自动新开 PTY 或替代接管。".into()); }
            terminal
        } else if writing {
            self.ensure(app, agent_id, session_id, mode == Some("dedicated"))?
        } else if mode == Some("dedicated") {
            self.entries.lock().map_err(|_| "AI 终端状态不可用")?.terminals.iter().rev().find(|t| t.source.is_none() && t.info().is_ok_and(|i| i.agent_id == agent_id && i.session_id == session_id)).cloned().ok_or("尚未打开独立 AI 终端")?
        } else { self.existing(agent_id, session_id)? };
        if terminal.source.is_some() && terminal.cancel.is_cancelled() { return Err("原标签接管已结束，不能继续输入或读取人工终端，也不会切到独立 Shell。用户再次授权后重新调用 take_over_session 或 send_input(action=takeover, data=空字符串)。".into()); }
        if writing && terminal.cancel.is_cancelled() { return Err("该终端已关闭，输入未发送。".into()); }
        Ok(terminal)
    }
    pub fn session_control(&self, session_id: &str, agent_id: &str) -> Result<serde_json::Value, String> {
        let entries = self.entries.lock().map_err(|_| "AI 终端状态不可用")?;
        let owner = entries.terminals.iter().find(|t| t.source.is_some() && !t.cancel.is_cancelled() && t.info().is_ok_and(|i| i.session_id == session_id)).map(|t| t.info()).transpose()?;
        let target = owner.as_ref().filter(|info| info.agent_id == agent_id).cloned().or(entries.terminals.iter().rev().find(|t| !t.cancel.is_cancelled() && t.info().is_ok_and(|i| i.agent_id == agent_id && i.session_id == session_id)).map(|t| t.info()).transpose()?);
        Ok(serde_json::json!({"originalTerminalOwner": owner.as_ref().map(|i| serde_json::json!({"agentId": i.agent_id, "agentName": i.agent_name, "terminalId": i.id, "ownedByThisAgent": i.agent_id == agent_id})),
            "thisAgentTerminal": target.as_ref().map(|i| serde_json::json!({"terminalId": i.id, "mode": i.mode, "sharesOriginalTerminal": i.mode == "takeover"})),
            "takeoverTool": "take_over_session"}))
    }
    pub fn owns(&self, terminal_id: &str, session_id: &str) -> bool {
        self.entries.lock().is_ok_and(|entries| entries.terminals.iter().any(|t| t.source.is_some() && !t.cancel.is_cancelled() && t.info().is_ok_and(|i| i.id == terminal_id && i.session_id == session_id)))
    }
    pub fn taken_over(&self, session_id: &str) -> bool {
        self.entries.lock().is_ok_and(|entries| entries.terminals.iter().any(|t| t.source.is_some() && !t.cancel.is_cancelled() && t.info().is_ok_and(|i| i.session_id == session_id)))
    }
    pub fn source_output(&self, app: &AppHandle, session_id: &str) {
        if let Ok(entries) = self.entries.lock() {
            for terminal in &entries.terminals {
                if terminal.source.is_some() && !terminal.cancel.is_cancelled() {
                    if let Ok(info) = terminal.info() { if info.session_id == session_id { self.notify(app, Some(&info.id)); } }
                }
            }
        }
    }
    pub fn release(&self, id: &str, agent_id: Option<&str>) -> Result<(), String> {
        let terminal = self.terminal(id)?;
        if terminal.source.is_none() { return Err("该标签不是接管的人工终端".into()); }
        if agent_id.is_some_and(|id| terminal.info().is_ok_and(|i| i.agent_id != id)) { return Err("该终端由另一个 Agent 接管".into()); }
        terminal.revoke(); Ok(())
    }
    pub fn take_over(&self, app: &AppHandle, agent_id: &str, session_id: &str) -> Result<Arc<AiTerminal>, String> {
        let live = app.state::<AppState>().session(session_id)?;
        if !live.connected.load(Ordering::Acquire) || live.cancel.is_cancelled() { return Err("请先在桌面完成 SSH 登录和主机身份确认".into()); }
        let mut entries = self.entries.lock().map_err(|_| "AI 终端状态不可用")?;
        if let Some(owned) = entries.terminals.iter().find(|t| t.source.is_some() && !t.cancel.is_cancelled() && t.info().is_ok_and(|i| i.session_id == session_id)) {
            return if owned.info()?.agent_id == agent_id { Ok(owned.clone()) } else { Err("该人工终端已由另一个 Agent 接管，请先交回人工".into()) };
        }
        let agent = entries.agents.iter().find(|a| a.id == agent_id && a.connected).cloned().ok_or("MCP 客户端尚未初始化")?;
        if entries.terminals.len() >= TERMINAL_LIMIT {
            if let Some(index) = entries.terminals.iter().position(|t| t.cancel.is_cancelled()) { entries.terminals.remove(index); }
            else { return Err("AI 终端数量达到上限".into()); }
        }
        let (tx, rx) = mpsc::channel(32);
        let terminal = Arc::new(AiTerminal { info: Mutex::new(TerminalInfo {
            mode: "takeover".into(), id: Uuid::new_v4().to_string(), agent_id: agent.id, agent_name: agent.name,
            session_id: session_id.into(), session_name: live.profile.name.clone(), hostname: live.profile.hostname.clone(), port: live.profile.port,
            username: if live.profile.username.starts_with("b64>>") { "堡垒机会话".into() } else { live.profile.username.clone() },
            status: "interactive".into(), message: "已接管当前终端 · 人工输入已锁定 · 退出码未知".into(), command: None, started_at: None, elapsed_ms: 0, exit_code: None, updated_at: now(),
        }), output: Mutex::new(OutputBuffer::new()), input: tx, cancel: CancellationToken::new(), access: Mutex::new(()), source: Some(live.clone()) });
        entries.terminals.push(terminal.clone()); drop(entries); self.notify(app, None);
        let task_terminal = terminal.clone(); let task_app = app.clone();
        tauri::async_runtime::spawn(async move { run_takeover(task_app, task_terminal, live, rx).await; });
        Ok(terminal)
    }
    fn ensure(&self, app: &AppHandle, agent_id: &str, session_id: &str, dedicated_only: bool) -> Result<Arc<AiTerminal>, String> {
        let live = app.state::<AppState>().session(session_id)?;
        if !live.connected.load(Ordering::Acquire) || live.cancel.is_cancelled() { return Err("SSH 尚未认证成功或已断开".into()); }
        let mut entries = self.entries.lock().map_err(|_| "AI 终端状态不可用")?;
        if !dedicated_only {
            if let Some(takeover) = entries.terminals.iter().rev().find(|t| t.source.is_some() && !t.cancel.is_cancelled() && t.info().is_ok_and(|i| i.agent_id == agent_id && i.session_id == session_id)) { return Ok(takeover.clone()); }
            if entries.terminals.iter().rev().find(|t| t.info().is_ok_and(|i| i.agent_id == agent_id && i.session_id == session_id)).is_some_and(|t| t.source.is_some()) {
                return Err("原标签已交回人工；未新建 PTY。重新接管请调用 take_over_session 或 send_input(action=takeover, data=空字符串)，需要独立终端请明确设置 mode=dedicated。".into());
            }
        }
        if let Some(existing) = entries.terminals.iter().rev().find(|t| t.source.is_none() && !t.cancel.is_cancelled() && t.info().is_ok_and(|i| i.agent_id == agent_id && i.session_id == session_id)) {
            return Ok(existing.clone());
        }
        let agent = entries.agents.iter().find(|a| a.id == agent_id && a.connected).cloned().ok_or("MCP 客户端尚未初始化或已断开")?;
        if entries.terminals.len() >= TERMINAL_LIMIT {
            if let Some(index) = entries.terminals.iter().position(|t| t.cancel.is_cancelled()) { entries.terminals.remove(index); }
            else { return Err("AI 终端数量达到上限，请断开不再使用的远程会话".into()); }
        }
        let (tx, rx) = mpsc::channel(32);
        let terminal = Arc::new(AiTerminal { info: Mutex::new(TerminalInfo {
            mode: "dedicated".into(), id: Uuid::new_v4().to_string(), agent_id: agent.id, agent_name: agent.name,
            session_id: session_id.into(), session_name: live.profile.name.clone(), hostname: live.profile.hostname.clone(),
            port: live.profile.port, username: if live.profile.username.starts_with("b64>>") { "堡垒机会话".into() } else { live.profile.username.clone() },
            status: "connecting".into(), message: "正在打开独立 AI 终端…".into(), command: None, started_at: None, elapsed_ms: 0, exit_code: None, updated_at: now(),
        }), output: Mutex::new(OutputBuffer::new()), input: tx, cancel: CancellationToken::new(), access: Mutex::new(()), source: None });
        entries.terminals.push(terminal.clone());
        drop(entries); self.notify(app, None);
        let task_terminal = terminal.clone(); let task_app = app.clone();
        tauri::async_runtime::spawn(async move { run(task_app, task_terminal, live, rx).await; });
        Ok(terminal)
    }
}

// Retain partial framing sequences across SSH packets; only the per-command nonce is recognized.
struct Marker { prefix: Vec<u8>, pending: Vec<u8> }
impl Marker {
    fn push(&mut self, bytes: &[u8]) -> (Vec<u8>, Option<u32>) {
        self.pending.extend_from_slice(bytes);
        let mut visible = Vec::new();
        loop {
            if let Some(index) = self.pending.windows(self.prefix.len()).position(|v| v == self.prefix) {
                visible.extend(self.pending.drain(..index));
                if let Some(end) = self.pending[self.prefix.len()..].iter().position(|b| *b == 7) {
                    let end = self.prefix.len() + end;
                    if let Ok(code) = std::str::from_utf8(&self.pending[self.prefix.len()..end]).unwrap_or("").parse::<u32>() {
                        self.pending.drain(..=end); visible.append(&mut self.pending); return (visible, Some(code));
                    }
                } else if self.pending.len() < self.prefix.len() + 16 { return (visible, None); }
                visible.push(self.pending.remove(0));
            } else {
                let keep = (1..self.prefix.len()).rev().find(|length| self.pending.ends_with(&self.prefix[..*length])).unwrap_or(0);
                let count = self.pending.len() - keep;
                visible.extend(self.pending.drain(..count));
                return (visible, None);
            }
        }
    }
}

struct Running {
    marker: Marker, result: oneshot::Sender<Result<CommandResult, String>>, started: Instant,
    capture: Vec<u8>, truncated: bool,
}
impl Running {
    fn capture(&mut self, data: &[u8]) {
        let count = data.len().min(CAPTURE_LIMIT.saturating_sub(self.capture.len()));
        self.capture.extend_from_slice(&data[..count]); self.truncated |= count != data.len();
    }
    fn finish(self, id: String, exit_code: Option<u32>, note: String) {
        let _ = self.result.send(Ok(CommandResult { terminal_id: id, stdout: String::from_utf8_lossy(&self.capture).into_owned(),
            stderr: String::new(), exit_code, elapsed_ms: self.started.elapsed().as_millis() as u64, running: false, truncated: self.truncated, note }));
    }
}

async fn accept(channel: &mut russh::Channel<client::Msg>, terminal: &AiTerminal, app: &AppHandle) -> Result<(), String> {
    loop {
        match channel.wait().await {
            Some(ChannelMsg::Success) => return Ok(()),
            Some(ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. }) => terminal.append(app, &data),
            Some(ChannelMsg::Failure | ChannelMsg::Close) | None => return Err("服务器拒绝独立 AI 终端，未向人工终端发送任何命令".into()),
            _ => {}
        }
    }
}

struct OpeningChannel(Option<russh::Channel<client::Msg>>);
impl Drop for OpeningChannel {
    fn drop(&mut self) { if let Some(channel) = self.0.take() { drop(channel.into_stream()); } }
}

async fn open(live: &LiveSession, terminal: &AiTerminal, app: &AppHandle) -> Result<russh::Channel<client::Msg>, String> {
    let connection = live.connection.get().ok_or("SSH 尚未就绪")?;
    let mut pending = OpeningChannel(Some(connection.channel_open_session().await.map_err(|_| "无法建立独立 AI 通道")?));
    let channel = pending.0.as_mut().ok_or("AI 通道已关闭")?;
    let result = timeout(Duration::from_secs(15), async {
        // Echo known commands ourselves; literal prompt answers must never be logged as commands.
        channel.request_pty(true, "xterm-256color", 120, 36, 0, 0, &[(Pty::ECHO, 0), (Pty::ECHONL, 0)]).await.map_err(|_| "无法申请 AI 终端")?;
        accept(channel, terminal, app).await?;
        channel.request_shell(true).await.map_err(|_| "无法启动 AI Shell")?;
        accept(channel, terminal, app).await
    }).await.unwrap_or_else(|_| Err("独立 AI 终端初始化超时".into()));
    if let Err(error) = result { let _ = timeout(Duration::from_secs(2), channel.close()).await; return Err(error); }
    pending.0.take().ok_or_else(|| "AI 通道已关闭".into())
}

async fn run(app: AppHandle, terminal: Arc<AiTerminal>, live: Arc<LiveSession>, mut inputs: mpsc::Receiver<Input>) {
    let opened = tokio::select! {
        _ = terminal.cancel.cancelled() => Err("AI 终端已停止".into()),
        _ = live.cancel.cancelled() => Err("所属 SSH 会话已断开".into()),
        value = timeout(Duration::from_secs(20), open(&live, &terminal, &app)) => value.unwrap_or_else(|_| Err("打开 AI 通道超时".into())),
    };
    let channel = match opened {
        Ok(channel) => channel,
        Err(error) => {
            terminal.patch(&app, |i| { i.status = "error".into(); i.message = error.clone(); });
            terminal.append(&app, format!("\r\n[OrbitShell] {error}\r\n").as_bytes());
            terminal.cancel.cancel(); return;
        }
    };
    let (mut reader, writer) = channel.split();
    terminal.patch(&app, |i| { i.status = "idle".into(); i.message = "AI 独立终端已连接 · 仅查看".into(); });
    let mut running: Option<Running> = None;
    let message = loop {
        tokio::select! {
            biased;
            _ = terminal.cancel.cancelled() => break "AI 终端已停止；远端进程状态以服务器为准",
            _ = live.cancel.cancelled() => break "所属 SSH 会话已断开",
            input = inputs.recv() => match input {
                Some(Input::Resize(columns, rows)) => { let _ = writer.window_change(columns.clamp(20, 500), rows.clamp(5, 300), 0, 0).await; },
                Some(Input::Literal { data, display_command, result }) => {
                    if let Some(command) = &display_command {
                        terminal.append(&app, format!("\r\n\x1b[36m$ \x1b[0m{}\r\n", command.replace('\n', "\r\n")).as_bytes());
                    }
                    let submitted = data.last().is_some_and(|b| matches!(*b, b'\r' | b'\n'));
                    let sent = writer.data_bytes(data.to_vec()).await.map_err(|_| "AI 终端输入发送失败".to_string());
                    if sent.is_ok() && submitted {
                        if let Some(command) = display_command { if let Ok(info) = terminal.info() { app.state::<AppState>().history.record(&app, &info.session_id, Uuid::new_v4().to_string(), &command, Some(&info.agent_name)); } }
                    }
                    if sent.is_ok() && running.is_none() { terminal.patch(&app, |i| { i.status = "interactive".into(); i.message = "交互输入已发送，命令退出状态未知".into(); i.exit_code = None; }); }
                    let failed = sent.is_err(); let _ = result.send(sent);
                    if failed { break "AI 终端输入发送失败"; }
                },
                Some(Input::Execute { command, result }) => {
                    if running.is_some() { let _ = result.send(Err("该 AI 终端已有命令在运行，请读取输出或用 send_input 交互，不要重复执行".into())); continue; }
                    let nonce = Uuid::new_v4().simple().to_string();
                    let prefix = format!("\x1b]633;OrbitShell;{nonce};").into_bytes();
                    // eval runs in the existing POSIX shell, so cd/export persist. Only a quoted command is inserted.
                    // Continue outside quoted chunks to stay below the PTY canonical line limit.
                    let chars: Vec<_> = command.chars().collect();
                    let quoted = chars.chunks(512).map(|chunk| format!("'{}'", chunk.iter().collect::<String>().replace('\'', "'\"'\"'"))).collect::<Vec<_>>().join("\\\n");
                    let wire = format!("eval {quoted}; command printf '\\033]633;OrbitShell;{nonce};%s\\007' \"$?\"\r");
                    terminal.append(&app, format!("\r\n\x1b[36m$ \x1b[0m{}\r\n", command.replace('\n', "\r\n")).as_bytes());
                    let history_command = command.clone();
                    terminal.patch(&app, |i| { i.status = "running".into(); i.message = "AI 操作中 · 仅查看".into(); i.command = Some(command); i.started_at = Some(now()); i.elapsed_ms = 0; i.exit_code = None; });
                    running = Some(Running { marker: Marker { prefix, pending: Vec::new() }, result, started: Instant::now(), capture: Vec::new(), truncated: false });
                    if writer.data_bytes(wire.into_bytes()).await.is_err() { break "AI 命令发送失败"; }
                    if let Ok(info) = terminal.info() { app.state::<AppState>().history.record(&app, &info.session_id, Uuid::new_v4().to_string(), &history_command, Some(&info.agent_name)); }
                },
                None => break "AI 终端已关闭",
            },
            message = reader.wait() => match message {
                Some(ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. }) => {
                    let (visible, code) = if let Some(command) = running.as_mut() {
                        let (visible, code) = command.marker.push(&data); command.capture(&visible); (visible, code)
                    } else { (data.to_vec(), None) };
                    terminal.append(&app, &visible);
                    if let Some(code) = code {
                        if let Some(command) = running.take() {
                            let elapsed = command.started.elapsed().as_millis() as u64;
                            terminal.patch(&app, |i| { i.status = "idle".into(); i.message = "等待 Agent 指令".into(); i.elapsed_ms = elapsed; i.exit_code = Some(code); });
                            command.finish(terminal.info().map(|i| i.id).unwrap_or_default(), Some(code), "PTY 输出合并 stdout/stderr；退出码来自当前 Shell。".into());
                        }
                    }
                },
                Some(ChannelMsg::Failure) => break "服务器拒绝 AI 终端请求",
                Some(ChannelMsg::Close) | None => break "远程 AI Shell 已结束",
                _ => {}
            }
        }
    };
    if let Some(command) = running.take() {
        let elapsed = command.started.elapsed().as_millis() as u64;
        terminal.patch(&app, |i| { i.elapsed_ms = elapsed; i.exit_code = None; });
        command.finish(terminal.info().map(|i| i.id).unwrap_or_default(), None, message.into());
    }
    terminal.cancel.cancel();
    terminal.patch(&app, |i| { i.status = "closed".into(); i.message = message.into(); });
    terminal.append(&app, format!("\r\n[OrbitShell] {message}\r\n").as_bytes());
    let _ = timeout(Duration::from_secs(2), writer.close()).await;
}

// Explicit takeover reuses the original PTY and its authentication / shell state.
// It sends literal input, without wrappers or prompt-based guesses about command completion.
async fn run_takeover(app: AppHandle, terminal: Arc<AiTerminal>, live: Arc<LiveSession>, mut inputs: mpsc::Receiver<Input>) {
    async fn deliver(terminal: &AiTerminal, live: &LiveSession, data: Zeroizing<Vec<u8>>) -> Result<(), String> {
        let (complete, response) = oneshot::channel();
        let input = SessionCommand::AiInput { owner: terminal.info()?.id, data, complete };
        tokio::select! {
            biased;
            _ = terminal.cancel.cancelled() => return Err("AI 接管已结束，输入未发送".into()),
            _ = live.cancel.cancelled() => return Err("SSH 会话已断开".into()),
            sent = live.commands.send(input) => sent.map_err(|_| "终端已关闭")?,
        }
        tokio::select! {
            biased;
            _ = terminal.cancel.cancelled() => Err("AI 接管已结束，请读取终端确认最后一次输入的状态".into()),
            _ = live.cancel.cancelled() => Err("SSH 会话已断开".into()),
            result = timeout(Duration::from_secs(5), response) => result.map_err(|_| "输入发送状态未确认，请读取输出后决定，不要重复发送")?.map_err(|_| "终端已关闭")?,
        }
    }
    loop {
        tokio::select! {
            biased;
            _ = terminal.cancel.cancelled() => break,
            _ = live.cancel.cancelled() => break,
            input = inputs.recv() => match input {
                Some(Input::Execute { command, result }) => {
                    let input = format!("{}\r", command.replace("\r\n", "\n").replace('\n', "\r").trim_end_matches('\r')).into_bytes();
                    let sent = deliver(&terminal, &live, Zeroizing::new(input)).await;
                    if sent.is_ok() {
                        if let Ok(info) = terminal.info() { app.state::<AppState>().history.record(&app, &info.session_id, Uuid::new_v4().to_string(), &command, Some(&info.agent_name)); }
                        terminal.patch(&app, |i| { i.status = "interactive".into(); i.command = Some(command); i.started_at = Some(now()); i.exit_code = None; i.elapsed_ms = 0; i.message = "AI 接管中 · 交互输入已发送，退出码未知".into(); });
                    }
                    let _ = result.send(sent.and_then(|_| Ok(CommandResult { terminal_id: terminal.info()?.id, stdout: String::new(), stderr: String::new(), exit_code: None,
                        elapsed_ms: 0, running: true, truncated: false, note: "已向接管的原始 PTY 发送命令。使用 read_output 读取命令回显和实时输出；不推断提示符或退出码。确认输出后可继续发送指令，release_session 交回人工。".into() })));
                },
                Some(Input::Literal { data, display_command, result }) => {
                    // Preserve the remote PTY's own echo rules, especially for password prompts.
                    let submitted = data.last().is_some_and(|b| matches!(*b, b'\r' | b'\n'));
                    let sent = deliver(&terminal, &live, data).await;
                    if sent.is_ok() && submitted {
                        if let Some(command) = display_command { if let Ok(info) = terminal.info() { app.state::<AppState>().history.record(&app, &info.session_id, Uuid::new_v4().to_string(), &command, Some(&info.agent_name)); } }
                    }
                    if sent.is_ok() { terminal.patch(&app, |i| { i.message = "AI 接管中 · 交互输入已发送，退出码未知".into(); }); }
                    let _ = result.send(sent);
                },
                Some(Input::Resize(_, _)) => {},
                None => break,
            }
        }
    }
    terminal.cancel.cancel();
    terminal.patch(&app, |i| { i.status = "closed".into(); i.message = if live.cancel.is_cancelled() { "所属 SSH 会话已断开".into() } else { "已交回人工；远端命令可能仍在运行".into() }; });
}

#[tauri::command]
pub fn ai_workspace(state: State<'_, AppState>) -> Result<Snapshot, String> {
    let entries = state.ai.entries.lock().map_err(|_| "Agent 状态不可用")?;
    let terminals = entries.terminals.iter().map(|t| t.info()).collect::<Result<Vec<_>, _>>()?;
    Ok(Snapshot { revision: state.ai.revision.load(Ordering::Relaxed), agents: entries.agents.clone(), terminals })
}
#[tauri::command]
pub fn ai_release_session(id: String, state: State<'_, AppState>) -> Result<(), String> { state.ai.release(&id, None) }
#[tauri::command]
pub fn ai_take_over_session(session_id: String, agent_id: String, app: AppHandle) -> Result<TerminalInfo, String> {
    if !app.state::<AppState>().mcp_enabled.load(Ordering::Acquire) { return Err("请先开启 MCP 并连接 Agent".into()); }
    app.state::<AppState>().ai.take_over(&app, &agent_id, &session_id)?.info()
}
#[tauri::command]
pub fn ai_set_active_session(session_id: Option<String>, state: State<'_, AppState>) -> Result<(), String> {
    *state.ai.active_session.lock().map_err(|_| "工作区会话状态不可用")? = session_id; Ok(())
}
#[tauri::command]
pub fn ai_terminal_output(id: String, cursor: Option<u64>, state: State<'_, AppState>) -> Result<TerminalOutput, String> { state.ai.terminal(&id)?.read(cursor, 65536) }
#[tauri::command]
pub async fn ai_terminal_resize(id: String, columns: u32, rows: u32, state: State<'_, AppState>) -> Result<(), String> {
    state.ai.terminal(&id)?.input.try_send(Input::Resize(columns, rows)).map_err(|_| "AI 终端尺寸更新队列繁忙".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use tokio::sync::OnceCell;

    fn takeover() -> Arc<AiTerminal> {
        let (commands, _) = mpsc::channel(8);
        let live = LiveSession {
            profile: crate::storage::ConnectionProfile { id: "profile".into(), name: "Demo".into(), group: "Demo".into(), hostname: "192.0.2.20".into(), port: 22, username: "demo".into(), auth_method: "password".into(), private_key_path: None },
            is_temporary: false, connected: AtomicBool::new(true), output: Mutex::new(OutputBuffer::new()), commands,
            cancel: CancellationToken::new(), trust: Mutex::new(None), verification_error: Mutex::new(None), connection: OnceCell::new(), sftp: OnceCell::new(),
        };
        let (input, _) = mpsc::channel(8);
        Arc::new(AiTerminal { info: Mutex::new(TerminalInfo { mode: "takeover".into(), id: "terminal".into(), agent_id: "agent".into(), agent_name: "Demo Agent".into(), session_id: "session".into(), session_name: "Demo".into(), hostname: "192.0.2.20".into(), port: 22, username: "demo".into(), status: "interactive".into(), message: String::new(), command: None, started_at: None, elapsed_ms: 0, exit_code: None, updated_at: 1 }),
            output: Mutex::new(OutputBuffer::new()), input, cancel: CancellationToken::new(), access: Mutex::new(()), source: Some(Arc::new(live)) })
    }

    #[test]
    fn takeover_reads_the_original_pty_and_revocation_invalidates_resolved_handles() {
        let terminal = takeover();
        terminal.source.as_ref().unwrap().output.lock().unwrap().append(b"original-shell");
        terminal.output.lock().unwrap().append(b"separate-shell");
        let terminals = AiTerminals::default();
        terminals.entries.lock().unwrap().terminals.push(terminal.clone());
        assert_eq!(terminal.read_for_agent(None, 1024).unwrap().data, b"original-shell");
        assert!(terminals.owns("terminal", "session"));
        assert!(!terminals.owns("another-terminal", "session"));
        assert!(terminals.release("terminal", Some("another-agent")).is_err());
        terminals.release("terminal", Some("agent")).unwrap();
        assert!(!terminals.owns("terminal", "session"));
        assert!(terminal.read_for_agent(None, 1024).is_err());
        assert!(!terminal.source.as_ref().unwrap().cancel.is_cancelled());
        // The desktop may still display already retained scrollback after handoff.
        assert_eq!(terminal.read(None, 1024).unwrap().data, b"original-shell");
    }

    #[test]
    fn completion_framing_handles_all_packet_boundaries() {
        let wire = b"output\x1b]633;OrbitShell;nonce;17\x07prompt";
        for split in 0..wire.len() {
            let mut parser = Marker { prefix: b"\x1b]633;OrbitShell;nonce;".to_vec(), pending: Vec::new() };
            let (mut first, code_a) = parser.push(&wire[..split]);
            let (last, code_b) = parser.push(&wire[split..]); first.extend(last);
            assert_eq!(first, b"outputprompt");
            assert_eq!(code_a.or(code_b), Some(17));
        }
    }
}
