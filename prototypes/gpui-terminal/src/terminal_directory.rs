use tokio::sync::oneshot;
use uuid::Uuid;

#[derive(Clone, Copy, Default)]
pub(super) enum PromptInput {
    #[default]
    Ready,
    Editing {
        saw_draft: bool,
    },
    Running {
        saw_line_feed: bool,
    },
}

impl PromptInput {
    pub(super) fn sent(&mut self, bytes: &[u8]) {
        let bracketed = bytes.starts_with(b"\x1b[200~") && bytes.ends_with(b"\x1b[201~");
        *self = if !bracketed && bytes.iter().any(|byte| matches!(byte, b'\r' | b'\n' | 3)) {
            Self::Running {
                saw_line_feed: false,
            }
        } else if matches!(self, Self::Running { .. }) {
            return;
        } else {
            Self::Editing {
                saw_draft: matches!(self, Self::Editing { saw_draft: true }),
            }
        };
    }

    pub(super) fn received(&mut self, bytes: &[u8], at_prompt: bool) {
        match self {
            Self::Ready => {}
            Self::Editing { saw_draft } => {
                if at_prompt && *saw_draft {
                    *self = Self::Ready;
                } else if !at_prompt {
                    *saw_draft = true;
                }
            }
            Self::Running { saw_line_feed } => {
                *saw_line_feed |= bytes.contains(&b'\n');
                if *saw_line_feed && at_prompt {
                    *self = Self::Ready;
                }
            }
        }
    }
}

pub(super) struct DirectoryQuery {
    pub id: Uuid,
    pub sender: oneshot::Sender<Result<String, &'static str>>,
    prefix: Vec<u8>,
    matched: usize,
    directory: Vec<u8>,
}

impl DirectoryQuery {
    pub fn start() -> (
        Self,
        Vec<u8>,
        oneshot::Receiver<Result<String, &'static str>>,
    ) {
        let id = Uuid::new_v4();
        let nonce = id.simple();
        let (sender, receiver) = oneshot::channel();
        let prefix = format!("\x1b]633;OrbitShellCwd;{nonce};").into_bytes();
        let command = format!(
            " command printf '\\033]633;OrbitShellCwd;{nonce};%s\\007' \"$(command pwd -P)\"\r"
        )
        .into_bytes();
        (
            Self {
                id,
                sender,
                prefix,
                matched: 0,
                directory: Vec::new(),
            },
            command,
            receiver,
        )
    }

    pub fn receive(&mut self, bytes: &[u8]) -> Option<Result<String, &'static str>> {
        const INVALID: &str = "终端未返回有效的绝对目录";
        for &byte in bytes {
            if self.matched < self.prefix.len() {
                self.matched = if byte == self.prefix[self.matched] {
                    self.matched + 1
                } else {
                    usize::from(byte == self.prefix[0])
                };
            } else if byte == 7 {
                let path = std::str::from_utf8(&self.directory)
                    .ok()
                    .filter(|path| path.starts_with('/') && !path.chars().any(char::is_control));
                return Some(path.map(str::to_owned).ok_or(INVALID));
            } else if self.directory.len() == 4096 || byte < 32 || byte == 127 {
                return Some(Err(INVALID));
            } else {
                self.directory.push(byte);
            }
        }
        None
    }
}
