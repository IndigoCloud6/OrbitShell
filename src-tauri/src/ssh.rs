use crate::{storage::ConnectionProfile, AppState};
use anyhow::{anyhow, bail, Context, Result};
use russh::{client, keys::{self, HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate}, ChannelMsg, Disconnect};
use russh_sftp::client::SftpSession;
use serde::{Deserialize, Serialize};
use std::{sync::{Arc, Mutex}, time::Duration};
use tauri::{ipc::Channel, AppHandle, Manager, Emitter};
use std::{collections::VecDeque, sync::atomic::{AtomicBool, Ordering}};
use tokio::{sync::{mpsc, oneshot, OnceCell}, time::timeout};
use tokio_util::sync::CancellationToken;
use zeroize::{Zeroizing, Zeroize};

const OUTPUT_BUDGET: usize = 256 * 1024;

#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SessionEvent {
    Status { status: String, message: String },
    HostKey { hostname: String, port: u16, fingerprint: String, algorithm: String },
    Data { data: Vec<u8> },
    Exit { code: Option<u32>, message: String },
    Error { message: String },
    CredentialWarning { message: String },
}

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
    fn drop(&mut self) { self.credential.zeroize(); }
}

#[derive(Clone)]
pub struct SessionEvents {
    channel: Option<Channel<SessionEvent>>,
    broadcast: Option<AppHandle>,
    session_id: String,
}
impl SessionEvents {
    pub fn channel(channel: Channel<SessionEvent>) -> Self {
        Self { channel: Some(channel), broadcast: None, session_id: String::new() }
    }
    pub fn broadcast(app: AppHandle, session_id: String) -> Self {
        Self { channel: None, broadcast: Some(app), session_id }
    }
    pub fn announce(&self, profile: &ConnectionProfile) -> Result<()> {
        if let Some(app) = &self.broadcast {
            app.emit_to("main", "orbitshell:session-opened", serde_json::json!({"id": self.session_id, "profile": profile}))?;
        }
        Ok(())
    }
    pub fn send(&self, event: SessionEvent) -> Result<()> {
        if let Some(channel) = &self.channel { channel.send(event)?; }
        else if let Some(app) = &self.broadcast {
            app.emit_to("main", "orbitshell:session-event", serde_json::json!({"sessionId": self.session_id, "event": event}))?;
        }
        Ok(())
    }
}
pub struct OutputBuffer {
    pub bytes: VecDeque<u8>,
    pub end: u64,
}
impl OutputBuffer {
    pub fn new() -> Self { Self { bytes: VecDeque::new(), end: 0 } }
    pub fn append(&mut self, data: &[u8]) {
        self.bytes.extend(data.iter().copied());
        self.end += data.len() as u64;
        while self.bytes.len() > 1024 * 1024 { self.bytes.pop_front(); }
    }
}
pub enum SessionCommand {
    Input(Vec<u8>),
    Resize(u32, u32),
    Acknowledge(usize),
}

pub struct LiveSession {
    pub profile: ConnectionProfile,
    pub connected: AtomicBool,
    pub output: Mutex<OutputBuffer>,
    pub commands: mpsc::Sender<SessionCommand>,
    pub cancel: CancellationToken,
    pub trust: Mutex<Option<oneshot::Sender<bool>>>,
    pub verification_error: Mutex<Option<String>>,
    pub connection: OnceCell<Arc<client::Handle<SshHandler>>>,
    pub sftp: OnceCell<Arc<SftpSession>>,
}

impl LiveSession {
    pub async fn sftp(&self) -> Result<Arc<SftpSession>> {
        if self.cancel.is_cancelled() { bail!("连接已关闭"); }
        let connection = self.connection.get().ok_or_else(|| anyhow!("SSH 尚未连接"))?.clone();
        let value = self.sftp.get_or_try_init(|| async {
            timeout(Duration::from_secs(20), async {
                let channel = connection.channel_open_session().await?;
                channel.request_subsystem(true, "sftp").await?;
                // Wait for the server's acceptance before constructing the SFTP stream.
                let mut channel = channel;
                loop {
                    match channel.wait().await {
                        Some(ChannelMsg::Success) => break,
                        Some(ChannelMsg::Failure | ChannelMsg::Close) | None => bail!("服务器未启用 SFTP"),
                        _ => {}
                    }
                }
                Ok::<_, anyhow::Error>(Arc::new(SftpSession::new(channel.into_stream()).await?))
            }).await.context("SFTP 响应超时")?
        }).await?;
        Ok(value.clone())
    }
}

pub struct SshHandler {
    pub app: AppHandle,
    pub live: Arc<LiveSession>,
    pub profile: ConnectionProfile,
    pub events: SessionEvents,
}

impl client::Handler for SshHandler {
    type Error = anyhow::Error;

    async fn check_server_key(&mut self, server_key: &PublicKeyOrCertificate) -> Result<bool> {
        let key = server_key.public_key();
        let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();
        let address = format!("[{}]:{}", self.profile.hostname, self.profile.port);
        let state = self.app.state::<AppState>();
        let previous = state.storage.store.lock().map_err(|_| anyhow!("配置暂不可用"))?
            .known_hosts.get(&address).cloned();
        if let Some(previous) = previous {
            if previous == fingerprint { return Ok(true); }
            let reason = format!("服务器主机密钥发生变化，已停止连接。已保存：{previous}；当前：{fingerprint}。请先核实服务器身份。");
            *self.live.verification_error.lock().map_err(|_| anyhow!("会话状态不可用"))? = Some(reason);
            return Ok(false);
        }
        let (sender, receiver) = oneshot::channel();
        *self.live.trust.lock().map_err(|_| anyhow!("会话状态不可用"))? = Some(sender);
        self.events.send(SessionEvent::HostKey {
            hostname: self.profile.hostname.clone(), port: self.profile.port,
            fingerprint: fingerprint.clone(), algorithm: key.algorithm().to_string(),
        })?;
        let accepted = tokio::select! {
            _ = self.live.cancel.cancelled() => false,
            answer = timeout(Duration::from_secs(120), receiver) => matches!(answer, Ok(Ok(true))),
        };
        if !accepted { return Ok(false); }
        let mut store = state.storage.store.lock().map_err(|_| anyhow!("配置暂不可用"))?;
        if let Some(saved) = store.known_hosts.get(&address) {
            if saved != &fingerprint { bail!("另一会话保存了不同的服务器指纹，请重新核实"); }
        }
        let mut next = store.clone();
        next.known_hosts.insert(address, fingerprint);
        state.storage.persist(&next)?;
        *store = next;
        Ok(true)
    }
}

async fn establish(
    app: AppHandle, live: Arc<LiveSession>, profile: ConnectionProfile,
    request: &ConnectRequest, events: &SessionEvents,
) -> Result<russh::Channel<client::Msg>> {
    let config = Arc::new(client::Config {
        keepalive_interval: Some(Duration::from_secs(20)), keepalive_max: 3,
        ..Default::default()
    });
    let handler = SshHandler { app, live: live.clone(), profile: profile.clone(), events: events.clone() };
    let stream = timeout(Duration::from_secs(15), tokio::net::TcpStream::connect((profile.hostname.as_str(), profile.port)))
        .await.context("连接超时，请确认服务器地址与网络")??;
    stream.set_nodelay(true)?;
    let mut connection = timeout(Duration::from_secs(150), client::connect_stream(config, stream, handler))
        .await.context("SSH 握手超时")??;
    events.send(SessionEvent::Status { status: "connecting".into(), message: "正在验证登录信息…".into() })?;
    let credential = Zeroizing::new(request.credential.clone());
    let authenticated = if profile.auth_method == "privateKey" {
        let path = profile.private_key_path.clone().ok_or_else(|| anyhow!("未选择私钥"))?;
        let passphrase = if credential.is_empty() { None } else { Some(credential.as_str()) };
        let key = keys::load_secret_key(path, passphrase).context("无法读取私钥，请确认文件和口令")?;
        let hash = connection.best_supported_rsa_hash().await?.flatten();
        timeout(Duration::from_secs(30), connection.authenticate_publickey(
            &profile.username, PrivateKeyWithHashAlg::new(Arc::new(key), hash),
        )).await.context("密钥认证超时")??.success()
    } else {
        timeout(Duration::from_secs(30), connection.authenticate_password(&profile.username, credential.as_str()))
            .await.context("密码认证超时")??.success()
    };
    if !authenticated { bail!("认证失败，请确认用户名、密码或密钥"); }
    let connection = Arc::new(connection);
    let mut channel = connection.channel_open_session().await?;
    channel.request_pty(true, "xterm-256color", request.columns.clamp(20, 500), request.rows.clamp(5, 300), 0, 0, &[]).await?;
    wait_for_success(&mut channel, "服务器拒绝分配终端", &live, events).await?;
    channel.request_shell(true).await?;
    wait_for_success(&mut channel, "服务器拒绝启动 Shell", &live, events).await?;
    live.connection.set(connection).map_err(|_| anyhow!("会话已初始化"))?;
    Ok(channel)
}

async fn wait_for_success(channel: &mut russh::Channel<client::Msg>, failure: &str, live: &LiveSession, events: &SessionEvents) -> Result<()> {
    timeout(Duration::from_secs(15), async {
        loop {
            match channel.wait().await {
                Some(ChannelMsg::Success) => return Ok(()),
                Some(ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. }) => {
                    if let Ok(mut output) = live.output.lock() { output.append(&data); }
                    events.send(SessionEvent::Data { data: data.to_vec() })?;
                }
                Some(ChannelMsg::Failure | ChannelMsg::Close) | None => bail!("{failure}"),
                _ => {}
            }
        }
    }).await.context("远程终端请求超时")?
}

pub async fn run_session(
    app: AppHandle, live: Arc<LiveSession>, mut request: ConnectRequest,
    profile: ConnectionProfile, events: SessionEvents, mut commands: mpsc::Receiver<SessionCommand>,
) {
    let result = tokio::select! {
        _ = live.cancel.cancelled() => Ok(()),
        result = async {
            let channel = establish(app.clone(), live.clone(), profile.clone(), &request, &events).await;
            if channel.is_ok() && request.remember_credential {
                if let Err(message) = crate::remember_session_credential(&app, &profile, &request.credential) {
                    let _ = events.send(SessionEvent::CredentialWarning { message: format!("SSH 已连接，但密码未保存：{message}") });
                }
            }
            request.credential.zeroize();
            let channel = channel?;
            let (mut reader, writer) = channel.split();
            live.connected.store(true, Ordering::Release);
            events.send(SessionEvent::Status { status: "connected".into(), message: "已连接".into() })?;
            let mut outstanding = 0usize;
            let mut exit_code = None;
            loop {
                tokio::select! {
                    command = commands.recv() => match command {
                        Some(SessionCommand::Input(data)) => writer.data_bytes(data).await?,
                        Some(SessionCommand::Resize(columns, rows)) => writer.window_change(columns.clamp(20, 500), rows.clamp(5, 300), 0, 0).await?,
                        Some(SessionCommand::Acknowledge(bytes)) => outstanding = outstanding.saturating_sub(bytes),
                        None => break,
                    },
                    message = reader.wait(), if outstanding < OUTPUT_BUDGET => match message {
                        Some(ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. }) => {
                            outstanding += data.len();
                            if let Ok(mut output) = live.output.lock() { output.append(&data); }
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
        let message = live.verification_error.lock().ok().and_then(|value| value.clone())
            .unwrap_or_else(|| format!("{error:#}"));
        let _ = events.send(SessionEvent::Error { message });
    } else if live.cancel.is_cancelled() {
        let _ = events.send(SessionEvent::Exit { code: None, message: "连接已断开".into() });
    }
    live.cancel.cancel();
    // Stop SSH independently so a peer cannot keep the session map occupied.
    if let Some(connection) = live.connection.get() {
        let _ = timeout(Duration::from_secs(2), connection.disconnect(Disconnect::ByApplication, "OrbitShell session closed", "en")).await;
    }
    let state = app.state::<AppState>();
    if let Ok(mut sessions) = state.sessions.lock() { sessions.remove(&request.session_id); };
}
