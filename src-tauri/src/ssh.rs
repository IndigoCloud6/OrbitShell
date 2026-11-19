use std::sync::Arc;

use anyhow::{anyhow, bail, Result};
use orbitshell_ssh::session::{self, SessionEvents as EventSink, SessionHooks};
use tauri::{ipc::Channel, AppHandle, Emitter, Manager};
use tokio::sync::mpsc;

use crate::{storage::ConnectionProfile, AppState};

pub use orbitshell_ssh::session::{
    ConnectRequest, LiveSession, OutputBuffer, SessionCommand, SessionEvent, SshHandler,
};

#[derive(Clone)]
pub struct SessionEvents {
    sink: EventSink,
    broadcast: Option<(AppHandle, String)>,
}

impl SessionEvents {
    pub fn channel(channel: Channel<SessionEvent>) -> Self {
        Self {
            sink: EventSink::new(move |event| Ok(channel.send(event)?)),
            broadcast: None,
        }
    }

    pub fn broadcast(app: AppHandle, session_id: String) -> Self {
        let (handle, id) = (app.clone(), session_id.clone());
        let sink = EventSink::new(move |event| {
            handle.emit_to(
                "main",
                "orbitshell:session-event",
                serde_json::json!({"sessionId": id, "event": event}),
            )?;
            Ok(())
        });
        Self {
            sink,
            broadcast: Some((app, session_id)),
        }
    }

    pub fn announce(&self, profile: &ConnectionProfile) -> Result<()> {
        if let Some((app, session_id)) = &self.broadcast {
            app.emit_to(
                "main",
                "orbitshell:session-opened",
                serde_json::json!({"id": session_id, "profile": profile}),
            )?;
        }
        Ok(())
    }
}

struct TauriHooks(AppHandle);

impl SessionHooks for TauriHooks {
    fn known_host(&self, address: &str) -> Result<Option<String>> {
        let state = self.0.state::<AppState>();
        let store = state.storage.lock().map_err(|_| anyhow!("配置暂不可用"))?;
        Ok(store.known_hosts.get(address).cloned())
    }

    fn save_host(&self, address: String, fingerprint: String) -> Result<()> {
        let state = self.0.state::<AppState>();
        let mut store = state.storage.lock().map_err(|_| anyhow!("配置暂不可用"))?;
        if store
            .known_hosts
            .get(&address)
            .is_some_and(|saved| saved != &fingerprint)
        {
            bail!("另一会话保存了不同的服务器指纹，请重新核实");
        }
        let mut next = store.clone();
        next.known_hosts.insert(address, fingerprint);
        state.storage.persist(&next)?;
        *store = next;
        Ok(())
    }

    fn remember_credential(&self, profile: &ConnectionProfile, secret: &str) -> Result<(), String> {
        crate::remember_session_credential(&self.0, profile, secret)
    }

    fn connected(&self, id: &str, live: &LiveSession) {
        self.0.state::<AppState>().history.register(id, live);
        crate::recent::record_connected(&self.0, live, id);
    }

    fn allow_input(&self, id: &str) -> bool {
        !self.0.state::<AppState>().ai.taken_over(id)
    }

    fn owns_input(&self, owner: &str, id: &str) -> bool {
        self.0.state::<AppState>().ai.owns(owner, id)
    }

    fn record_input(&self, id: &str, entry_id: String, command: &str) {
        self.0
            .state::<AppState>()
            .history
            .record(&self.0, id, entry_id, command, None);
    }

    fn output(&self, id: &str) {
        self.0.state::<AppState>().ai.source_output(&self.0, id);
    }

    fn finished(&self, id: &str) {
        let state = self.0.state::<AppState>();
        if let Ok(mut sessions) = state.sessions.lock() {
            sessions.remove(id);
        };
    }
}

pub async fn run_session(
    app: AppHandle,
    live: Arc<LiveSession>,
    request: ConnectRequest,
    events: SessionEvents,
    commands: mpsc::Receiver<SessionCommand>,
) {
    session::run_session(
        Arc::new(TauriHooks(app)),
        live,
        request,
        events.sink,
        commands,
    )
    .await;
}
