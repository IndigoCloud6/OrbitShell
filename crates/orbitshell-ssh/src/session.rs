use crate::{
    channel_diagnostics::{ChannelDiagnostics, SftpStream},
    ConnectionProfile,
};
use anyhow::{anyhow, bail, Context, Result};
use russh::{
    client,
    keys::{self, HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate},
    Channel, ChannelId, ChannelMsg, Disconnect,
};
use russh_sftp::client::SftpSession;
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    fmt,
    sync::atomic::{AtomicBool, Ordering},
};
use std::{
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};
use tokio::{
    sync::{mpsc, oneshot, OnceCell},
    time::{sleep_until, timeout, timeout_at, Instant},
};
use tokio_util::sync::CancellationToken;
use zeroize::{Zeroize, Zeroizing};

const OUTPUT_BUDGET: usize = 256 * 1024;

#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SessionEvent {
    Status {
        status: String,
        message: String,
    },
    HostKey {
        hostname: String,
        port: u16,
        fingerprint: String,
        algorithm: String,
    },
    Data {
        data: Vec<u8>,
    },
    Exit {
        code: Option<u32>,
        message: String,
    },
    Error {
        message: String,
        #[serde(skip)]
        authentication: bool,
    },
    CredentialWarning {
        message: String,
    },
}

#[derive(Debug)]
struct AuthenticationError(&'static str);

impl fmt::Display for AuthenticationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for AuthenticationError {}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectRequest {
    pub session_id: String,
    pub profile_id: String,
    pub profile: Option<ConnectionProfile>,
    pub credential: String,
    #[serde(default)]
    pub use_saved_credential: bool,
    #[serde(default)]
    pub remember_credential: bool,
    pub columns: u32,
    pub rows: u32,
}

impl Drop for ConnectRequest {
    fn drop(&mut self) {
        self.credential.zeroize();
    }
}

#[derive(Clone)]
pub struct SessionEvents(Arc<dyn Fn(SessionEvent) -> Result<()> + Send + Sync>);
impl SessionEvents {
    pub fn new(send: impl Fn(SessionEvent) -> Result<()> + Send + Sync + 'static) -> Self {
        Self(Arc::new(send))
    }
    pub fn send(&self, event: SessionEvent) -> Result<()> {
        (self.0)(event)
    }
}

pub trait SessionHooks: Send + Sync {
    fn known_host(&self, address: &str) -> Result<Option<String>>;
    fn save_host(&self, address: String, fingerprint: String) -> Result<()>;
    fn remember_credential(
        &self,
        _profile: &ConnectionProfile,
        _secret: &str,
    ) -> Result<(), String> {
        Err("当前会话无法保存凭据".into())
    }
    fn connected(&self, _id: &str, _live: &LiveSession) {}
    fn allow_input(&self, _id: &str) -> bool {
        true
    }
    fn owns_input(&self, _owner: &str, _id: &str) -> bool {
        false
    }
    fn record_input(&self, _id: &str, _entry_id: String, _command: &str) {}
    fn output(&self, _id: &str) {}
    fn finished(&self, _id: &str) {}
}
pub struct OutputBuffer {
    pub bytes: VecDeque<u8>,
    pub end: u64,
}
impl OutputBuffer {
    pub fn new() -> Self {
        Self {
            bytes: VecDeque::new(),
            end: 0,
        }
    }
    pub fn append(&mut self, data: &[u8]) {
        self.bytes.extend(data.iter().copied());
        self.end += data.len() as u64;
        while self.bytes.len() > 1024 * 1024 {
            self.bytes.pop_front();
        }
    }
}
pub enum SessionCommand {
    Input {
        data: Vec<u8>,
        history: Option<(String, String)>,
    },
    AiInput {
        owner: String,
        data: Zeroizing<Vec<u8>>,
        complete: oneshot::Sender<std::result::Result<(), String>>,
    },
    Resize(u32, u32),
    Acknowledge(usize),
}

pub struct LiveSession {
    pub profile: ConnectionProfile,
    pub is_temporary: bool,
    pub connected: AtomicBool,
    pub output: Mutex<OutputBuffer>,
    pub commands: mpsc::Sender<SessionCommand>,
    pub cancel: CancellationToken,
    pub trust: Mutex<Option<oneshot::Sender<bool>>>,
    pub verification_error: Mutex<Option<String>>,
    pub connection: OnceCell<Arc<client::Handle<SshHandler>>>,
    pub sftp: OnceCell<Arc<SftpSession>>,
    pub diagnostics: OnceLock<Arc<ChannelDiagnostics>>,
}

impl LiveSession {
    pub fn new(
        profile: ConnectionProfile,
        is_temporary: bool,
    ) -> (Arc<Self>, mpsc::Receiver<SessionCommand>) {
        let (commands, receiver) = mpsc::channel(128);
        (
            Arc::new(Self {
                profile,
                is_temporary,
                commands,
                connected: AtomicBool::new(false),
                output: Mutex::new(OutputBuffer::new()),
                cancel: CancellationToken::new(),
                trust: Mutex::new(None),
                verification_error: Mutex::new(None),
                connection: OnceCell::new(),
                sftp: OnceCell::new(),
                diagnostics: OnceLock::new(),
            }),
            receiver,
        )
    }

    pub fn trust_host(&self, accept: bool) -> Result<(), String> {
        let answer = self
            .trust
            .lock()
            .map_err(|_| "会话状态不可用")?
            .take()
            .ok_or("确认已过期，请重新连接")?;
        answer
            .send(accept)
            .map_err(|_| "确认已过期，请重新连接".into())
    }

    pub(crate) async fn open_file_channel(
        &self,
        connection: &client::Handle<SshHandler>,
    ) -> Result<Channel<client::Msg>, russh::Error> {
        if let Some(trace) = self.diagnostics.get() {
            trace.sftp(None, "sftp_channel_open_attempt", None);
        }
        let channel = connection.channel_open_session().await?;
        if let Some(trace) = self.diagnostics.get() {
            trace.bind(channel.id(), "sftp");
        }
        Ok(channel)
    }

    pub(crate) async fn request_sftp_subsystem(
        &self,
        channel: &Channel<client::Msg>,
    ) -> Result<(), russh::Error> {
        if let Some(trace) = self.diagnostics.get() {
            trace.sftp(Some(channel.id()), "sftp_subsystem_request", None);
        }
        channel.request_subsystem(true, "sftp").await
    }

    pub(crate) fn sftp_stream(&self, channel: Channel<client::Msg>) -> SftpStream {
        SftpStream::new(channel, self.diagnostics.get().cloned())
    }

    pub async fn sftp(&self) -> Result<Arc<SftpSession>> {
        if self.cancel.is_cancelled() {
            bail!("连接已关闭");
        }
        if !self.connected.load(Ordering::Acquire) {
            bail!("SSH 尚未连接，请等待认证完成");
        }
        let connection = self
            .connection
            .get()
            .ok_or_else(|| anyhow!("SSH 尚未连接"))?
            .clone();
        let value = self
            .sftp
            .get_or_try_init(|| async {
                // A link-launched bastion can accept the shell before its target session is ready.
                // Only retry initial SFTP negotiation; never retry file writes or SSH authentication.
                let deadline =
                    Instant::now() + Duration::from_secs(if self.is_temporary { 60 } else { 20 });
                let mut retry_delay = Duration::from_millis(500);
                loop {
                    let attempt_deadline = deadline.min(Instant::now() + Duration::from_secs(20));
                    let result = tokio::select! {
                        _ = self.cancel.cancelled() => bail!("连接已关闭"),
                        result = open_sftp(self, &connection, attempt_deadline) => result,
                    };
                    match result {
                        Ok(sftp) => return Ok(sftp),
                        Err(error) => {
                            if let Some(trace) = self.diagnostics.get() {
                                trace.sftp(None, "sftp_setup_failed", None);
                            }
                            if !self.is_temporary {
                                return Err(error);
                            }
                            if Instant::now() >= deadline {
                                return Err(error)
                                    .context("等待堡垒机 SFTP 服务就绪超时，请稍后重试");
                            }
                            tokio::select! {
                                _ = self.cancel.cancelled() => bail!("连接已关闭"),
                                _ = sleep_until(deadline.min(Instant::now() + retry_delay)) => {}
                            }
                            if Instant::now() >= deadline {
                                return Err(error)
                                    .context("等待堡垒机 SFTP 服务就绪超时，请稍后重试");
                            }
                            retry_delay = (retry_delay * 2).min(Duration::from_secs(2));
                        }
                    }
                }
            })
            .await?;
        Ok(value.clone())
    }
}

async fn open_sftp(
    live: &LiveSession,
    connection: &client::Handle<SshHandler>,
    deadline: Instant,
) -> Result<Arc<SftpSession>> {
    let mut channel = timeout_at(deadline, live.open_file_channel(connection))
        .await
        .context("SFTP 通道建立超时")??;
    let accepted = timeout_at(deadline, async {
        live.request_sftp_subsystem(&channel).await?;
        loop {
            match channel.wait().await {
                Some(ChannelMsg::Success) => return Ok::<_, anyhow::Error>(()),
                Some(ChannelMsg::Failure | ChannelMsg::Close) | None => {
                    bail!("服务器尚未接受 SFTP 子系统")
                }
                _ => {}
            }
        }
    })
    .await
    .context("SFTP 响应超时")
    .and_then(|result| result);
    // ChannelStream closes its SSH channel on drop, including rejected or timed-out requests.
    let id = channel.id();
    let stream = live.sftp_stream(channel);
    accepted?;
    let session = timeout_at(deadline, SftpSession::new(stream))
        .await
        .context("SFTP 初始化超时")??;
    if let Some(trace) = live.diagnostics.get() {
        trace.sftp(Some(id), "sftp_ready", None);
    }
    Ok(Arc::new(session))
}

pub struct SshHandler {
    hooks: Arc<dyn SessionHooks>,
    live: Arc<LiveSession>,
    events: SessionEvents,
}

impl SshHandler {
    fn received(
        &self,
        channel: ChannelId,
        message: &'static str,
        ext: Option<u32>,
        bytes: Option<usize>,
    ) {
        if let Some(trace) = self.live.diagnostics.get() {
            trace.received(channel, message, ext, bytes);
        }
    }
}

impl client::Handler for SshHandler {
    type Error = anyhow::Error;

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        _: &mut client::Session,
    ) -> Result<()> {
        self.received(channel, "SSH_MSG_CHANNEL_DATA", None, Some(data.len()));
        Ok(())
    }

    async fn extended_data(
        &mut self,
        channel: ChannelId,
        ext: u32,
        data: &[u8],
        _: &mut client::Session,
    ) -> Result<()> {
        self.received(channel, "SSH_MSG_CHANNEL_EXTENDED_DATA", Some(ext), Some(data.len()));
        Ok(())
    }

    async fn channel_success(&mut self, channel: ChannelId, _: &mut client::Session) -> Result<()> {
        self.received(channel, "SSH_MSG_CHANNEL_SUCCESS", None, None);
        Ok(())
    }

    async fn channel_failure(&mut self, channel: ChannelId, _: &mut client::Session) -> Result<()> {
        self.received(channel, "SSH_MSG_CHANNEL_FAILURE", None, None);
        Ok(())
    }

    async fn channel_eof(&mut self, channel: ChannelId, _: &mut client::Session) -> Result<()> {
        self.received(channel, "SSH_MSG_CHANNEL_EOF", None, None);
        Ok(())
    }

    async fn channel_close(&mut self, channel: ChannelId, _: &mut client::Session) -> Result<()> {
        self.received(channel, "SSH_MSG_CHANNEL_CLOSE", None, None);
        Ok(())
    }

    async fn check_server_key(&mut self, server_key: &PublicKeyOrCertificate) -> Result<bool> {
        let key = server_key.public_key();
        let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();
        let profile = &self.live.profile;
        let address = format!("[{}]:{}", profile.hostname, profile.port);
        let previous = self.hooks.known_host(&address)?;
        if let Some(previous) = previous {
            if previous == fingerprint {
                return Ok(true);
            }
            let reason = format!("服务器主机密钥发生变化，已停止连接。已保存：{previous}；当前：{fingerprint}。请先核实服务器身份。");
            *self
                .live
                .verification_error
                .lock()
                .map_err(|_| anyhow!("会话状态不可用"))? = Some(reason);
            return Ok(false);
        }
        let (sender, receiver) = oneshot::channel();
        *self
            .live
            .trust
            .lock()
            .map_err(|_| anyhow!("会话状态不可用"))? = Some(sender);
        self.events.send(SessionEvent::HostKey {
            hostname: profile.hostname.clone(),
            port: profile.port,
            fingerprint: fingerprint.clone(),
            algorithm: key.algorithm().to_string(),
        })?;
        let accepted = tokio::select! {
            _ = self.live.cancel.cancelled() => false,
            answer = timeout(Duration::from_secs(120), receiver) => matches!(answer, Ok(Ok(true))),
        };
        if !accepted {
            return Ok(false);
        }
        self.hooks.save_host(address, fingerprint)?;
        Ok(true)
    }
}

async fn establish(
    hooks: Arc<dyn SessionHooks>,
    live: Arc<LiveSession>,
    profile: &ConnectionProfile,
    request: &ConnectRequest,
    events: &SessionEvents,
) -> Result<russh::Channel<client::Msg>> {
    let config = Arc::new(client::Config {
        keepalive_interval: Some(Duration::from_secs(20)),
        keepalive_max: 3,
        ..Default::default()
    });
    let handler = SshHandler {
        hooks,
        live: live.clone(),
        events: events.clone(),
    };
    let stream = timeout(
        Duration::from_secs(15),
        tokio::net::TcpStream::connect((profile.hostname.as_str(), profile.port)),
    )
    .await
    .context("连接超时，请确认服务器地址与网络")??;
    stream.set_nodelay(true)?;
    let mut connection = timeout(
        Duration::from_secs(150),
        client::connect_stream(config, stream, handler),
    )
    .await
    .context("SSH 握手超时")??;
    events.send(SessionEvent::Status {
        status: "connecting".into(),
        message: "正在验证登录信息…".into(),
    })?;
    let credential = Zeroizing::new(request.credential.clone());
    let authenticated = if profile.auth_method == "privateKey" {
        let path = profile
            .private_key_path
            .clone()
            .ok_or_else(|| anyhow!("未选择私钥"))?;
        let passphrase = if credential.is_empty() {
            None
        } else {
            Some(credential.as_str())
        };
        let key = keys::load_secret_key(path, passphrase)
            .map_err(|_| AuthenticationError("无法读取私钥，请确认文件和口令"))?;
        let hash = connection.best_supported_rsa_hash().await?.flatten();
        timeout(
            Duration::from_secs(30),
            connection.authenticate_publickey(
                &profile.username,
                PrivateKeyWithHashAlg::new(Arc::new(key), hash),
            ),
        )
        .await
        .context("密钥认证超时")??
        .success()
    } else {
        timeout(
            Duration::from_secs(30),
            connection.authenticate_password(&profile.username, credential.as_str()),
        )
        .await
        .context("密码认证超时")??
        .success()
    };
    if !authenticated {
        return Err(AuthenticationError("认证失败，请确认用户名、密码或密钥").into());
    }
    let connection = Arc::new(connection);
    let mut channel = connection.channel_open_session().await?;
    if let Some(trace) = live.diagnostics.get() {
        trace.bind(channel.id(), "shell");
    }
    let columns = request.columns.clamp(20, 500);
    let rows = request.rows.clamp(5, 300);
    channel
        .request_pty(true, "xterm-256color", columns, rows, 0, 0, &[])
        .await?;
    if let Some(trace) = live.diagnostics.get() {
        trace.terminal_size(channel.id(), "pty_request_sent", columns, rows);
    }
    wait_for_success(&mut channel, "服务器拒绝分配终端", &live, events).await?;
    channel.request_shell(true).await?;
    wait_for_success(&mut channel, "服务器拒绝启动 Shell", &live, events).await?;
    live.connection
        .set(connection)
        .map_err(|_| anyhow!("会话已初始化"))?;
    Ok(channel)
}

async fn wait_for_success(
    channel: &mut russh::Channel<client::Msg>,
    failure: &str,
    live: &LiveSession,
    events: &SessionEvents,
) -> Result<()> {
    timeout(Duration::from_secs(15), async {
        loop {
            match channel.wait().await {
                Some(ChannelMsg::Success) => return Ok(()),
                Some(ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. }) => {
                    if let Ok(mut output) = live.output.lock() {
                        output.append(&data);
                    }
                    events.send(SessionEvent::Data {
                        data: data.to_vec(),
                    })?;
                }
                Some(ChannelMsg::Failure | ChannelMsg::Close) | None => bail!("{failure}"),
                _ => {}
            }
        }
    })
    .await
    .context("远程终端请求超时")?
}

pub async fn run_session(
    hooks: Arc<dyn SessionHooks>,
    live: Arc<LiveSession>,
    mut request: ConnectRequest,
    events: SessionEvents,
    mut commands: mpsc::Receiver<SessionCommand>,
) {
    let profile = &live.profile;
    let result = tokio::select! {
        _ = live.cancel.cancelled() => Ok(()),
        result = async {
            let channel = establish(hooks.clone(), live.clone(), profile, &request, &events).await;
            if channel.is_ok() && request.remember_credential {
                if let Err(message) = hooks.remember_credential(profile, &request.credential) {
                    let _ = events.send(SessionEvent::CredentialWarning { message: format!("SSH 已连接，但凭据未保存：{message}") });
                }
            }
            request.credential.zeroize();
            let channel = channel?;
            let (mut reader, writer) = channel.split();
            live.connected.store(true, Ordering::Release);
            hooks.connected(&request.session_id, &live);
            events.send(SessionEvent::Status { status: "connected".into(), message: "已连接".into() })?;
            let mut outstanding = 0usize;
            let mut exit_code = None;
            loop {
                tokio::select! {
                    command = commands.recv() => match command {
                        Some(SessionCommand::Input { data, history }) => {
                            if hooks.allow_input(&request.session_id) {
                                writer.data_bytes(data).await?;
                                if let Some((id, command)) = history { hooks.record_input(&request.session_id, id, &command); }
                            }
                        },
                        Some(SessionCommand::AiInput { owner, data, complete }) => {
                            let owns = !complete.is_closed() && hooks.owns_input(&owner, &request.session_id);
                            let result = if owns { writer.data_bytes(data.to_vec()).await.map_err(|_| "AI 输入发送失败".to_string()) } else { Err("AI 接管已结束，输入未发送".into()) };
                            let _ = complete.send(result);
                        },
                        Some(SessionCommand::Resize(columns, rows)) => {
                            let columns = columns.clamp(20, 500);
                            let rows = rows.clamp(5, 300);
                            writer.window_change(columns, rows, 0, 0).await?;
                            if let Some(trace) = live.diagnostics.get() {
                                trace.terminal_size(writer.id(), "window_change_sent", columns, rows);
                            }
                        }
                        Some(SessionCommand::Acknowledge(bytes)) => outstanding = outstanding.saturating_sub(bytes),
                        None => break,
                    },
                    message = reader.wait(), if outstanding < OUTPUT_BUDGET => match message {
                        Some(ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. }) => {
                            outstanding += data.len();
                            if let Ok(mut output) = live.output.lock() { output.append(&data); }
                            hooks.output(&request.session_id);
                            events.send(SessionEvent::Data { data: data.to_vec() })?;
                        }
                        Some(ChannelMsg::ExitStatus { exit_status }) => exit_code = Some(exit_status),
                        Some(ChannelMsg::Failure) => bail!("远程终端请求被拒绝"),
                        Some(ChannelMsg::Close) | None => break,
                        _ => {}
                    }
                }
            }
            events.send(SessionEvent::Exit { code: exit_code, message: "远程会话已结束".into() })?;
            Ok::<(), anyhow::Error>(())
        } => result,
    };
    request.credential.zeroize();
    live.connected.store(false, Ordering::Release);
    if let Err(error) = result {
        let message = live
            .verification_error
            .lock()
            .ok()
            .and_then(|value| value.clone())
            .unwrap_or_else(|| format!("{error:#}"));
        let _ = events.send(SessionEvent::Error {
            message,
            authentication: error.is::<AuthenticationError>(),
        });
    } else if live.cancel.is_cancelled() {
        let _ = events.send(SessionEvent::Exit {
            code: None,
            message: "连接已断开".into(),
        });
    }
    live.cancel.cancel();
    // Stop SSH independently so a peer cannot keep the session map occupied.
    if let Some(connection) = live.connection.get() {
        let _ = timeout(
            Duration::from_secs(2),
            connection.disconnect(Disconnect::ByApplication, "OrbitShell session closed", "en"),
        )
        .await;
    }
    hooks.finished(&request.session_id);
}
