use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use russh::{client, Channel, ChannelMsg, Pty};
use serde::Serialize;
use tokio::{
    sync::{mpsc, oneshot, watch},
    time::{Instant as TokioInstant, sleep_until, timeout},
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::{
    command_guard::{self, ReadOnlyCommand},
    session::{LiveSession, OutputBuffer, PtySizeChanges},
};

const CAPTURE_LIMIT: usize = 512 * 1024;
const DEFAULT_PTY_SIZE: (u32, u32) = (120, 36);
const TERMINAL_ENDED: &str = "AI 终端已结束";
const INTERRUPT_UNCONFIRMED: &str = "已发出停止请求，等待确认远端退出状态";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellStatus {
    pub status: &'static str,
    pub message: &'static str,
    pub command: Option<String>,
    #[serde(skip)]
    pub started: Option<Instant>,
    #[serde(skip)]
    pub interrupt_pending: bool,
    pub elapsed_ms: u64,
    pub exit_code: Option<u32>,
}

impl ShellStatus {
    pub fn elapsed(&self) -> u64 {
        if self.status == "running" {
            self.started
                .map_or(0, |start| start.elapsed().as_millis() as u64)
        } else {
            self.elapsed_ms
        }
    }
}

pub struct ShellOutput {
    pub bytes: Vec<u8>,
    pub cursor: u64,
    pub truncated: bool,
    pub has_more: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandResult {
    pub stdout: String,
    pub stderr: &'static str,
    pub exit_code: Option<u32>,
    pub elapsed_ms: u64,
    pub running: bool,
    pub truncated: bool,
    pub note: &'static str,
}

type Delivery = oneshot::Sender<Result<(), String>>;

enum Input {
    Execute {
        command: Zeroizing<String>,
        display: String,
        sent: Delivery,
        result: oneshot::Sender<CommandResult>,
        read_only: Option<CancellationToken>,
    },
    Literal {
        bytes: Zeroizing<Vec<u8>>,
        sent: Delivery,
        display: Option<String>,
    },
    Interrupt {
        started: Instant,
    },
}

pub struct AiShell {
    status: Mutex<ShellStatus>,
    output: Mutex<OutputBuffer>,
    changes: watch::Sender<()>,
    input: mpsc::Sender<Input>,
    size: watch::Sender<(u32, u32)>,
    read_only: Mutex<Option<CancellationToken>>,
    pub cancel: CancellationToken,
}

impl AiShell {
    pub fn start(
        live: Arc<LiveSession>,
        changes: watch::Sender<()>,
        service: CancellationToken,
        client: CancellationToken,
    ) -> Arc<Self> {
        let (input, inputs) = mpsc::channel(8);
        let (size, sizes) = watch::channel(DEFAULT_PTY_SIZE);
        let shell = Arc::new(Self {
            status: Mutex::new(ShellStatus {
                status: "connecting",
                message: "正在打开独立 AI 终端",
                command: None,
                started: None,
                interrupt_pending: false,
                elapsed_ms: 0,
                exit_code: None,
            }),
            output: Mutex::new(OutputBuffer::new()),
            changes,
            input,
            size,
            read_only: Mutex::default(),
            cancel: service.child_token(),
        });
        tokio::spawn(run(shell.clone(), live, client, inputs, sizes));
        shell
    }

    pub fn status(&self) -> ShellStatus {
        self.status
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    fn update(&self, change: impl FnOnce(&mut ShellStatus)) {
        change(
            &mut self
                .status
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
        );
        self.changes.send_replace(());
    }

    fn append(&self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.output
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .append(bytes);
        self.changes.send_replace(());
    }

    pub fn read(&self, cursor: Option<u64>, limit: usize) -> ShellOutput {
        let output = self
            .output
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        read_buffer(&output, cursor, limit)
    }

    pub fn resize(&self, columns: u32, rows: u32) {
        let size = (columns.clamp(20, 500), rows.clamp(5, 300));
        self.size.send_if_modified(|current| {
            if *current == size {
                return false;
            }
            *current = size;
            true
        });
    }

    pub fn read_only_available(&self) -> bool {
        !self.cancel.is_cancelled() && matches!(self.status().status, "connecting" | "idle")
    }

    pub fn read_only_authorized(&self) -> bool {
        self.read_only_lease().is_some()
    }

    fn read_only_lease(&self) -> Option<CancellationToken> {
        if self.cancel.is_cancelled() {
            return None;
        }
        self.read_only
            .lock()
            .ok()?
            .as_ref()
            .filter(|lease| !lease.is_cancelled())
            .cloned()
    }

    pub fn grant_read_only(&self) -> bool {
        if !self.read_only_available() {
            return false;
        }
        let Ok(mut current) = self.read_only.lock() else {
            return false;
        };
        if let Some(previous) = current.replace(self.cancel.child_token()) {
            previous.cancel();
        }
        self.changes.send_replace(());
        true
    }

    pub fn revoke_read_only(&self) {
        let Ok(mut current) = self.read_only.lock() else {
            return;
        };
        if let Some(lease) = current.take() {
            lease.cancel();
            self.changes.send_replace(());
        }
    }

    pub async fn execute(
        &self,
        command: Zeroizing<String>,
        display: String,
        seconds: u64,
        on_sent: impl FnOnce() + Send,
    ) -> Result<CommandResult, String> {
        self.submit_execution(command, display, seconds, None, on_sent)
            .await
    }

    pub async fn execute_read_only(
        &self,
        command: ReadOnlyCommand,
        display: String,
        seconds: u64,
        on_sent: impl FnOnce() + Send,
    ) -> Result<CommandResult, String> {
        let lease = self.read_only_lease().ok_or("只读授权已结束，请重新确认")?;
        self.submit_execution(command.wire(), display, seconds, Some(lease), on_sent)
            .await
    }

    async fn submit_execution(
        &self,
        command: Zeroizing<String>,
        display: String,
        seconds: u64,
        read_only: Option<CancellationToken>,
        on_sent: impl FnOnce() + Send,
    ) -> Result<CommandResult, String> {
        let (sent, acknowledgement) = oneshot::channel();
        let (result, completion) = oneshot::channel();
        command_guard::check_command(&command)?;
        self.input
            .try_send(Input::Execute {
                command,
                display,
                sent,
                result,
                read_only,
            })
            .map_err(|_| "AI 终端忙或已关闭")?;
        send_receipt(acknowledgement).await?;
        on_sent();
        match timeout(Duration::from_secs(seconds.clamp(1, 300)), completion).await {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(_)) => Err("AI 终端已关闭".into()),
            Err(_) => {
                let status = self.status();
                Ok(CommandResult {
                    stdout: String::new(),
                    stderr: "",
                    exit_code: None,
                    elapsed_ms: status.elapsed(),
                    running: true,
                    truncated: false,
                    note: "等待超时，命令未被取消；请用 read_output 继续观察。",
                })
            }
        }
    }

    pub async fn send(
        &self,
        bytes: Zeroizing<Vec<u8>>,
        display: Option<String>,
    ) -> Result<(), String> {
        let (sent, acknowledgement) = oneshot::channel();
        self.input
            .try_send(Input::Literal {
                bytes,
                sent,
                display,
            })
            .map_err(|_| "AI 终端忙或已关闭")?;
        send_receipt(acknowledgement).await
    }

    pub fn interrupt(&self, started: Instant) -> Result<(), &'static str> {
        if self.cancel.is_cancelled() {
            return Err("AI 终端已结束。");
        }
        self.input
            .try_send(Input::Interrupt { started })
            .map_err(|_| "AI 终端暂时无法接收停止请求。")
    }

    pub fn end(&self) {
        self.cancel.cancel();
        self.revoke_read_only();
        self.changes.send_replace(());
    }
}

async fn send_receipt(
    acknowledgement: oneshot::Receiver<Result<(), String>>,
) -> Result<(), String> {
    timeout(Duration::from_secs(25), acknowledgement)
        .await
        .map_err(|_| "发送状态未知，请读取终端输出后再决定是否重试")?
        .map_err(|_| "AI 终端已关闭，未收到发送确认")?
}

pub fn read_buffer(buffer: &OutputBuffer, cursor: Option<u64>, limit: usize) -> ShellOutput {
    let first = buffer.end.saturating_sub(buffer.bytes.len() as u64);
    let requested = cursor.unwrap_or(first);
    let start = requested.clamp(first, buffer.end);
    let bytes: Vec<_> = buffer
        .bytes
        .iter()
        .skip((start - first) as usize)
        .take(limit.clamp(1, 65536))
        .copied()
        .collect();
    let next = start + bytes.len() as u64;
    ShellOutput {
        bytes,
        cursor: next,
        truncated: requested < first || (cursor.is_none() && first > 0),
        has_more: next < buffer.end,
    }
}

pub struct CompletionMarker {
    prefix: Vec<u8>,
    pending: Vec<u8>,
}

impl CompletionMarker {
    pub fn new(nonce: &str) -> Self {
        Self {
            prefix: format!("\x1b]633;OrbitShell;{nonce};").into_bytes(),
            pending: Vec::new(),
        }
    }

    pub fn take_pending(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.pending)
    }

    pub fn push(&mut self, bytes: &[u8]) -> (Vec<u8>, Option<u32>) {
        self.pending.extend_from_slice(bytes);
        let mut visible = Vec::new();
        loop {
            if let Some(index) = self
                .pending
                .windows(self.prefix.len())
                .position(|slice| slice == self.prefix)
            {
                visible.extend(self.pending.drain(..index));
                if let Some(end) = self.pending[self.prefix.len()..]
                    .iter()
                    .position(|byte| *byte == 7)
                {
                    let end = self.prefix.len() + end;
                    if let Ok(code) = std::str::from_utf8(&self.pending[self.prefix.len()..end])
                        .unwrap_or("")
                        .parse::<u32>()
                    {
                        self.pending.drain(..=end);
                        visible.append(&mut self.pending);
                        return (visible, Some(code));
                    }
                } else if self.pending.len() < self.prefix.len() + 16 {
                    return (visible, None);
                }
                visible.push(self.pending.remove(0));
            } else {
                let keep = (1..self.prefix.len())
                    .rev()
                    .find(|length| self.pending.ends_with(&self.prefix[..*length]))
                    .unwrap_or(0);
                let count = self.pending.len() - keep;
                visible.extend(self.pending.drain(..count));
                return (visible, None);
            }
        }
    }
}

pub fn command_frame(command: &str, nonce: &str) -> Zeroizing<String> {
    // Run in the current POSIX shell to retain cd/export. Physical lines stay below the PTY limit.
    let characters = Zeroizing::new(command.chars().collect::<Vec<_>>());
    let pieces = Zeroizing::new(
        characters
            .chunks(300)
            .map(|chunk| {
                let text = Zeroizing::new(chunk.iter().collect::<String>());
                format!("'{}'", text.replace('\'', "'\"'\"'"))
            })
            .collect::<Vec<_>>(),
    );
    let quoted = Zeroizing::new(pieces.join("\\\n"));
    Zeroizing::new(format!(
        "eval {}; command printf '\\033]633;OrbitShell;{nonce};%s\\007' \"$?\"\r",
        quoted.as_str()
    ))
}

struct OpeningChannel(Option<Channel<client::Msg>>);
impl Drop for OpeningChannel {
    fn drop(&mut self) {
        if let Some(channel) = self.0.take() {
            drop(channel.into_stream());
        }
    }
}

pub async fn open(
    live: &LiveSession,
    append: impl Fn(&[u8]) + Send + Sync,
) -> Result<Channel<client::Msg>, String> {
    async fn accept(
        channel: &mut Channel<client::Msg>,
        append: &(impl Fn(&[u8]) + Sync),
    ) -> Result<(), String> {
        loop {
            match channel.wait().await {
                Some(ChannelMsg::Success) => return Ok(()),
                Some(ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. }) => {
                    append(&data)
                }
                Some(ChannelMsg::Failure | ChannelMsg::Close) | None => {
                    return Err("服务器拒绝独立 AI 终端".into())
                }
                _ => {}
            }
        }
    }
    let connection = live.connection.get().ok_or("SSH 尚未就绪")?;
    let mut pending = OpeningChannel(Some(
        connection
            .channel_open_session()
            .await
            .map_err(|_| "无法建立独立 AI 通道")?,
    ));
    let channel = pending.0.as_mut().ok_or("AI 通道已关闭")?;
    if let Some(trace) = live.diagnostics.get() {
        trace.bind(channel.id(), "ai_shell");
    }
    let result = timeout(Duration::from_secs(15), async {
        channel
            .request_pty(
                true,
                "xterm-256color",
                DEFAULT_PTY_SIZE.0,
                DEFAULT_PTY_SIZE.1,
                0,
                0,
                &[(Pty::ECHO, 0), (Pty::ECHONL, 0)],
            )
            .await
            .map_err(|_| "无法申请 AI 终端")?;
        accept(channel, &append).await?;
        channel
            .request_shell(true)
            .await
            .map_err(|_| "无法启动 AI Shell")?;
        accept(channel, &append).await
    })
    .await
    .unwrap_or_else(|_| Err("独立 AI 终端初始化超时".into()));
    if let Err(error) = result {
        let _ = timeout(Duration::from_secs(2), channel.close()).await;
        return Err(error);
    }
    pending.0.take().ok_or_else(|| "AI 通道已关闭".into())
}

struct Running {
    marker: CompletionMarker,
    result: oneshot::Sender<CommandResult>,
    started: Instant,
    capture: Vec<u8>,
    truncated: bool,
}

impl Running {
    fn capture(&mut self, data: &[u8]) {
        let count = data
            .len()
            .min(CAPTURE_LIMIT.saturating_sub(self.capture.len()));
        self.capture.extend_from_slice(&data[..count]);
        self.truncated |= count < data.len();
    }

    fn finish(
        self,
        shell: &AiShell,
        exit: Option<u32>,
        status: &'static str,
        message: &'static str,
    ) -> CompletionMarker {
        let elapsed_ms = self.started.elapsed().as_millis() as u64;
        shell.update(|state| {
            state.status = status;
            state.message = message;
            state.elapsed_ms = elapsed_ms;
            state.exit_code = exit;
            state.interrupt_pending = false;
        });
        let _ = self.result.send(CommandResult {
            stdout: String::from_utf8_lossy(&self.capture).into_owned(),
            stderr: "",
            exit_code: exit,
            elapsed_ms,
            running: status == "interactive",
            truncated: self.truncated,
            note: message,
        });
        self.marker
    }
}

async fn run(
    shell: Arc<AiShell>,
    live: Arc<LiveSession>,
    client: CancellationToken,
    mut inputs: mpsc::Receiver<Input>,
    sizes: watch::Receiver<(u32, u32)>,
) {
    let channel = tokio::select! {
        _ = shell.cancel.cancelled() => Err(TERMINAL_ENDED.into()),
        _ = live.cancel.cancelled() => Err("所属 SSH 会话已断开".into()),
        _ = client.cancelled() => Err("MCP 客户端已断开".into()),
        opened = timeout(Duration::from_secs(20), open(&live, |bytes| shell.append(bytes))) => opened.unwrap_or_else(|_| Err("打开 AI 通道超时".into())),
    };
    let channel = match channel {
        Ok(channel) => channel,
        Err(error) => {
            let ended = shell.cancel.is_cancelled();
            shell.cancel.cancel();
            shell.revoke_read_only();
            if ended {
                shell.update(|status| {
                    status.status = "closed";
                    status.message = TERMINAL_ENDED;
                });
                return;
            }
            shell.update(|status| {
                status.status = "error";
                status.message = "无法打开独立 AI 终端";
            });
            shell.append(format!("\r\n[OrbitShell] {error}\r\n").as_bytes());
            return;
        }
    };
    let (mut reader, writer) = channel.split();
    let mut sizes = PtySizeChanges::new(sizes, DEFAULT_PTY_SIZE, live.is_temporary);
    shell.update(|state| {
        state.status = "idle";
        state.message = "等待 Agent 指令";
    });
    let mut running: Option<Running> = None;
    let mut interrupt_deadline = None;
    let mut interrupted_marker: Option<CompletionMarker> = None;
    let mut disconnected = false;
    let message = loop {
        tokio::select! {
            biased;
            _ = shell.cancel.cancelled() => break TERMINAL_ENDED,
            _ = live.cancel.cancelled() => break "所属 SSH 会话已断开",
            _ = client.cancelled(), if !disconnected => {
                disconnected = true;
                shell.revoke_read_only();
                shell.update(|state| state.message = "Agent 已断开");
            }
            _ = async {
                if let Some(deadline) = interrupt_deadline {
                    sleep_until(deadline).await;
                }
            }, if interrupt_deadline.is_some() => {
                interrupt_deadline = None;
                if let Some(command) = running.take() {
                    // Some shells skip the trailing completion marker after Ctrl+C.
                    // Keep observing it without claiming that the command exited.
                    if let Some(mut marker) = interrupted_marker.take() {
                        shell.append(&marker.take_pending());
                    }
                    interrupted_marker = Some(command.finish(
                        &shell,
                        None,
                        "interactive",
                        INTERRUPT_UNCONFIRMED,
                    ));
                }
            }
            size = sizes.next() => {
                let Some((columns, rows)) = size else { break "AI 终端已关闭" };
                tokio::select! {
                    biased;
                    _ = shell.cancel.cancelled() => break TERMINAL_ENDED,
                    _ = writer.window_change(columns, rows, 0, 0) => {}
                }
            }
            input = inputs.recv() => match input {
                Some(Input::Execute { command, display, sent, result, read_only }) => {
                    if client.is_cancelled() || sent.is_closed() { continue; }
                    if running.is_some() { let _ = sent.send(Err("已有命令在运行，请先查看输出".into())); continue; }
                    if read_only.is_some() && (shell.status().status != "idle" || live.cancel.is_cancelled()) {
                        let _ = sent.send(Err("独立 AI 终端当前无法继续只读排查，请重新确认".into()));
                        continue;
                    }
                    let nonce = Uuid::new_v4().simple().to_string();
                    let wire = command_frame(&command, &nonce);
                    let submitted = if let Some(lease) = &read_only {
                        tokio::select! {
                            biased;
                            _ = lease.cancelled() => None,
                            _ = shell.cancel.cancelled() => None,
                            _ = live.cancel.cancelled() => None,
                            _ = client.cancelled() => None,
                            result = writer.data_bytes(wire.as_bytes().to_vec()) => Some(result),
                        }
                    } else {
                        tokio::select! {
                            biased;
                            _ = shell.cancel.cancelled() => break TERMINAL_ENDED,
                            result = writer.data_bytes(wire.as_bytes().to_vec()) => Some(result),
                        }
                    };
                    let Some(submitted) = submitted else {
                        let _ = sent.send(Err("只读授权或所属会话已结束，命令未发送".into()));
                        continue;
                    };
                    if submitted.is_err() {
                        let _ = sent.send(Err("AI 命令发送失败".into()));
                        break "AI 命令发送失败";
                    }
                    let started = Instant::now();
                    shell.append(format!("\r\n\x1b[36m$ \x1b[0m{}\r\n", display.replace('\n', "\r\n")).as_bytes());
                    shell.update(|state| {
                        state.status = "running";
                        state.message = if read_only.is_some() { "AI 只读排查中" } else { "AI 执行命令中" };
                        state.command = Some(display);
                        state.started = Some(started);
                        state.interrupt_pending = false;
                        state.exit_code = None;
                        state.elapsed_ms = 0;
                    });
                    running = Some(Running { marker: CompletionMarker::new(&nonce), result, started, capture: Vec::new(), truncated: false });
                    let _ = sent.send(Ok(()));
                }
                Some(Input::Interrupt { started }) => {
                    if interrupt_deadline.is_some()
                        || running.as_ref().is_none_or(|command| command.started != started)
                    {
                        continue;
                    }
                    let sent = tokio::select! {
                        biased;
                        _ = shell.cancel.cancelled() => break TERMINAL_ENDED,
                        result = timeout(Duration::from_secs(2), writer.data_bytes(vec![0x03])) => result,
                    };
                    if sent.is_ok_and(|result| result.is_ok()) {
                        interrupt_deadline = Some(TokioInstant::now() + Duration::from_secs(5));
                        shell.update(|state| {
                            state.interrupt_pending = true;
                            state.message = "已发出停止请求，等待远端响应";
                        });
                    } else {
                        shell.update(|state| state.message = "停止信号发送状态未知，可重试");
                    }
                }
                Some(Input::Literal { bytes, sent, display }) => {
                    if client.is_cancelled() || sent.is_closed() { continue; }
                    let submitted = tokio::select! {
                        biased;
                        _ = shell.cancel.cancelled() => break TERMINAL_ENDED,
                        result = writer.data_bytes(bytes.to_vec()) => result,
                    };
                    if submitted.is_err() {
                        let _ = sent.send(Err("AI 输入发送失败".into()));
                        break "AI 输入发送失败";
                    }
                    if let Some(command) = display.as_ref() {
                        shell.append(format!("\r\n\x1b[36m$ \x1b[0m{}\r\n", command.replace('\n', "\r\n")).as_bytes());
                    }
                    if running.is_none() {
                        shell.update(|state| { state.status = "interactive"; state.message = "交互输入已发送"; state.command = display; state.exit_code = None; state.elapsed_ms = 0; });
                    }
                    let _ = sent.send(Ok(()));
                }
                None => break "AI 终端已关闭",
            },
            message = reader.wait() => match message {
                Some(ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. }) => {
                    let mut visible = None;
                    if let Some(marker) = interrupted_marker.as_mut() {
                        let (bytes, exit) = marker.push(&data);
                        visible = Some(bytes);
                        if let Some(exit) = exit {
                            interrupted_marker = None;
                            if running.is_none() {
                                shell.update(|state| {
                                    if state.message == INTERRUPT_UNCONFIRMED {
                                        state.status = "idle";
                                        state.message = "命令已结束";
                                        state.exit_code = Some(exit);
                                    }
                                });
                            }
                        }
                    }
                    let data = visible.as_deref().unwrap_or(data.as_ref());
                    if let Some(command) = running.as_mut() {
                        let (visible, code) = command.marker.push(data);
                        command.capture(&visible);
                        shell.append(&visible);
                        if let Some(exit) = code {
                            if let Some(command) = running.take() {
                                interrupt_deadline = None;
                                command.finish(&shell, Some(exit), "idle", if disconnected { "Agent 已断开" } else { "等待 Agent 指令" });
                            }
                        }
                    } else { shell.append(data); }
                }
                Some(ChannelMsg::Failure) => break "服务器拒绝 AI 终端请求",
                Some(ChannelMsg::Close) | None => break "远程 AI Shell 已结束",
                _ => {}
            }
        }
    };
    if let Some(mut marker) = interrupted_marker {
        shell.append(&marker.take_pending());
    }
    if let Some(mut command) = running.take() {
        let pending = command.marker.take_pending();
        command.capture(&pending);
        shell.append(&pending);
        command.finish(&shell, None, "closed", message);
    }
    shell.cancel.cancel();
    shell.revoke_read_only();
    shell.update(|state| {
        state.status = "closed";
        state.message = message;
    });
    let _ = timeout(Duration::from_secs(2), writer.close()).await;
}
