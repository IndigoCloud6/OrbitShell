use std::{
    io::{BufRead, BufReader as StdReader},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
};

use orbitshell_ssh::{ConnectionProfile, session::SessionEvent};
use tokio::{io::{AsyncBufReadExt, AsyncWriteExt, BufReader}, net::{TcpListener, TcpStream}, time::{sleep, timeout}};

use super::*;
use crate::ssh_backend::Backend;

struct Fixture(Child);
impl Drop for Fixture {
    fn drop(&mut self) {
        drop(self.0.stdin.take());
        let _ = self.0.wait();
    }
}

impl Fixture {
    fn start() -> (Self, PathBuf, u16) {
        let python = std::env::var_os("ORBIT_FIXTURE_PYTHON").unwrap_or_else(|| "python".into());
        let mut child = Command::new(python)
            .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/mcp_ssh.py"))
            .stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().expect("start SSH fixture");
        let mut announce = String::new();
        StdReader::new(child.stdout.take().unwrap()).read_line(&mut announce).unwrap();
        let fixture = Self(child);
        let ready: Value = serde_json::from_str(&announce).expect("fixture requires Python and Paramiko");
        (fixture, PathBuf::from(ready["root"].as_str().unwrap()), ready["port"].as_u64().unwrap() as u16)
    }
}

struct Client<'a> {
    port: u16,
    token: &'a str,
    session: String,
}

impl<'a> Client<'a> {
    async fn request(&self, method: &str, extra: &str, payload: Option<Value>) -> (u16, String, Option<Value>) {
        timeout(Duration::from_secs(12), async {
            let mut conn = TcpStream::connect(("127.0.0.1", self.port)).await.unwrap();
            let host = format!("127.0.0.1:{}", self.port);
            let body = payload.map(|json| json.to_string()).unwrap_or_default();
            let auth = if self.token.is_empty() { Zeroizing::new(String::new()) }
                else { Zeroizing::new(format!("Authorization: Bearer {}\r\n", self.token)) };
            let session = if self.session.is_empty() { String::new() }
                else { format!("MCP-Session-Id: {}\r\nMCP-Protocol-Version: 2025-11-25\r\n", self.session) };
            let request = Zeroizing::new(format!("{method} /mcp HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nAccept: application/json, text/event-stream\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{}{session}{extra}\r\n{body}", body.len(), auth.as_str()));
            conn.write_all(request.as_bytes()).await.unwrap();
            let mut response = BufReader::new(conn).lines();
            let status = response.next_line().await.unwrap().unwrap().split_whitespace().nth(1).unwrap().parse::<u16>().unwrap();
            let mut id = String::new();
            while let Some(header) = response.next_line().await.unwrap() {
                if header.is_empty() { break; }
                if let Some((name, value)) = header.split_once(':') && name.eq_ignore_ascii_case("mcp-session-id") { id = value.trim().into(); }
            }
            if status != 200 || method != "POST" { return (status, id, None); }
            while let Some(line) = response.next_line().await.unwrap() {
                let value = line.strip_prefix("data:").unwrap_or(&line).trim();
                if let Ok(value) = serde_json::from_str::<Value>(value) && value.get("id").is_some() {
                    return (status, id, Some(value));
                }
            }
            (status, id, None)
        }).await.expect("MCP HTTP request timed out")
    }

    async fn connect(port: u16, token: &'a str, name: &str) -> Self {
        let mut client = Self { port, token, session: String::new() };
        let (status, id, reply) = client.request("POST", "", Some(json!({"jsonrpc":"2.0","id":7,"method":"initialize","params":{
            "protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":name,"version":"test"}}}))).await;
        assert_eq!(status, 200);
        assert!(!id.is_empty());
        assert!(reply.unwrap()["result"]["capabilities"]["tools"].is_object());
        client.session = id;
        let (status, _, _) = client.request("POST", "", Some(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))).await;
        assert_eq!(status, 202);
        client
    }

    async fn rpc(&self, method: &str, params: Value) -> Value {
        let (status, _, reply) = self.request("POST", "", Some(json!({"jsonrpc":"2.0","id":8,"method":method,"params":params}))).await;
        assert_eq!(status, 200, "{method}");
        reply.expect("MCP RPC body")
    }

    async fn call(&self, tool: &str, arguments: Value) -> Result<Value, Value> {
        let reply = self.rpc("tools/call", json!({"name":tool,"arguments":arguments})).await;
        if let Some(error) = reply.get("error") { return Err(error.clone()); }
        let result = &reply["result"];
        if result["isError"].as_bool() == Some(true) { return Err(result.clone()); }
        if let Some(content) = result.get("structuredContent") { return Ok(content.clone()); }
        Ok(serde_json::from_str(result["content"][0]["text"].as_str().expect("MCP text result")).expect("MCP tool JSON"))
    }

    async fn transfer(&self, tool: &str, params: Value) -> Value {
        let started = self.call(tool, params).await.unwrap();
        let id = started["transferId"].as_str().unwrap();
        for _ in 0..40 {
            let task = self.call("get_file_transfer", json!({"transfer_id":id})).await.unwrap();
            if task["finished"] == true { return task; }
            sleep(Duration::from_millis(70)).await;
        }
        panic!("SFTP fixture task failed to finish");
    }
}

#[test]
#[ignore = "Requires local Python with Paramiko. Uses loopback and isolated temporary files."]
fn mcp_loopback() {
    let (_fixture, root, port) = Fixture::start();
    let backend = Backend::open_in(root.clone()).unwrap();
    let profile = ConnectionProfile { id: uuid::Uuid::new_v4().to_string(), name: "MCP fixture".into(),
        group: "Test".into(), hostname: "127.0.0.1".into(), port, username: "fixture".into(),
        auth_method: "password".into(), private_key_path: None };
    backend.save_profile(&profile, None).unwrap();

    backend.runtime_handle().block_on(async {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let workspace = Arc::new(AiWorkspace::open(Some(backend.history_path())));
        let (desktop, mut request) = mpsc::channel(8);
        let server = serve(listener, backend.mcp_catalog(), Arc::new(TransferQueue::default()), desktop, workspace.clone()).unwrap();
        let client = Client::connect(port, server.token(), "Smoke A").await;
        let other = Client::connect(port, server.token(), "Smoke B").await;
        let denied = Client { port, token: "", session: String::new() };
        assert_eq!(denied.request("POST", "", Some(json!({}))).await.0, 401);
        assert_ne!(client.request("POST", "Origin: https://example.invalid\r\n", Some(json!({}))).await.0, 200);

        let indexed = client.rpc("tools/list", json!({})).await;
        let tools = indexed["result"]["tools"].as_array().unwrap();
        let names: Vec<_> = tools.iter().map(|tool| tool["name"].as_str().unwrap()).collect();
        for tool in ["list_connections", "list_sessions", "open_connection", "take_over_session", "open_dedicated_terminal", "release_session",
            "send_input", "read_output", "execute_command", "disconnect", "upload_file", "download_file", "get_file_transfer", "list_file_transfers"] {
            assert!(names.contains(&tool), "missing {tool}");
        }
        for (name, required) in [
            ("take_over_session", &["session_id"][..]),
            ("release_session", &["session_id", "terminal_id"][..]),
            ("read_output", &["session_id", "terminal_id", "mode"][..]),
            ("execute_command", &["session_id", "terminal_id", "mode", "command"][..]),
            ("send_input", &["session_id", "terminal_id", "mode", "data", "append_newline"][..]),
        ] {
            let schema = &tools[names.iter().position(|tool| *tool == name).unwrap()]["inputSchema"];
            let fields = schema["required"].as_array().unwrap();
            assert_eq!(fields.len(), required.len(), "{name}");
            for field in required {
                assert!(fields.contains(&json!(field)), "{name} requires {field}");
            }
        }
        let found = client.call("list_connections", json!({})).await.unwrap();
        assert_eq!(found["connections"][0]["id"], profile.id);
        assert!(found["connections"][0].get("credential").is_none());
        let opened = client.call("open_connection", json!({"connection_id":profile.id})).await.unwrap();
        assert_eq!(opened["state"], "confirmation_required");
        let login = request.recv().await.unwrap();
        assert!(matches!(login, DesktopRequest::OpenConnection { .. }));
        assert_eq!(client.call("list_sessions", json!({})).await.unwrap()["sessions"], json!([]));

        let reject = Arc::new(AtomicBool::new(false));
        let reject_next = reject.clone();
        let desktop = tokio::spawn(async move {
            while let Some(item) = request.recv().await {
                if let DesktopRequest::Approval(approval) = item {
                    approval.resolve(!reject_next.swap(false, Ordering::AcqRel));
                }
            }
        });
        let (terminal, mut events) = backend.connect(profile.clone(), false, Zeroizing::new("fixtures-only".into()), false, false, (120, 36));
        let live = terminal.live.clone();
        let ssh = tokio::spawn(async move {
            while let Some(event) = events.recv().await {
                match event {
                    SessionEvent::HostKey { .. } => live.trust_host(true).unwrap(),
                    SessionEvent::Data { data } => { let _ = live.commands.send(SessionCommand::Acknowledge(data.len())).await; }
                    SessionEvent::Error { message, .. } => panic!("fixture SSH error: {message}"),
                    _ => {}
                }
            }
        });
        let id = terminal.id.as_str();
        timeout(Duration::from_secs(10), async {
            while !terminal.live.connected.load(Ordering::Acquire) {
                sleep(Duration::from_millis(20)).await;
            }
        }).await.expect("fixture authenticated and opened a PTY");
        assert_eq!(client.call("open_connection", json!({"connection_id":profile.id})).await.unwrap()["sessionId"], id);

        reject.store(true, Ordering::Release);
        assert!(client.call("take_over_session", json!({"session_id":id})).await.is_err());
        assert!(terminal.control.binding().is_none());
        assert!(client.call("take_over_session", json!({})).await.is_err());
        let bound = client.call("take_over_session", json!({"session_id":id})).await.unwrap();
        let binding = bound["terminalId"].as_str().unwrap();
        let read_args = &bound["nextRead"]["arguments"];
        assert_eq!(bound["sent"], false);
        assert_eq!(bound["sharesOriginalTerminal"], true);
        assert!(other.call("take_over_session", json!({"session_id":id})).await.is_err());
        assert!(other.call("read_output", read_args.clone()).await.is_err());
        assert!(client.call("read_output", json!({"session_id":id,"terminal_id":binding})).await.is_err());
        assert!(client.call("read_output", json!({"session_id":id,"terminal_id":binding,"mode":"dedicated"})).await.is_err());
        reject.store(true, Ordering::Release);
        assert!(client.call("execute_command", json!({"session_id":id,"terminal_id":binding,"mode":"takeover","command":"printf fixture-pty"})).await.is_err());
        let empty = client.call("execute_command", json!({})).await.unwrap_err();
        assert!(empty["content"][0]["text"].as_str().unwrap().contains("nextRead.arguments"));
        let buffered = client.call("send_input", json!({"session_id":id,"terminal_id":binding,"mode":"takeover","data":"printf fixture-pty","append_newline":false})).await.unwrap();
        assert_eq!(buffered["sent"], false);
        assert_eq!(buffered["buffered"], true);
        let submit = json!({"session_id":id,"terminal_id":binding,"mode":"takeover","data":"","append_newline":true});
        let sent = client.call("send_input", submit).await.unwrap();
        assert_eq!(sent["sent"], true);
        timeout(Duration::from_secs(3), async {
            loop {
                let read = client.call("read_output", read_args.clone()).await.unwrap();
                let out = read["output"].as_str().unwrap();
                assert_eq!(read["completionKnown"], false);
                if out.contains("fixture-pty") {
                    assert!(out.contains("fixture-terminal-1"));
                    break;
                }
                sleep(Duration::from_millis(30)).await;
            }
        }).await.unwrap();
        assert_eq!(client.call("release_session", bound["nextRelease"]["arguments"].clone()).await.unwrap()["released"], true);
        assert!(terminal.control.binding().is_none());
        assert!(client.call("read_output", json!({"session_id":id,"terminal_id":binding,"mode":"takeover"})).await.is_err());

        let opened = client.call("open_dedicated_terminal", json!({"session_id":id})).await.unwrap();
        let dedicated = opened["terminalId"].as_str().unwrap();
        let dir = client.call("execute_command", json!({"session_id":id,"terminal_id":dedicated,"mode":"dedicated","command":"cd /fixture"})).await.unwrap();
        assert_eq!(dir["exitCode"], 0);
        assert_eq!(dir["sharesOriginalTerminal"], false);
        let pwd_args = json!({"session_id":id,"terminal_id":dedicated,"mode":"dedicated","command":"pwd"});
        let pwd = client.call("execute_command", pwd_args).await.unwrap();
        assert!(pwd["stdout"].as_str().unwrap().contains("/fixture"));
        assert_eq!(client.call("execute_command", json!({"session_id":id,"terminal_id":dedicated,"mode":"dedicated","command":"false"})).await.unwrap()["exitCode"], 1);
        let running = client.call("execute_command", json!({"session_id":id,"terminal_id":dedicated,"mode":"dedicated","command":"sleep 2","timeout_seconds":1})).await.unwrap();
        assert_eq!(running["running"], true);
        timeout(Duration::from_secs(4), async {
            loop {
                let finished = client.call("read_output", opened["nextRead"]["arguments"].clone()).await.unwrap();
                if finished["completionKnown"] == true {
                    assert!(finished["output"].as_str().unwrap().contains("fixture-finished"));
                    break;
                }
                sleep(Duration::from_millis(40)).await;
            }
        }).await.unwrap();
        assert!(workspace.executions().iter().any(|record| record.command == "pwd"));
        timeout(Duration::from_secs(4), async {
            while !orbitshell_ssh::command_history::read(&backend.history_path()).unwrap().iter().any(|record| record.command == "pwd" && record.source == "ai") {
                sleep(Duration::from_millis(30)).await;
            }
        }).await.expect("command history is persisted");

        let upload = root.join("upload.txt");
        let download = root.join("download.txt");
        std::fs::write(&upload, b"OrbitShell transfer fixture\r\n").unwrap();
        let args = json!({"session_id":id,"local_path":upload,"remote_directory":"/"});
        let uploaded = client.transfer("upload_file", args.clone()).await;
        assert_eq!(uploaded["completed"], true);
        assert_eq!(uploaded["transfer"]["agentName"], "Smoke A");
        assert!(other.call("get_file_transfer", json!({"transfer_id":uploaded["transferId"]})).await.is_err());
        assert_eq!(client.transfer("upload_file", args).await["completed"], false);
        let args = json!({"session_id":id,"local_path":download,"remote_path":"/upload.txt"});
        assert_eq!(client.transfer("download_file", args.clone()).await["completed"], true);
        assert_eq!(std::fs::read(&download).unwrap(), std::fs::read(&upload).unwrap());
        assert_eq!(client.transfer("download_file", args).await["completed"], false);
        assert_eq!(client.call("list_file_transfers", json!({})).await.unwrap()["transfers"].as_array().unwrap().len(), 4);

        assert!(other.call("take_over_session", json!({"session_id":id})).await.is_ok());
        let (deleted, _, _) = other.request("DELETE", "", None).await;
        assert!((200..300).contains(&deleted));
        timeout(Duration::from_secs(3), async {
            while terminal.control.binding().is_some() { sleep(Duration::from_millis(20)).await; }
        }).await.expect("MCP disconnect releases original PTY");
        assert_eq!(client.call("disconnect", json!({"session_id":id})).await.unwrap()["disconnected"], true);
        assert!(terminal.live.cancel.is_cancelled());
        desktop.abort();
        ssh.abort();
    });
}

#[test]
#[ignore = "Opens an isolated GPUI window briefly for a desktop smoke test."]
fn mcp_desktop_startup() {
    use gpui::{Application, Bounds, TitlebarOptions, WindowBounds, WindowOptions, prelude::*, px, size};

    let (_fixture, root, _) = Fixture::start();
    let backend = Backend::open_in(root).unwrap();
    let runtime = backend.runtime_handle();
    let passed = Arc::new(AtomicBool::new(false));
    let received = passed.clone();
    Application::new().with_assets(crate::WorkbenchAssets).run(move |cx| {
        let bounds = Bounds::centered(None, size(px(1030.), px(680.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions { title: Some("OrbitShell MCP Smoke".into()), appears_transparent: true, ..TitlebarOptions::default() }),
            ..WindowOptions::default()
        };
        cx.open_window(options, move |window, cx| cx.new(|cx| {
            let mut view = crate::Workbench::with_backend(Ok(backend), window, cx);
            view.show_mcp_smoke(window, cx);
            cx.spawn_in(window, async move |view, cx| {
                let mut state = None;
                for _ in 0..50 {
                    state = view.update_in(cx, |view, _, _| view.mcp_smoke_state()).ok().flatten();
                    if state.is_some() { break; }
                    cx.background_executor().timer(Duration::from_millis(100)).await;
                }
                if let Some((port, token)) = state {
                    let proof = runtime.spawn(async move {
                        let client = Client::connect(port, &token, "GPUI Smoke").await;
                        assert_eq!(client.call("list_sessions", json!({})).await.unwrap()["sessions"], json!([]));
                    });
                    if proof.await.is_ok() { received.store(true, Ordering::Release); }
                }
                cx.background_executor().timer(Duration::from_secs(24)).await;
                let _ = view.update_in(cx, |_, _, cx| cx.quit());
            }).detach();
            view
        })).expect("open isolated GPUI workbench");
    });
    assert!(passed.load(Ordering::Acquire), "GPUI startup path opened an authenticated MCP endpoint");
}
