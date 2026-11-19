use std::{
    collections::{HashMap, VecDeque},
    io,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::{SystemTime, UNIX_EPOCH},
};

use russh::{client, Channel, ChannelId, ChannelStream};
use serde::Serialize;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

const MAX_EVENTS: usize = 8192;

// Only protocol metadata crosses this boundary. Labels are local constants.
#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
struct Event {
    unix_ms: u64,
    channel: Option<u32>,
    role: &'static str,
    message: &'static str,
    #[serde(flatten)]
    details: EventDetails,
}

#[derive(Clone, Copy, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct EventDetails {
    #[serde(skip_serializing_if = "Option::is_none")]
    extended_type: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bytes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    columns: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rows: Option<u32>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelSnapshot {
    discarded_events: u64,
    events: VecDeque<Event>,
}

#[derive(Default)]
struct History {
    channels: HashMap<ChannelId, &'static str>,
    snapshot: ChannelSnapshot,
}

impl History {
    fn record(
        &mut self,
        channel: Option<ChannelId>,
        role: &'static str,
        message: &'static str,
        details: EventDetails,
    ) {
        let unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        if self.snapshot.events.len() == MAX_EVENTS {
            self.snapshot.events.pop_front();
            self.snapshot.discarded_events += 1;
        }
        self.snapshot.events.push_back(Event {
            unix_ms,
            channel: channel.map(u32::from),
            role,
            message,
            details,
        });
    }
}

#[derive(Default)]
pub struct ChannelDiagnostics(Mutex<History>);

impl ChannelDiagnostics {
    pub fn snapshot(&self) -> ChannelSnapshot {
        let history = self.0.lock().unwrap_or_else(|error| error.into_inner());
        history.snapshot.clone()
    }

    pub(crate) fn bind(&self, channel: ChannelId, role: &'static str) {
        let mut history = self.0.lock().unwrap_or_else(|error| error.into_inner());
        history.channels.insert(channel, role);
        history.record(
            Some(channel),
            role,
            "channel_assigned",
            EventDetails::default(),
        );
    }

    pub(crate) fn received(
        &self,
        channel: ChannelId,
        message: &'static str,
        extended_type: Option<u32>,
        bytes: Option<usize>,
    ) {
        let mut history = self.0.lock().unwrap_or_else(|error| error.into_inner());
        let role = history.channels.get(&channel).copied().unwrap_or("unassigned");
        let details = EventDetails { extended_type, bytes, ..Default::default() };
        history.record(Some(channel), role, message, details);
        if message == "SSH_MSG_CHANNEL_CLOSE" {
            history.channels.remove(&channel);
        }
    }

    pub(crate) fn sftp(
        &self,
        channel: Option<ChannelId>,
        message: &'static str,
        bytes: Option<usize>,
    ) {
        let mut history = self.0.lock().unwrap_or_else(|error| error.into_inner());
        history.record(channel, "sftp", message, EventDetails { bytes, ..Default::default() });
    }

    pub(crate) fn terminal_size(
        &self,
        channel: ChannelId,
        message: &'static str,
        columns: u32,
        rows: u32,
    ) {
        let mut history = self.0.lock().unwrap_or_else(|error| error.into_inner());
        let details = EventDetails {
            columns: Some(columns),
            rows: Some(rows),
            ..Default::default()
        };
        history.record(Some(channel), "shell", message, details);
    }

    pub fn file_panel(&self, event: FilePanelEvent) {
        let message = match event {
            FilePanelEvent::Opened => "file_panel_opened",
            FilePanelEvent::Closed => "file_panel_closed",
            FilePanelEvent::DragStarted => "file_panel_drag_started",
            FilePanelEvent::DragFinished => "file_panel_drag_finished",
        };
        let mut history = self.0.lock().unwrap_or_else(|error| error.into_inner());
        history.record(None, "ui", message, EventDetails::default());
    }
}

pub enum FilePanelEvent {
    Opened,
    Closed,
    DragStarted,
    DragFinished,
}

/// russh-sftp 3.0.1 calls write_all once per serialized request. Record its
/// first accepted write, including when the SSH window forces partial writes.
/// No part of the request buffer is inspected or copied by the observer.
pub(crate) struct SftpStream {
    stream: ChannelStream<client::Msg>,
    id: ChannelId,
    diagnostics: Option<Arc<ChannelDiagnostics>>,
    partial_request: bool,
}

impl SftpStream {
    pub(crate) fn new(
        channel: Channel<client::Msg>,
        diagnostics: Option<Arc<ChannelDiagnostics>>,
    ) -> Self {
        Self {
            id: channel.id(),
            stream: channel.into_stream(),
            diagnostics,
            partial_request: false,
        }
    }
}

impl AsyncRead for SftpStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}

impl AsyncWrite for SftpStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let written = std::task::ready!(Pin::new(&mut self.stream).poll_write(cx, buf))?;
        if written == 0 {
            return Poll::Ready(Ok(0));
        }
        if let Some(trace) = &self.diagnostics {
            if !self.partial_request {
                trace.sftp(Some(self.id), "sftp_request_write", Some(buf.len()));
            }
            self.partial_request = written < buf.len();
        }
        Poll::Ready(Ok(written))
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}
