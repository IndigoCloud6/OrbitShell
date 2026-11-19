use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::Path,
};
use zeroize::Zeroizing;

pub mod ai_shell;
pub mod channel_diagnostics;
pub mod command_guard;
pub mod credentials;
pub mod command_history;
#[cfg(windows)]
pub mod file_drag;
pub mod launch;
mod profile;
pub mod quick_command;
pub mod remote_editor;
pub mod server_metrics;
pub mod session;
pub mod sftp;
pub mod transfer;

pub use profile::ConnectionProfile;

pub fn lock_workspace(path: &Path) -> std::io::Result<File> {
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path.with_extension("lock"))?;
    lock.lock()?;
    Ok(lock)
}

pub fn write_workspace<T: serde::Serialize>(path: &Path, value: &T) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("存储目录无效"))?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(&Zeroizing::new(serde_json::to_vec_pretty(value)?))?;
    file.as_file().sync_all()?;
    file.persist(path)?;
    Ok(())
}
