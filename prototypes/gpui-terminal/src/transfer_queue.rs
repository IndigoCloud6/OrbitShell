use std::{
    sync::{Arc, Mutex},
    time::Instant,
};

use orbitshell_ssh::transfer::TransferProgress;
use serde::Serialize;
use tokio::sync::{oneshot, watch};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Direction {
    Upload,
    Download,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TransferStatus {
    Running,
    Done,
    Cancelled,
    Error,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTransfer {
    pub agent_id: String,
    pub agent_name: String,
    pub local_path: String,
    pub remote_path: String,
    pub session_id: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Transfer {
    pub id: uuid::Uuid,
    pub direction: Direction,
    pub status: TransferStatus,
    pub name: String,
    pub session: String,
    pub temporary: bool,
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentTransfer>,
    pub message: String,
    pub bytes: u64,
    pub total: Option<u64>,
    pub bytes_per_second: u64,
}

pub struct TransferJob {
    pub cancel: CancellationToken,
    start: Instant,
    data: Mutex<Transfer>,
    changed: watch::Sender<()>,
}

impl TransferJob {
    pub fn snapshot(&self) -> Transfer {
        self.data
            .lock()
            .unwrap_or_else(|lock| lock.into_inner())
            .clone()
    }

    fn update(&self, update: impl FnOnce(&mut Transfer)) {
        let mut data = self.data.lock().unwrap_or_else(|lock| lock.into_inner());
        if data.status == TransferStatus::Running {
            update(&mut data);
            self.changed.send_replace(());
        }
    }

    pub fn progress(&self, event: TransferProgress) {
        self.update(|data| {
            data.name = event.name;
            data.bytes = event.bytes;
            data.total = event.total;
            data.bytes_per_second =
                (event.bytes as f64 / self.start.elapsed().as_secs_f64().max(0.1)) as u64;
            data.message = "传输中".into();
        });
    }

    pub fn message(&self, message: &str) {
        self.update(|data| data.message = message.into());
    }

    pub fn finish(&self, result: Result<Option<u64>, String>, disconnected: bool) {
        self.update(|data| {
            data.bytes_per_second = 0;
            match result {
                Ok(Some(bytes)) => {
                    data.status = TransferStatus::Done;
                    data.bytes = bytes;
                    data.total = Some(bytes);
                    data.message = "已完成".into();
                }
                Ok(None) => {
                    data.status = TransferStatus::Cancelled;
                    data.message = "已取消".into();
                }
                Err(_) if self.cancel.is_cancelled() || disconnected => {
                    data.status = TransferStatus::Cancelled;
                    data.message = if disconnected {
                        "SSH 会话已断开"
                    } else {
                        "已取消传输"
                    }
                    .into();
                }
                Err(error) => {
                    data.status = TransferStatus::Error;
                    data.message = error;
                }
            }
        });
    }
}

pub struct UploadConflict {
    pub job: Arc<TransferJob>,
    pub remote_path: String,
    pub answer: oneshot::Sender<bool>,
}

pub struct TransferQueue {
    jobs: Mutex<Vec<Arc<TransferJob>>>,
    changed: watch::Sender<()>,
}

impl Default for TransferQueue {
    fn default() -> Self {
        Self {
            jobs: Mutex::default(),
            changed: watch::channel(()).0,
        }
    }
}

impl TransferQueue {
    pub fn subscribe(&self) -> watch::Receiver<()> {
        self.changed.subscribe()
    }

    pub fn start(
        &self,
        direction: Direction,
        name: String,
        session: String,
        temporary: bool,
    ) -> Result<Arc<TransferJob>, &'static str> {
        self.start_owned(direction, name, session, temporary, None)
    }

    pub fn start_owned(
        &self,
        direction: Direction,
        name: String,
        session: String,
        temporary: bool,
        agent: Option<AgentTransfer>,
    ) -> Result<Arc<TransferJob>, &'static str> {
        let mut jobs = self.jobs.lock().map_err(|_| "传输队列暂不可用")?;
        if jobs
            .iter()
            .filter(|job| job.snapshot().status == TransferStatus::Running)
            .count()
            >= 32
        {
            return Err("同时传输的任务已达到上限");
        }
        if jobs.len() >= 100
            && let Some(oldest) = jobs
                .iter()
                .position(|job| job.snapshot().status != TransferStatus::Running)
        {
            jobs.remove(oldest);
        }
        let job = Arc::new(TransferJob {
            cancel: CancellationToken::new(),
            start: Instant::now(),
            changed: self.changed.clone(),
            data: Mutex::new(Transfer {
                id: uuid::Uuid::new_v4(),
                direction,
                name,
                session,
                temporary,
                agent,
                status: TransferStatus::Running,
                message: "准备传输".into(),
                bytes: 0,
                total: None,
                bytes_per_second: 0,
            }),
        });
        jobs.push(job.clone());
        self.changed.send_replace(());
        Ok(job)
    }

    pub fn snapshots(&self) -> Vec<Transfer> {
        self.jobs
            .lock()
            .unwrap_or_else(|lock| lock.into_inner())
            .iter()
            .rev()
            .map(|job| job.snapshot())
            .collect()
    }

    pub fn clear_finished(&self) {
        if let Ok(mut jobs) = self.jobs.lock() {
            jobs.retain(|job| job.snapshot().status == TransferStatus::Running);
            self.changed.send_replace(());
        }
    }

    pub fn cancel(&self, id: uuid::Uuid) {
        if let Ok(jobs) = self.jobs.lock()
            && let Some(job) = jobs.iter().find(|job| {
                let item = job.snapshot();
                item.id == id && item.status == TransferStatus::Running
            })
        {
            job.cancel.cancel();
        }
    }
}
