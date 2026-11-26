use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    pin::Pin,
    sync::{Arc, atomic::{AtomicUsize, Ordering}},
    task::{Context, Poll},
};

use axum::{
    body::{Body, Bytes},
    http::{Method, Request, header::CONTENT_LENGTH},
    middleware::Next,
    response::Response,
};
use http_body::{Body as HttpBody, Frame, SizeHint};
use serde::Deserialize;
use serde_json::{Value, json, value::RawValue};
use tokio::sync::mpsc;
use zeroize::Zeroizing;

const MAX_LOG_BYTES: u64 = 512 * 1024;

const METHODS: &[&str] = &["initialize", "notifications/initialized", "ping", "tools/list", "tools/call"];
const TOOLS: &[&str] = &[
    "list_connections", "list_sessions", "open_connection", "take_over_session",
    "open_dedicated_terminal", "release_session",
    "read_output", "send_input", "execute_command",
    "disconnect", "upload_file", "download_file",
    "get_file_transfer", "list_file_transfers",
];
const FIELDS: &[&str] = &[
    "session_id", "terminal_id", "mode", "cursor", "max_bytes", "command", "data",
    "timeout_seconds", "action", "append_newline", "is_command", "connection_id", "credential",
    "local_path", "remote_path", "remote_directory", "transfer_id",
];

#[derive(Clone)]
pub(super) struct McpDiagnostics {
    entries: mpsc::Sender<Value>,
    dropped: Arc<AtomicUsize>,
}

impl McpDiagnostics {
    pub(super) fn start(path: PathBuf, port: u16) -> Self {
        let (entries, mut receiver) = mpsc::channel::<Value>(256);
        let dropped = Arc::new(AtomicUsize::new(0));
        let dropped_for_writer = Arc::clone(&dropped);
        tokio::task::spawn_blocking(move || {
            let mut reported_failure = false;
            while let Some(mut entry) = receiver.blocking_recv() {
                let missed = dropped_for_writer.swap(0, Ordering::Relaxed);
                if missed != 0 {
                    entry["dropped_entries"] = json!(missed);
                }
                if append(&path, &entry).is_err() {
                    dropped_for_writer.fetch_add(missed.saturating_add(1), Ordering::Relaxed);
                    if !reported_failure {
                        eprintln!("无法写入 OrbitShell MCP 诊断日志");
                        reported_failure = true;
                    }
                }
            }
        });
        let diagnostics = Self { entries, dropped };
        diagnostics.record(None, "listener_started", json!({ "port": port }));
        diagnostics
    }

    fn record(&self, request_id: Option<&str>, event: &'static str, details: Value) {
        let entry = json!({
            "timestamp": chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, false),
            "request_id": request_id,
            "event": event,
            "details": details,
        });
        if self.entries.try_send(entry).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(super) async fn observe(
        &self,
        mut request: Request<Body>,
        next: Next,
        max_body_bytes: usize,
    ) -> Response {
        if request.method() != Method::POST {
            return next.run(request).await;
        }

        let trace = RequestTrace {
            diagnostics: self.clone(),
            id: uuid::Uuid::new_v4().simple().to_string(),
        };
        let declared_bytes = request
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|header| header.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok());
        trace.record("http_received", json!({ "declared_bytes": declared_bytes }));
        request.extensions_mut().insert(trace.clone());
        // Observe the same frames that rmcp reads, leaving its header and body limits in place.
        let request = request.map(|inner| {
            Body::new(ObservedBody {
                inner,
                trace: trace.clone(),
                captured: Some(Zeroizing::new(Vec::new())),
                max_body_bytes,
            })
        });
        let response = next.run(request).await;
        trace.record(
            "http_response",
            json!({ "status": response.status().as_u16() }),
        );
        response
    }
}

#[derive(Clone)]
pub(super) struct RequestTrace {
    diagnostics: McpDiagnostics,
    id: String,
}

impl RequestTrace {
    pub(super) fn record(&self, event: &'static str, details: Value) {
        self.diagnostics.record(Some(&self.id), event, details);
    }
}

pub(super) fn tool_name(name: &str) -> &'static str {
    known_label(name, TOOLS)
}

fn known_label(name: &str, allowed: &'static [&'static str]) -> &'static str {
    allowed.iter().copied().find(|label| *label == name).unwrap_or("other")
}

struct ObservedBody {
    inner: Body,
    trace: RequestTrace,
    captured: Option<Zeroizing<Vec<u8>>>,
    max_body_bytes: usize,
}

impl HttpBody for ObservedBody {
    type Data = Bytes;
    type Error = <Body as HttpBody>::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        let this = self.get_mut();
        let frame = Pin::new(&mut this.inner).poll_frame(cx);
        match &frame {
            Poll::Ready(Some(Ok(frame))) => {
                if let Some(data) = frame.data_ref()
                    && let Some(captured) = &mut this.captured
                {
                    let bytes = captured.len().saturating_add(data.len());
                    if bytes > this.max_body_bytes {
                        this.captured = None;
                        this.trace.record("body_limit", json!({ "bytes_at_least": bytes }));
                    } else {
                        captured.extend_from_slice(data);
                    }
                }
            }
            Poll::Ready(Some(Err(_))) => {
                if let Some(captured) = this.captured.take() {
                    this.trace.record("body_read_error", json!({ "wire_bytes": captured.len() }));
                }
            }
            Poll::Ready(None) => {
                if let Some(captured) = this.captured.take() {
                    this.trace.record("body_parsed", summarize(&captured));
                }
            }
            Poll::Pending => {}
        }
        frame
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

#[derive(Deserialize)]
struct Envelope<'a> {
    #[serde(borrow)]
    method: Option<&'a RawValue>,
    #[serde(borrow)]
    params: Option<&'a RawValue>,
}

fn summarize(body: &[u8]) -> Value {
    use serde_json::error::Category;
    let envelope = match serde_json::from_slice::<Envelope<'_>>(body) {
        Ok(envelope) => envelope,
        Err(error) => {
            let category = match error.classify() {
                Category::Syntax => "syntax",
                Category::Eof => "unexpected_eof",
                Category::Data => "envelope",
                Category::Io => "io",
            };
            return json!({
                "wire_bytes": body.len(),
                "error": { "category": category, "line": error.line(), "column": error.column() },
            });
        }
    };
    let method = envelope.method.map_or("other", |raw| known_label(unquoted(raw), METHODS));
    let mut metadata = json!({ "wire_bytes": body.len(), "method": method });
    if method == "tools/call"
        && let Some(params) = envelope.params
        && let Ok(params) = serde_json::from_str::<BTreeMap<String, &RawValue>>(params.get())
    {
        metadata["tool"] = json!(params.get("name").map_or("other", |raw| tool_name(unquoted(raw))));
        metadata["arguments"] = params.get("arguments")
            .map_or_else(|| json!({ "type": "missing" }), |raw| summarize_arguments(raw));
    }
    metadata
}

fn summarize_arguments(arguments: &RawValue) -> Value {
    let raw = arguments.get();
    let mut metadata = json!({ "type": json_type(raw), "wire_bytes": raw.len() });
    if let Ok(arguments) = serde_json::from_str::<BTreeMap<String, &RawValue>>(raw) {
        let mut fields = serde_json::Map::new();
        for key in FIELDS.iter().copied() {
            let Some(value) = arguments.get(key) else { continue };
            let raw = value.get();
            let mut field = json!({ "type": json_type(raw) });
            if key != "credential" {
                field["wire_bytes"] = json!(raw.len());
            }
            match key {
                "mode" => field["value"] = json!(known_label(unquoted(value), &["dedicated", "takeover"])),
                "action" => field["value"] = json!(known_label(unquoted(value), &["input", "takeover", "release"])),
                "append_newline" | "is_command" if matches!(raw, "true" | "false") => {
                    field["value"] = json!(raw == "true");
                }
                "data" | "command" if json_type(raw) == "string" => {
                    field["newline_escapes"] = json!(count_newline_escapes(raw));
                }
                _ => {}
            }
            fields.insert(key.into(), field);
        }
        metadata["unknown_fields"] = json!(arguments.len() - fields.len());
        metadata["fields"] = Value::Object(fields);
    }
    metadata
}

fn unquoted(raw: &RawValue) -> &str {
    raw.get().strip_prefix('"').and_then(|value| value.strip_suffix('"')).unwrap_or_default()
}

fn json_type(raw: &str) -> &'static str {
    match raw.as_bytes().first() {
        Some(b'{') => "object",
        Some(b'[') => "array",
        Some(b'"') => "string",
        Some(b't' | b'f') => "boolean",
        Some(b'n') => "null",
        _ => "number",
    }
}

fn count_newline_escapes(raw: &str) -> usize {
    let mut bytes = raw.bytes();
    let mut count = 0;
    while let Some(byte) = bytes.next() {
        if byte == b'\\' && matches!(bytes.next(), Some(b'n' | b'r')) {
            count += 1;
        }
    }
    count
}

fn append(path: &Path, entry: &Value) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let _lock = orbitshell_ssh::lock_workspace(path)?;
    let mut line = serde_json::to_vec(entry).map_err(io::Error::other)?;
    line.push(b'\n');
    if fs::metadata(path).is_ok_and(|file| file.len() + line.len() as u64 > MAX_LOG_BYTES) {
        let previous = path.with_extension("previous.jsonl");
        match fs::remove_file(&previous) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        fs::rename(path, previous)?;
    }
    OpenOptions::new().create(true).append(true).open(path)?.write_all(&line)
}
