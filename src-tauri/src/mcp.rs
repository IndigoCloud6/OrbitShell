use crate::{credentials, ssh::{ConnectRequest, SessionEvents}, storage::ConnectionProfile, AppState, start_session};
use axum::{Router, extract::State, http::{Request, StatusCode}, middleware::{self, Next}, response::Response};
use rmcp::{
    ErrorData as McpError, ServerHandler, RoleServer,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig, InitializeRequestParams, InitializeResult},
    schemars, tool, tool_handler, tool_router, service::{RequestContext, NotificationContext},
    transport::{StreamableHttpServerConfig, streamable_http_server::{session::local::LocalSessionManager, tower::StreamableHttpService}},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex, atomic::Ordering};
use subtle::ConstantTimeEq;
use tauri::{AppHandle, Emitter, Manager};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

const PORT: u16 = 43123;
const CONTROL_PROTOCOL: &str = "orbitshell-terminal-control/3";
// Serialize MCP connection requests so retries can reuse the session announced
// by the preceding call instead of racing to start another SSH connection.
static CONNECTION_REQUESTS: Mutex<()> = Mutex::new(());

fn registered_tool_names() -> Vec<String> {
    OrbitTools::tool_router().list_all().into_iter().map(|tool| tool.name.to_string()).collect()
}

pub struct RunningMcp {
    cancel: CancellationToken,
    task: tauri::async_runtime::JoinHandle<()>,
}
impl RunningMcp {
    pub fn stop(self) { self.cancel.cancel(); self.task.abort(); }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpStatus { enabled: bool, url: Option<String>, token: String, tools: Vec<String>, protocol_version: String, auto_start: bool, startup_error: Option<String> }

#[tauri::command]
pub fn mcp_status(app: AppHandle) -> Result<McpStatus, String> {
    let state = app.state::<AppState>();
    let enabled = state.mcp_enabled.load(Ordering::Acquire);
    let store = state.storage.lock().map_err(|_| "配置暂不可用")?;
    let startup_error = state.mcp_startup_error.lock().map_err(|_| "MCP 状态不可用")?.clone();
    Ok(McpStatus { enabled, url: enabled.then(|| format!("http://127.0.0.1:{PORT}/mcp")), token: store.mcp_token.clone(), auto_start: store.mcp_auto_start,
        startup_error, tools: registered_tool_names(), protocol_version: CONTROL_PROTOCOL.into() })
}

pub fn notify_status(app: &AppHandle) { let _ = app.emit_to("main", "orbitshell:mcp-status-changed", ()); }

#[tauri::command]
pub fn mcp_set_auto_start(auto_start: bool, app: AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let mut store = state.storage.lock().map_err(|_| "配置暂不可用")?;
    let mut next = store.clone(); next.mcp_auto_start = auto_start;
    state.storage.persist(&next).map_err(|_| "MCP 启动方式保存失败")?;
    *store = next; drop(store); notify_status(&app); Ok(())
}

#[tauri::command]
pub async fn mcp_set_enabled(enabled: bool, app: AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let mut runtime = state.mcp_runtime.lock().await;
    if !enabled {
        state.mcp_epoch.fetch_add(1, Ordering::AcqRel);
        state.mcp_enabled.store(false, Ordering::Release);
        state.transfers.stop_mcp();
        state.ai.stop(&app);
        if let Some(server) = runtime.take() { server.stop(); }
        if let Ok(mut error) = state.mcp_startup_error.lock() { *error = None; }
        notify_status(&app);
        return Ok(());
    }
    if runtime.is_some() && state.mcp_enabled.load(Ordering::Acquire) { return Ok(()); }
    if let Some(server) = runtime.take() { server.stop(); }
    let epoch = state.mcp_epoch.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
    if state.desktop.exiting.load(Ordering::Acquire) { return Err("程序正在退出，未启动 MCP".into()); }
    let tools = registered_tool_names();
    if ["take_over_session", "release_session", "upload_file", "download_file", "get_file_transfer"].iter().any(|required| !tools.iter().any(|name| name.as_str() == *required)) {
        return Err("MCP 工具路由不完整，缺少终端控制或文件传输工具，服务未启动。".into());
    }
    let token = state.storage.lock().map_err(|_| "配置暂不可用")?.mcp_token.clone();
    if token.is_empty() { return Err("MCP 访问令牌尚未初始化".into()); }
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", PORT)).await.map_err(|_| format!("无法使用本机 {PORT} 端口，请先关闭占用该端口的程序"))?;
    if state.desktop.exiting.load(Ordering::Acquire) { return Err("程序正在退出，未启动 MCP".into()); }
    let cancel = CancellationToken::new();
    let mut config = StreamableHttpServerConfig::default();
    config.cancellation_token = cancel.clone();
    config.max_request_body_bytes = 1024 * 1024;
    config.allowed_hosts = vec![format!("127.0.0.1:{PORT}"), format!("localhost:{PORT}")];
    config.allowed_origins = vec![format!("http://127.0.0.1:{PORT}"), format!("http://localhost:{PORT}")];
    let app_for_tools = app.clone();
    let service = StreamableHttpService::new(move || Ok(OrbitTools::new(app_for_tools.clone())), LocalSessionManager::default().into(), config);
    let auth = Authorization { app: app.clone(), token };
    let router = Router::new().nest_service("/mcp", service).layer(middleware::from_fn_with_state(auth, authorize));
    state.mcp_enabled.store(true, Ordering::Release);
    if let Ok(mut error) = state.mcp_startup_error.lock() { *error = None; }
    let shutdown = cancel.clone();
    let app_for_exit = app.clone();
    let task = tauri::async_runtime::spawn(async move {
        let _ = axum::serve(listener, router).with_graceful_shutdown(shutdown.cancelled_owned()).await;
        let state = app_for_exit.state::<AppState>();
        let mut runtime = state.mcp_runtime.lock().await;
        if state.mcp_epoch.load(Ordering::Acquire) == epoch {
            runtime.take();
            state.mcp_enabled.store(false, Ordering::Release);
            state.transfers.stop_mcp();
            state.ai.stop(&app_for_exit);
            notify_status(&app_for_exit);
        }
    });
    *runtime = Some(RunningMcp { cancel, task });
    notify_status(&app);
    Ok(())
}

#[derive(Clone)]
struct Authorization { app: AppHandle, token: String }

async fn authorize(State(auth): State<Authorization>, request: Request<axum::body::Body>, next: Next) -> Result<Response, StatusCode> {
    if !auth.app.state::<AppState>().mcp_enabled.load(Ordering::Acquire) { return Err(StatusCode::SERVICE_UNAVAILABLE); }
    let supplied = request.headers().get("authorization").and_then(|value| value.to_str().ok()).and_then(|value| value.strip_prefix("Bearer "));
    match supplied {
        Some(token) if token.len() == auth.token.len() && bool::from(token.as_bytes().ct_eq(auth.token.as_bytes())) => Ok(next.run(request).await),
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}

#[derive(Clone)]
struct OrbitTools { app: AppHandle, tool_router: ToolRouter<Self>, agent: Arc<AgentLease> }

// The handler is cloned for requests; the last lease ends the logical MCP client session.
struct AgentLease { app: AppHandle, id: String }
impl Drop for AgentLease {
    fn drop(&mut self) { self.app.state::<AppState>().ai.disconnect_agent(&self.app, &self.id); }
}

#[derive(Deserialize, schemars::JsonSchema)]
struct OpenArgs {
    /// Saved connection identifier from list_connections.
    connection_id: String,
    /// Optional one-time password or key passphrase. Omit to reuse an existing session or use credentials saved locally in OrbitShell; the desktop requests login only when no saved credential is available. Saved secrets are never returned to the Agent.
    credential: Option<String>,
}
#[derive(Deserialize, schemars::JsonSchema)]
struct SessionArgs { session_id: String }
#[derive(Deserialize, schemars::JsonSchema)]
struct UploadArgs {
    /// Connected sessionId from list_sessions. Uses SFTP on this SSH connection, not the terminal shell's current directory.
    session_id: String,
    /// Absolute source FILE path on the computer running OrbitShell, NOT the Agent's container/remote host. No file picker is shown. Local disks only on Windows.
    local_path: String,
    /// Existing absolute SFTP directory. Upload keeps the local filename and never overwrites an existing remote file.
    remote_directory: String,
}
#[derive(Deserialize, schemars::JsonSchema)]
struct DownloadArgs {
    session_id: String,
    /// Absolute remote FILE path in this connection's SFTP namespace.
    remote_path: String,
    /// Absolute destination FILE path on the computer running OrbitShell. Parent must exist. Existing files/symlinks are never replaced. No file picker is shown.
    local_path: String,
}
#[derive(Deserialize, schemars::JsonSchema)]
struct TransferArgs {
    /// transferId returned by upload_file or download_file, scoped to this MCP client.
    transfer_id: String,
}
#[derive(Deserialize, schemars::JsonSchema)]
struct TakeoverArgs {
    /// Existing session ID; omit to take over the currently selected human terminal tab.
    session_id: Option<String>,
}
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
enum TerminalMode { Dedicated, Takeover }
impl TerminalMode { fn as_str(&self) -> &'static str { match self { Self::Dedicated => "dedicated", Self::Takeover => "takeover" } } }

#[derive(Clone, Copy, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
enum InputAction { Input, Takeover, Release }

#[derive(Deserialize, schemars::JsonSchema)]
struct ExecArgs {
    session_id: String,
    /// Pin the terminalId returned by take_over_session; prevents routing to another PTY.
    terminal_id: Option<String>,
    /// takeover requires prior explicit ownership; dedicated intentionally uses a separate shell.
    mode: Option<TerminalMode>,
    command: String,
    /// Wait up to 1..300 seconds (default 30). A timeout leaves the command running; poll read_output.
    timeout_seconds: Option<u64>,
}
#[derive(Deserialize, schemars::JsonSchema)]
struct InputArgs {
    session_id: String,
    /// Default input. For clients with only the original seven tools: takeover/release use data="" and do NOT send terminal input.
    action: Option<InputAction>,
    /// Pin the terminalId returned by take_over_session.
    terminal_id: Option<String>,
    mode: Option<TerminalMode>,
    /// Text is buffered locally until newline, then checked by AI safety rules. Ctrl+C must be sent alone. No cursor editing or history recall.
    data: String,
    append_newline: Option<bool>,
    /// True only for a shell command that may be displayed. Never set for passwords, tokens or prompt answers.
    is_command: Option<bool>,
}
impl Drop for InputArgs { fn drop(&mut self) { self.data.zeroize(); } }

impl InputArgs {
    fn validated_action(&self) -> Result<InputAction, &'static str> {
        let action = self.action.unwrap_or(InputAction::Input);
        if matches!(action, InputAction::Takeover | InputAction::Release) {
            if !self.data.is_empty() || self.append_newline.unwrap_or(false) || self.is_command.unwrap_or(false) || matches!(self.mode.as_ref(), Some(TerminalMode::Dedicated)) {
                return Err("接管/交回控制操作必须 data=空字符串，不能携带按键、回车、命令标记或 dedicated 模式；未发送输入。");
            }
            if matches!(action, InputAction::Takeover) && self.terminal_id.is_some() { return Err("建立接管时不要携带旧 terminal_id，请使用新回执的绑定。"); }
        } else if self.data.is_empty() && !self.append_newline.unwrap_or(false) {
            return Err("空输入未发送，也未创建 PTY。需要接管时请明确传递 action=takeover；若宿主过滤该参数，请刷新工具清单或使用桌面接管入口。");
        }
        if self.data.len() > 64 * 1024 || self.data.contains('\0') { return Err("Input exceeds 65536 bytes or contains NUL"); }
        Ok(action)
    }
}

fn text_output(mut output: crate::ai_terminals::TerminalOutput, complete: bool) -> (String, u64, bool) {
    // JSON strings cannot carry an unfinished UTF-8 sequence. Leave that suffix
    // behind the cursor so the next read decodes it together with its next packet.
    if !complete {
        let mut offset = 0;
        while let Err(error) = std::str::from_utf8(&output.data[offset..]) {
            offset += error.valid_up_to();
            if let Some(length) = error.error_len() { offset += length; }
            else { output.cursor -= (output.data.len() - offset) as u64; output.data.truncate(offset); break; }
        }
    }
    (String::from_utf8_lossy(&output.data).into_owned(), output.cursor, output.truncated)
}

#[derive(Deserialize, schemars::JsonSchema)]
struct ReadArgs {
    session_id: String,
    /// Pin the terminalId returned by take_over_session. Reset cursor after switching terminals.
    terminal_id: Option<String>,
    mode: Option<TerminalMode>,
    /// Byte cursor from the previous read_output response.
    cursor: Option<u64>,
    /// Maximum bytes, at most 65536.
    max_bytes: Option<usize>,
}

fn failure(error: impl std::fmt::Display) -> McpError { McpError::internal_error(error.to_string(), None) }
fn result(value: Value) -> Result<CallToolResult, McpError> {
    Ok(CallToolResult::success(vec![ContentBlock::text(value.to_string())]))
}

#[tool_router]
impl OrbitTools {
    fn new(app: AppHandle) -> Self {
        let agent = Arc::new(AgentLease { app: app.clone(), id: Uuid::new_v4().to_string() });
        Self { app, tool_router: Self::tool_router(), agent }
    }

    fn ensure_enabled(&self) -> Result<(), McpError> {
        if !self.app.state::<AppState>().mcp_enabled.load(Ordering::Acquire) { return Err(failure("MCP is disabled")); }
        Ok(())
    }

    fn request_credentials(&self, profile: &ConnectionProfile, reason: &str, message: &str) -> Result<CallToolResult, McpError> {
        self.app.emit_to("main", "orbitshell:request-credentials", profile).map_err(failure)?;
        result(json!({"state": "awaiting_user_authentication", "connectionId": profile.id,
            "reason": reason, "message": message,
            "nextStep": "请在 OrbitShell 桌面完成登录或处理凭据问题，然后轮询 list_sessions。不要要求用户通过聊天发送已保存的密码或私钥口令。"}))
    }

    fn takeover_receipt(&self, session_id: Option<&str>) -> Result<Value, McpError> {
        let state = self.app.state::<AppState>();
        let session_id = session_id.map(str::to_owned).or_else(|| state.ai.active_session()).ok_or_else(|| failure("当前未选中人工终端，请明确指定 list_sessions 中的 session_id"))?;
        let terminal = state.ai.take_over(&self.app, &self.agent.id, &session_id).map_err(failure)?;
        let info = terminal.info().map_err(failure)?;
        Ok(json!({"operation": "takeover", "sent": false, "protocolVersion": CONTROL_PROTOCOL,
            "terminalId": info.id, "agentId": info.agent_id, "agentName": info.agent_name, "sessionId": session_id,
            "mode": "takeover", "sharesOriginalTerminal": true, "controlActive": state.ai.owns(&info.id, &session_id),
            "nextRead": {"tool": "read_output", "arguments": {"session_id": session_id, "terminal_id": info.id, "mode": "takeover", "cursor": null}},
            "note": "已绑定原标签的同一 PTY 输入队列和输出缓冲。未新建 SSH/PTY/Shell，未发送字符。后续固定 terminal_id 和 mode=takeover；首次读取 cursor 留空。"}))
    }

    fn release_receipt(&self, session_id: &str, terminal_id: Option<&str>) -> Result<Value, McpError> {
        let state = self.app.state::<AppState>();
        let terminal = match terminal_id { Some(id) => state.ai.terminal(id), None => state.ai.existing(&self.agent.id, session_id) }.map_err(failure)?;
        let info = terminal.info().map_err(failure)?;
        if info.session_id != session_id || info.mode != "takeover" { return Err(failure("会话或接管绑定不匹配，未交回其他终端。")); }
        state.ai.release(&info.id, Some(&self.agent.id)).map_err(failure)?;
        Ok(json!({"operation": "release", "sent": false, "released": true, "controlActive": false, "protocolVersion": CONTROL_PROTOCOL,
            "sessionId": session_id, "terminalId": info.id, "note": "已交回人工，未发送字符；已发出的远端命令可能仍在运行。"}))
    }

    #[tool(description = "List saved SSH connection profiles. Returns identifiers and destinations, never passwords or private key contents.")]
    fn list_connections(&self) -> Result<CallToolResult, McpError> {
        self.ensure_enabled()?;
        let state = self.app.state::<AppState>();
        let store = state.storage.lock().map_err(failure)?;
        let profiles: Vec<_> = store.connections.iter().map(|profile| json!({
            "id": profile.id, "name": profile.name, "group": profile.group,
            "hostname": profile.hostname, "port": profile.port, "authMethod": profile.auth_method,
        })).collect();
        result(json!({"connections": profiles}))
    }

    #[tool(description = "List live OrbitShell SSH sessions, ownership and this MCP client agentId. activeSessionId=null does not mean sessions are unavailable: explicitly use a connected sessionId with take_over_session. No UI toggle is required for MCP takeover. Use a connected session_id to execute commands or interact with a bastion terminal.")]
    fn list_sessions(&self) -> Result<CallToolResult, McpError> {
        self.ensure_enabled()?;
        let state = self.app.state::<AppState>();
        let sessions = state.sessions.lock().map_err(failure)?;
        let values = sessions.iter().map(|(id, session)| Ok(json!({
            "sessionId": id, "connectionId": session.profile.id, "name": session.profile.name,
            "hostname": session.profile.hostname, "port": session.profile.port,
            "connected": session.connected.load(Ordering::Acquire),
            "control": state.ai.session_control(id, &self.agent.id).map_err(failure)?,
        }))).collect::<Result<Vec<_>, McpError>>()?;
        result(json!({"sessions": values, "agentId": self.agent.id, "activeSessionId": state.ai.active_session(),
            "fileTransfers": {"tools": ["upload_file", "download_file", "get_file_transfer"], "localPaths": "运行 OrbitShell 的电脑上的绝对文件路径；不是 Agent 容器或远端主机的路径", "filePicker": false, "requiresSftp": true, "overwritesExistingFiles": false, "note": "返回任务编号只表示已受理，查询 get_file_transfer，status=done 才表示完成。不会创建或接管终端。"},
            "commandSafety": {"enabled": true, "version": crate::command_guard::POLICY_VERSION, "scope": "所有 AI 独立终端和原标签接管输入；人工输入不受影响", "literalInput": "文本先在本机缓存，换行时检查并发送；Ctrl+C 单独即时发送；不支持 Tab、光标编辑或历史召回", "onBlocked": "停止，不改写或编码重试；需执行时交回人工；这不是远端权限沙箱"},
            "activeSessionNote": "activeSessionId=null 只表示当前未选中人工终端；会话仍可通过明确的 sessionId 接管，不需要 UI 先切换 AI 模式。",
            "protocolVersion": CONTROL_PROTOCOL, "serverToolNames": self.tool_router.list_all().into_iter().map(|tool| tool.name.to_string()).collect::<Vec<_>>(),
            "takeoverInstructions": "优先调用 take_over_session(session_id)。若宿主只提供旧七工具，改用 send_input(session_id, data=空字符串, action=takeover)，只绑定不发送字符。保存回执 terminalId，后续固定 terminal_id 与 mode=takeover。第一次读取 cursor 留空，独立 PTY 不等于接管。",
            "compatibility": {"takeover": {"tool": "send_input", "arguments": {"session_id": "<选择的完整 sessionId>", "data": "", "action": "takeover"}}, "release": {"tool": "send_input", "arguments": {"session_id": "<同一 sessionId>", "data": "", "action": "release"}},
                "note": "serverToolNames 是服务端真实路由，不等于宿主已绑定的函数。宿主函数缺失不代表服务端拒绝注册；若 action 参数也被过滤，使用桌面接管入口或刷新宿主工具清单，不新开 PTY 替代。"}}))
    }

    #[tool(description = "Open a saved SSH connection in the visible OrbitShell workspace. Omit credential to first reuse a matching connected/in-progress session, or start SSH with credentials already saved in the local OS vault; saved passwords/passphrases are never returned to the Agent and require no extra Connect click. Only missing/unavailable saved credentials request desktop login. Do not ask users to send saved secrets in chat. Explicit credential supplies a one-time secret for a new connection. Unknown host fingerprints still require desktop confirmation. Poll list_sessions using the returned sessionId until connected; do not repeat open_connection while it is connecting. Opening/reusing SSH does not take over the original terminal: use take_over_session separately when authorized.")]
    fn open_connection(&self, Parameters(args): Parameters<OpenArgs>) -> Result<CallToolResult, McpError> {
        let supplied_credential = args.credential.is_some();
        let mut credential = Zeroizing::new(args.credential.unwrap_or_default());
        let _request = CONNECTION_REQUESTS.lock().map_err(|_| failure("连接请求状态不可用"))?;
        self.ensure_enabled()?;
        let profile = {
            let state = self.app.state::<AppState>();
            let store = state.storage.lock().map_err(failure)?;
            store.connections.iter().find(|profile| profile.id == args.connection_id).cloned().ok_or_else(|| failure("Connection not found"))?
        };
        if !supplied_credential {
            let state = self.app.state::<AppState>();
            let active_id = state.ai.active_session();
            let existing = {
                let sessions = state.sessions.lock().map_err(failure)?;
                sessions.iter().filter(|(_, session)| !session.is_temporary &&
                    credentials::same_identity(&session.profile, &profile) &&
                    !session.cancel.is_cancelled() && !session.commands.is_closed())
                    .max_by_key(|(id, session)| (session.connected.load(Ordering::Acquire), active_id.as_deref() == Some(id.as_str())))
                    .map(|(id, session)| (id.clone(), session.connected.load(Ordering::Acquire)))
            };
            if let Some((id, connected)) = existing {
                return result(json!({"state": if connected { "connected" } else { "connecting" },
                    "connectionId": profile.id, "sessionId": id, "reused": true,
                    "note": "复用已有 SSH 会话，未接管原终端。正在连接时继续轮询 list_sessions；若等待首次主机指纹确认，请在桌面处理。"}));
            }
            // Probe only availability here. start_session loads the secret again
            // against the current profile identity; the probe's zeroizing value
            // is dropped locally and never placed in a tool response.
            match credentials::load(&profile) {
                Ok(Some(_)) => {},
                Ok(None) => return self.request_credentials(&profile, "credentials_missing", "本机没有此连接可用的已保存凭据，需要在桌面完成登录。"),
                Err(message) => return self.request_credentials(&profile, "credentials_unavailable", &message),
            }
        }
        let id = Uuid::new_v4().to_string();
        let request = ConnectRequest { session_id: id.clone(), profile_id: profile.id.clone(), profile: None,
            credential: std::mem::take(&mut *credential), use_saved_credential: !supplied_credential,
            remember_credential: false, columns: 120, rows: 36 };
        start_session(self.app.clone(), request, SessionEvents::broadcast(self.app.clone(), id.clone())).map_err(failure)?;
        result(json!({"state": "connecting", "connectionId": profile.id, "sessionId": id, "reused": false,
            "authentication": if supplied_credential { "provided_credential" } else { "saved_credential" },
            "note": "SSH 连接已发起，不代表认证完成。继续轮询 list_sessions，首次主机指纹仍需在桌面确认。"}))
    }

    #[tool(description = "Default is a separate AI shell, NOT a takeover of an existing tab. To operate the user's existing shell, first take_over_session OR send_input(action=takeover, empty data) if the host has only the original tools and pass its terminal_id plus mode=takeover here. Only a response with sharesOriginalTerminal=true confirms the original PTY. UI handoff also establishes ownership. Reuses the shell per Agent/session: cd and export persist. Streams command and PTY output into an AI workspace tab. After explicit take_over_session, sends the raw command to that existing PTY instead, returns sent=true, running=null and exitCode=null: inspect read_output before sending further input. stdout contains combined PTY stdout/stderr. Wait timeout does NOT kill the command: running=true means poll read_output; do not execute it again. Concurrent execute_command calls on the same dedicated AI terminal are rejected. Built-in AI danger rules apply before sending in BOTH modes; blocked commands must not be rewritten or encoded to retry. Use send_input for line-based interactive answers or Ctrl+C. Dedicated mode requires an additional PTY shell; takeover mode reuses the original PTY; never silently falls back to a human terminal; that requires explicit take_over_session.")]
    async fn execute_command(&self, Parameters(args): Parameters<ExecArgs>) -> Result<CallToolResult, McpError> {
        self.ensure_enabled()?;
        if args.command.is_empty() || args.command.len() > 64 * 1024 || args.command.contains('\0') {
            return Err(failure("Command must contain 1 to 65536 bytes, without NUL"));
        }
        crate::command_guard::check_command(&args.command).map_err(failure)?;
        let terminal = self.app.state::<AppState>().ai.resolve(&self.app, &self.agent.id, &args.session_id, args.terminal_id.as_deref(), args.mode.as_ref().map(TerminalMode::as_str), true).map_err(failure)?;
        let response = terminal.execute(args.command.replace("\r\n", "\n"), args.timeout_seconds.unwrap_or(30).clamp(1, 300)).await.map_err(failure)?;
        let info = terminal.info().map_err(failure)?;
        let mut response = serde_json::to_value(response).map_err(failure)?;
        response["mode"] = json!(info.mode);
        response["sharesOriginalTerminal"] = json!(info.mode == "takeover");
        response["sessionId"] = json!(info.session_id);
        response["agentId"] = json!(info.agent_id);
        response["completionKnown"] = json!(info.mode != "takeover" && !response["running"].as_bool().unwrap_or(true) && !response["exitCode"].is_null());
        if info.mode == "takeover" {
            response["sent"] = json!(true);
            response["running"] = Value::Null;
            response["elapsedMs"] = Value::Null;
            response["controlActive"] = json!(self.app.state::<AppState>().ai.owns(&info.id, &info.session_id));
        }
        result(response)
    }

    #[tool(description = "Send input or explicitly control an existing terminal. If take_over_session is not exposed by the host, use this SAME tool with action=takeover, empty data and session_id; it binds the original PTY and sends NO input. action=release with empty data hands it back. Default action=input keeps normal behavior. For an existing user tab, first obtain the takeover receipt, then pin terminal_id and mode=takeover. sharesOriginalTerminal=false means a separate PTY, never a takeover. Text is buffered locally until newline (append_newline=true submits); inspect sent and buffered in the receipt and never resend buffered fragments. Safety checks apply regardless of is_command. Tab, arrows, history recall and other editing controls are rejected. Use line-based interactive prompts or a separate Ctrl+C (U+0003) to interrupt a running command. Input is not logged as a command by default. Set is_command=true only for a shell command safe to display and save in local command history, never for a password/token/prompt answer. No exit code is inferred from literal input; use execute_command in dedicated mode for POSIX command completion tracking.")]
    async fn send_input(&self, Parameters(mut args): Parameters<InputArgs>) -> Result<CallToolResult, McpError> {
        self.ensure_enabled()?;
        match args.validated_action().map_err(failure)? {
            InputAction::Takeover | InputAction::Release => {
                return match args.action.unwrap_or(InputAction::Input) {
                    InputAction::Takeover => {
                        result(self.takeover_receipt(Some(&args.session_id))?)
                    },
                    InputAction::Release => result(self.release_receipt(&args.session_id, args.terminal_id.as_deref())?),
                    InputAction::Input => Err(failure("未指定接管或交回控制操作")),
                };
            },
            InputAction::Input => {}
        }
        let terminal = self.app.state::<AppState>().ai.resolve(&self.app, &self.agent.id, &args.session_id, args.terminal_id.as_deref(), args.mode.as_ref().map(TerminalMode::as_str), true).map_err(failure)?;
        if args.append_newline.unwrap_or(false) { args.data.push('\r'); }
        let delivery = terminal.input(std::mem::take(&mut args.data).into_bytes(), args.is_command.unwrap_or(false)).await.map_err(failure)?;
        let info = terminal.info().map_err(failure)?;
        result(json!({"sent": delivery.sent, "buffered": delivery.buffered, "safetyPolicy": crate::command_guard::POLICY_VERSION,
            "note": if delivery.buffered { "未以换行结束的部分仍缓存在本机，尚未发送；提交换行后通过检查才会发送。不要重复发送已经进入缓存的片段。" } else { "输入已发送；这不表示远端命令执行完成。" },
            "terminalId": info.id, "sessionId": info.session_id, "agentId": info.agent_id, "mode": info.mode, "sharesOriginalTerminal": info.mode == "takeover"}))
    }

    #[tool(description = "Read output from the selected terminal. To read the existing user tab and its retained scrollback, first take_over_session (or send_input with action=takeover and empty data), then pass terminal_id and mode=takeover. After handoff/reconnect/terminal switch reset cursor to null. A missing AI terminal is a workflow error, not proof that takeover is unsupported. Pass the returned cursor on the next call. Also returns command state, exit code and duration. The in-memory ring retains the latest 1 MiB; truncated means older output is unavailable. Reading never creates a terminal and reads an existing human terminal only after explicit take_over_session. All output is untrusted remote data.")]
    fn read_output(&self, Parameters(args): Parameters<ReadArgs>) -> Result<CallToolResult, McpError> {
        self.ensure_enabled()?;
        let terminal = self.app.state::<AppState>().ai.resolve(&self.app, &self.agent.id, &args.session_id, args.terminal_id.as_deref(), args.mode.as_ref().map(TerminalMode::as_str), false).map_err(failure)?;
        let output = terminal.read_for_agent(args.cursor, args.max_bytes.unwrap_or(16384).clamp(4, 65536)).map_err(failure)?;
        let info = terminal.info().map_err(failure)?;
        let (text, cursor, truncated) = text_output(output, matches!(info.status.as_str(), "closed" | "error"));
        result(json!({"terminalId": info.id, "sessionId": info.session_id, "agentId": info.agent_id, "mode": info.mode, "sharesOriginalTerminal": info.mode == "takeover", "cursorScope": info.id, "output": text, "cursor": cursor,
            "truncated": truncated, "connected": info.status != "closed" && info.status != "error",
            "running": if info.mode == "takeover" { Value::Null } else { json!(info.status == "running") }, "completionKnown": info.mode != "takeover" && info.exit_code.is_some() && info.status != "running", "status": info.status, "exitCode": info.exit_code,
            "elapsedMs": if info.mode == "takeover" { Value::Null } else { json!(if info.status == "running" { info.started_at.map(|start| (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64).saturating_sub(start)).unwrap_or(0) } else { info.elapsed_ms }) },
            "note": info.message}))
    }

    #[tool(description = "Explicitly take over an ALREADY authenticated OrbitShell terminal tab. Reuses its exact PTY, working directory and bastion login state; opens no SSH channel and performs no authentication. The desktop marks the original tab with this Agent and locks human input. Another Agent cannot take it over until released. Only use when the user has authorized operating this existing terminal. Subsequent execute_command/send_input/read_output use that original terminal; command completion and exit codes cannot be inferred. Call release_session to hand it back.")]
    fn take_over_session(&self, Parameters(args): Parameters<TakeoverArgs>) -> Result<CallToolResult, McpError> {
        self.ensure_enabled()?;
        result(self.takeover_receipt(args.session_id.as_deref())?)
    }

    #[tool(description = "Release this Agent's takeover of an existing terminal tab and unlock human input. This does not stop the remote process or disconnect SSH. Release or MCP client disconnection removes ownership; queued Agent input is rejected after release.")]
    fn release_session(&self, Parameters(args): Parameters<SessionArgs>) -> Result<CallToolResult, McpError> {
        self.ensure_enabled()?;
        result(self.release_receipt(&args.session_id, None)?)
    }

    #[tool(description = "Upload one user-authorized local file through SFTP on an authenticated OrbitShell SSH connection. Supply an absolute local_path on the computer RUNNING ORBITSHELL, not an Agent container path, and an existing absolute remote_directory. No file picker is shown; the Agent chooses paths within the user's authorization. Does not create/take over a terminal, inject commands, or automatically execute the uploaded file. Never overwrites an existing remote file. Returns immediately with transferId; poll get_file_transfer until status=done/error/cancelled. Do not resubmit a running transfer when waiting times out. Progress and human/AI transfer history appear in the desktop transfer queue. A bastion shell prompt does not prove its SSH connection supports target SFTP.")]
    fn upload_file(&self, Parameters(args): Parameters<UploadArgs>) -> Result<CallToolResult, McpError> {
        self.ensure_enabled()?;
        let transfer = self.app.state::<AppState>().transfers.start(self.app.clone(), &self.agent.id, &args.session_id,
            crate::file_transfers::Direction::Upload, args.local_path, args.remote_directory).map_err(failure)?;
        result(json!({"transferId": transfer.id, "status": transfer.status, "completed": false,
            "note": "任务已受理，尚未完成；用 get_file_transfer 查询真实结果，不要重复发起同一传输。", "transfer": transfer}))
    }

    #[tool(description = "Download one user-authorized remote file through SFTP. Supply session_id, absolute remote_path, and an absolute local_path FILE destination on the computer RUNNING ORBITSHELL. No file picker is shown. Parent directory must already exist; local files and symlinks are never overwritten. Writes a temporary file and commits it only after transfer, flush and sync finish. Returns transferId immediately; poll get_file_transfer for status=done/error/cancelled. A created task or 100% bytes does not by itself prove a successful commit. Does not return file contents, execute the file, create a terminal or use the shell's pwd.")]
    fn download_file(&self, Parameters(args): Parameters<DownloadArgs>) -> Result<CallToolResult, McpError> {
        self.ensure_enabled()?;
        let transfer = self.app.state::<AppState>().transfers.start(self.app.clone(), &self.agent.id, &args.session_id,
            crate::file_transfers::Direction::Download, args.local_path, args.remote_path).map_err(failure)?;
        result(json!({"transferId": transfer.id, "status": transfer.status, "completed": false,
            "note": "任务已受理，尚未完成；用 get_file_transfer 查询真实结果，不要重复发起同一传输。", "transfer": transfer}))
    }

    #[tool(description = "Read this MCP client's file-transfer task by transfer_id. Returns session, paths, direction, bytes, total (null if unknown), speed, timestamps and status. running means continue polling without resubmitting; done confirms success; error/cancelled are unsuccessful terminal states. Does not return file contents or credentials. Desktop history persists for normal connections; temporary/bastion sessions are memory-only. Disabling MCP or disconnecting SSH stops active AI transfers. File tasks may continue after the initiating MCP client disconnects; a newly initialized client has a new Agent identity and cannot query the old client's task; inspect it in the desktop queue instead.")]
    fn get_file_transfer(&self, Parameters(args): Parameters<TransferArgs>) -> Result<CallToolResult, McpError> {
        self.ensure_enabled()?;
        let transfer = self.app.state::<AppState>().transfers.get(&self.agent.id, &args.transfer_id).map_err(failure)?;
        let finished = transfer.status != "running";
        let completed = transfer.status == "done";
        result(json!({"transferId": transfer.id, "finished": finished, "completed": completed, "transfer": transfer}))
    }

    #[tool(description = "Disconnect a live SSH session. Stops the terminal and file transfers using that session.")]
    fn disconnect(&self, Parameters(args): Parameters<SessionArgs>) -> Result<CallToolResult, McpError> {
        self.ensure_enabled()?;
        let live = self.app.state::<AppState>().session(&args.session_id).map_err(failure)?;
        live.cancel.cancel();
        result(json!({"disconnected": true}))
    }
}

#[tool_handler]
impl ServerHandler for OrbitTools {
    async fn initialize(&self, request: InitializeRequestParams, context: RequestContext<RoleServer>) -> Result<InitializeResult, McpError> {
        let response = self.negotiate_initialize(&request)?;
        self.ensure_enabled()?;
        self.app.state::<AppState>().ai.connect_agent(&self.app, &self.agent.id, &request.client_info.name, &request.client_info.version).map_err(failure)?;
        context.peer.set_peer_info(request);
        Ok(response)
    }
    async fn on_initialized(&self, context: NotificationContext<RoleServer>) {
        // Ask hosts that support MCP listChanged to refresh their bound tool functions.
        let _ = context.peer.notify_tool_list_changed().await;
    }
    fn get_info(&self) -> ServerConfig {
        let mut info = Implementation::from_build_env();
        info.version = format!("{}+terminal-control.3.file-transfer.1", env!("CARGO_PKG_VERSION"));
        ServerConfig::new(ServerCapabilities::builder().enable_tools().enable_tool_list_changed().build())
            .with_server_info(info)
            .with_instructions(include_str!("../../docs/mcp-agent-rules.md"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actual_router_publishes_control_tools_and_compatibility_schema() {
        let tools = OrbitTools::tool_router().list_all();
        assert_eq!(tools.len(), 12);
        for name in ["take_over_session", "release_session", "execute_command", "send_input", "read_output", "upload_file", "download_file", "get_file_transfer"] {
            assert!(tools.iter().any(|tool| tool.name == name), "missing {name}");
        }
        let input = tools.iter().find(|tool| tool.name == "send_input").unwrap();
        let schema = serde_json::to_value(&input.input_schema).unwrap();
        assert!(schema["properties"]["action"].is_object());
        assert!(schema["properties"]["terminal_id"].is_object());
        assert!(schema.to_string().contains("takeover"));
    }

    fn args(value: Value) -> InputArgs { serde_json::from_value(value).unwrap() }

    #[test]
    fn compat_takeover_is_explicit_and_cannot_send_keystrokes() {
        let control = json!({"session_id":"session", "data":"", "action":"takeover"});
        assert!(matches!(args(control.clone()).validated_action(), Ok(InputAction::Takeover)));
        for (field, value) in [("data", json!("echo probe")), ("append_newline", json!(true)), ("is_command", json!(true)), ("mode", json!("dedicated")), ("terminal_id", json!("old-binding"))] {
            let mut invalid = control.clone(); invalid[field] = value;
            assert!(args(invalid).validated_action().is_err(), "accepted {field}");
        }
        assert!(args(json!({"session_id":"session", "data":""})).validated_action().is_err());
        assert!(matches!(args(json!({"session_id":"session", "data":"", "append_newline":true})).validated_action(), Ok(InputAction::Input)));
        assert!(serde_json::from_value::<InputArgs>(json!({"session_id":"session", "data":"", "action":"typo"})).is_err());
    }

    #[test]
    fn paged_output_preserves_utf8_across_every_byte_boundary() {
        let bytes = "中文输出🙂done".as_bytes();
        for limit in 4..bytes.len() {
            let first = crate::ai_terminals::TerminalOutput { data: bytes[..limit].to_vec(), cursor: limit as u64, truncated: false };
            let (a, cursor, _) = text_output(first, false);
            let (b, end, _) = text_output(crate::ai_terminals::TerminalOutput { data: bytes[cursor as usize..].to_vec(), cursor: bytes.len() as u64, truncated: false }, false);
            assert_eq!(a + &b, "中文输出🙂done");
            assert_eq!(end, bytes.len() as u64);
        }
    }
}
