use crate::session::LiveSession;
use anyhow::{anyhow, bail, Context, Result};
use russh_sftp::protocol::{FileAttributes, OpenFlags};
use serde::Serialize;
use std::{
    future::Future,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferProgress {
    pub name: String,
    pub bytes: u64,
    pub total: Option<u64>,
}

pub struct Outcome {
    pub name: String,
    pub bytes: u64,
    pub local_path: PathBuf,
    pub remote_path: String,
}

// Only a definite server-side exclusive-create rejection permits a desktop
// collision re-check. A timeout or failed write must never trigger an automatic retry.
#[derive(Debug)]
pub struct UploadCreateRejected;
impl std::fmt::Display for UploadCreateRejected {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("服务器拒绝排他创建上传目标")
    }
}
impl std::error::Error for UploadCreateRejected {}

struct CreatedUpload {
    sftp: std::sync::Arc<russh_sftp::client::SftpSession>,
    path: String,
    armed: bool,
}
impl Drop for CreatedUpload {
    fn drop(&mut self) {
        if self.armed {
            let sftp = self.sftp.clone();
            let path = self.path.clone();
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    let _ = timeout(Duration::from_secs(5), sftp.remove_file(path)).await;
                });
            }
        }
    }
}

async fn connected<T>(
    live: &LiveSession,
    cancel: &CancellationToken,
    action: impl Future<Output = Result<T>>,
) -> Result<T> {
    tokio::select! {
        biased;
        _ = live.cancel.cancelled() => Err(anyhow!("连接已关闭，传输已停止")),
        _ = cancel.cancelled() => Err(anyhow!("传输已取消")),
        result = action => result,
    }
}

async fn copy<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    source: &mut R,
    destination: &mut W,
    name: &str,
    total: Option<u64>,
    report: &impl Fn(TransferProgress),
) -> Result<u64> {
    let mut buffer = vec![0u8; 64 * 1024];
    let mut bytes = 0u64;
    let mut last = Instant::now();
    let progress = |bytes| {
        report(TransferProgress {
            name: name.into(),
            bytes,
            total,
        })
    };
    progress(0);
    loop {
        let count = timeout(Duration::from_secs(60), source.read(&mut buffer))
            .await
            .context("等待文件数据超时")??;
        if count == 0 {
            break;
        }
        timeout(
            Duration::from_secs(60),
            destination.write_all(&buffer[..count]),
        )
        .await
        .context("写入文件超时")??;
        bytes += count as u64;
        if last.elapsed() >= Duration::from_millis(100) {
            progress(bytes);
            last = Instant::now();
        }
    }
    if total.is_some_and(|total| total != bytes) {
        bail!("文件大小在传输过程中发生变化，请确认文件稳定后重试");
    }
    progress(bytes);
    Ok(bytes)
}

pub async fn download(
    live: &LiveSession,
    remote: &str,
    destination: &Path,
    overwrite: bool,
    report: impl Fn(TransferProgress),
    cancel: &CancellationToken,
) -> Result<Outcome> {
    let name = remote
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty() && *name != "." && *name != "..")
        .ok_or_else(|| anyhow!("远程文件路径无效"))?
        .to_owned();
    connected(live, cancel, async {
        let sftp = live.sftp().await?;
        let before = timeout(Duration::from_secs(60), sftp.metadata(remote))
            .await
            .context("读取远程文件信息超时")??;
        if before
            .permissions
            .is_some_and(|mode| mode & 0o170000 != 0 && mode & 0o170000 != 0o100000)
        {
            bail!("只支持下载普通文件，不支持目录、设备或管道");
        }
        let mut source = timeout(Duration::from_secs(60), sftp.open(remote))
            .await
            .context("打开远程文件超时")??;
        let metadata = timeout(Duration::from_secs(60), source.metadata())
            .await
            .context("读取远程文件信息超时")??;
        if metadata
            .permissions
            .is_some_and(|mode| mode & 0o170000 != 0 && mode & 0o170000 != 0o100000)
        {
            bail!("只支持下载普通文件，不支持目录、设备或管道");
        }
        let parent = destination
            .parent()
            .ok_or_else(|| anyhow!("本机保存目录无效"))?;
        let temporary = tempfile::NamedTempFile::new_in(parent)?;
        let mut output = tokio::fs::File::from_std(temporary.reopen()?);
        let bytes = copy(&mut source, &mut output, &name, metadata.size, &report).await?;
        output.flush().await?;
        output.sync_all().await?;
        drop(output);
        timeout(Duration::from_secs(60), source.close())
            .await
            .context("关闭远程文件超时")??;
        if live.cancel.is_cancelled() || cancel.is_cancelled() {
            bail!("传输已停止，未提交下载文件");
        }
        // The desktop save dialog may approve replacement; MCP always uses no-clobber.
        if overwrite {
            temporary
                .persist(destination)
                .map_err(|error| error.error)?;
        } else {
            temporary
                .persist_noclobber(destination)
                .map_err(|error| error.error)
                .context("无法提交下载文件；目标可能已存在，未覆盖原文件")?;
        }
        Ok(Outcome {
            name,
            bytes,
            local_path: destination.into(),
            remote_path: remote.into(),
        })
    })
    .await
}

pub async fn upload(
    live: &LiveSession,
    source: &Path,
    directory: &str,
    filename: &str,
    permissions: Option<u32>,
    report: impl Fn(TransferProgress),
    cancel: &CancellationToken,
) -> Result<Outcome> {
    if filename.is_empty()
        || matches!(filename, "." | "..")
        || filename
            .chars()
            .any(|c| c.is_control() || matches!(c, '/' | '\\'))
    {
        bail!("本机文件名无效");
    }
    let name = filename.to_owned();
    let (sftp, path, mut input, total) = connected(live, cancel, async {
        let sftp = live.sftp().await?;
        let directory = timeout(Duration::from_secs(60), sftp.canonicalize(directory))
            .await
            .context("读取远程目录超时")??;
        let mut path = directory.trim_end_matches('/').to_owned();
        path.push('/');
        path.push_str(&name);
        if !tokio::fs::metadata(source).await?.is_file() {
            bail!("只支持上传本机普通文件");
        }
        let input = tokio::fs::File::open(source).await?;
        let metadata = input.metadata().await?;
        if !metadata.is_file() {
            bail!("只支持上传本机普通文件");
        }
        Ok((sftp, path, input, metadata.len()))
    })
    .await?;
    // Never truncate or replace an existing remote file, including a symlink.
    let mut output = connected(live, cancel, async {
        Ok(timeout(
            Duration::from_secs(60),
            sftp.open_with_flags_and_attributes(
                &path,
                OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::EXCLUDE,
                FileAttributes {
                    permissions,
                    ..FileAttributes::empty()
                },
            ),
        )
        .await
        .context("创建远程文件超时，结果未确认，请先核对目标文件")?
        .map_err(|error| anyhow::Error::new(UploadCreateRejected).context(error))
        .context("无法创建远程文件；若存在同名文件，请先重命名")?)
    })
    .await?;
    let mut cleanup = CreatedUpload {
        sftp: sftp.clone(),
        path: path.clone(),
        armed: true,
    };
    let copied = connected(live, cancel, async {
        let bytes = copy(&mut input, &mut output, &name, Some(total), &report).await?;
        timeout(Duration::from_secs(60), output.flush())
            .await
            .context("刷新远程文件超时")??;
        Ok(bytes)
    })
    .await;
    // close consumes the SFTP file. Close exactly once after releasing the copy borrow.
    let close_wait = Duration::from_secs(if copied.is_ok() { 60 } else { 5 });
    let closed = connected(live, cancel, async {
        timeout(close_wait, output.close())
            .await
            .context("关闭远程文件超时")??;
        Ok(())
    })
    .await;
    let result = copied.and_then(|bytes| closed.map(|()| bytes));
    match result {
        Ok(bytes) => {
            cleanup.armed = false;
            Ok(Outcome {
                name,
                bytes,
                local_path: source.into(),
                remote_path: path,
            })
        }
        Err(error) => {
            // Only clean up the new file created by this attempt, never an existing destination.
            let _ = timeout(Duration::from_secs(5), sftp.remove_file(path)).await;
            cleanup.armed = false;
            Err(error)
        }
    }
}

use russh::{client, ChannelMsg};
use russh_sftp::{
    client::{RawSftpSession, SftpSession},
    protocol::{Packet, StatusCode},
};
use std::sync::Arc;
use uuid::Uuid;
struct RemoteTemporary {
    sftp: Arc<SftpSession>,
    path: String,
    armed: bool,
}
impl Drop for RemoteTemporary {
    fn drop(&mut self) {
        if self.armed {
            let sftp = self.sftp.clone();
            let path = self.path.clone();
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    let _ = timeout(Duration::from_secs(5), sftp.remove_file(path)).await;
                });
            }
        }
    }
}

async fn rename_channel(
    live: &LiveSession,
    connection: &client::Handle<crate::session::SshHandler>,
) -> Result<RawSftpSession> {
    let mut channel = live.open_file_channel(connection).await?;
    live.request_sftp_subsystem(&channel).await?;
    loop {
        match channel.wait().await {
            Some(ChannelMsg::Success) => break,
            Some(ChannelMsg::Failure | ChannelMsg::Close) | None => {
                bail!("服务器未接受文件替换通道")
            }
            _ => {}
        }
    }
    let raw = RawSftpSession::new(live.sftp_stream(channel));
    let version = raw.init().await?;
    if version
        .extensions
        .get("posix-rename@openssh.com")
        .map(String::as_str)
        != Some("1")
    {
        let _ = raw.close_session();
        bail!("服务器不支持安全原子覆盖，原文件未修改；请取消或更换上传文件名");
    }
    Ok(raw)
}

pub async fn upload_with_confirmation<F>(
    live: &LiveSession,
    source: &Path,
    directory: &str,
    name: &str,
    report: impl Fn(TransferProgress),
    confirm: impl FnOnce(String) -> F,
    committing: impl FnOnce(),
    cancel: &CancellationToken,
) -> Result<Outcome>
where
    F: Future<Output = Result<bool>>,
{
    if name.is_empty()
        || matches!(name, "." | "..")
        || name
            .chars()
            .any(|c| c.is_control() || matches!(c, '/' | '\\'))
    {
        bail!("上传文件名无效");
    }
    if !source.is_absolute() || !tokio::fs::metadata(source).await?.is_file() {
        bail!("只支持上传本机普通文件");
    }
    let sftp = live.sftp().await?;
    let directory = timeout(Duration::from_secs(60), sftp.canonicalize(directory))
        .await
        .context("读取远程目录超时")??;
    let target = format!("{}/{name}", directory.trim_end_matches('/'));
    let before = match timeout(Duration::from_secs(60), sftp.symlink_metadata(&target))
        .await
        .context("读取目标文件超时")?
    {
        Ok(metadata) => metadata,
        Err(russh_sftp::client::error::Error::Status(status))
            if status.status_code == StatusCode::NoSuchFile =>
        {
            match upload(live, source, &directory, name, None, &report, cancel).await {
                Ok(outcome) => return Ok(outcome),
                Err(error) if error.is::<UploadCreateRejected>() => {
                    // A file may have appeared after the first lookup. Still ask
                    // before replacing it, without retrying a write or a timeout.
                    match timeout(Duration::from_secs(60), sftp.symlink_metadata(&target)).await {
                        Ok(Ok(metadata)) => metadata,
                        _ => return Err(error),
                    }
                }
                Err(error) => return Err(error),
            }
        }
        Err(error) => return Err(error.into()),
    };
    if !before.is_regular() || before.is_symlink() {
        bail!("同名目标不是普通文件，不能覆盖目录或符号链接");
    }
    if !connected(live, cancel, confirm(target.clone())).await? {
        bail!("用户取消覆盖");
    }
    let connection = live.connection.get().ok_or_else(|| anyhow!("会话已断开"))?;
    let raw = timeout(Duration::from_secs(25), rename_channel(live, connection))
        .await
        .context("建立文件替换通道超时")??;
    let result = async {
        let temporary_name = format!(".orbitshell-upload-{}.tmp", Uuid::new_v4());
        let mut temporary = RemoteTemporary {
            sftp: sftp.clone(),
            path: format!("{}/{temporary_name}", directory.trim_end_matches('/')),
            armed: false,
        };
        let uploaded = upload(
            live,
            source,
            &directory,
            &temporary_name,
            Some(0o600),
            |mut event| {
                event.name = name.into();
                report(event);
            },
            cancel,
        )
        .await?;
        temporary.armed = true;
        // Match the existing owner's permissions before committing the complete file.
        let owner = sftp.symlink_metadata(&temporary.path).await?;
        let ownership_changed = owner.uid != before.uid || owner.gid != before.gid;
        if ownership_changed && (before.uid.is_none() || before.gid.is_none()) {
            bail!("无法确认原文件的属主与属组，已停止覆盖");
        }
        sftp.set_metadata(
            &temporary.path,
            FileAttributes {
                uid: if ownership_changed { before.uid } else { None },
                gid: if ownership_changed { before.gid } else { None },
                permissions: before.permissions,
                ..FileAttributes::empty()
            },
        )
        .await
        .context("无法保留原文件的权限或属主，已停止覆盖")?;
        let latest = sftp.symlink_metadata(&target).await?;
        if !latest.is_regular()
            || latest.is_symlink()
            || latest.size != before.size
            || latest.mtime != before.mtime
            || latest.permissions != before.permissions
            || latest.uid != before.uid
            || latest.gid != before.gid
        {
            bail!("目标文件在确认后发生变化，已停止覆盖，请刷新目录后重新确认");
        }
        if live.cancel.is_cancelled() || cancel.is_cancelled() {
            bail!("传输已停止，未提交覆盖文件");
        }
        committing();
        let mut payload = Vec::new();
        for path in [&temporary.path, &target] {
            payload.extend_from_slice(&(path.len() as u32).to_be_bytes());
            payload.extend_from_slice(path.as_bytes());
        }
        match timeout(
            Duration::from_secs(60),
            raw.extended("posix-rename@openssh.com", payload),
        )
        .await
        .context("文件覆盖结果未确认，请核对远程文件后再决定是否重试")??
        {
            Packet::Status(status) if status.status_code == StatusCode::Ok => {}
            _ => bail!("远程文件覆盖未成功，请核对目标文件"),
        }
        temporary.armed = false;
        Ok(Outcome {
            name: name.into(),
            bytes: uploaded.bytes,
            local_path: source.into(),
            remote_path: target,
        })
    }
    .await;
    let _ = raw.close_session();
    result
}

pub fn validate_remote_path(path: &str, file: bool) -> Result<(), String> {
    if !path.starts_with('/') || path.len() > 4096 || path.chars().any(char::is_control) {
        return Err(
            "请提供 SFTP 中的绝对路径，不支持相对路径、控制字符或超过 4096 字节的路径".into(),
        );
    }
    if file
        && path
            .rsplit('/')
            .next()
            .is_none_or(|name| name.is_empty() || matches!(name, "." | ".."))
    {
        return Err("下载路径必须指向具体文件".into());
    }
    Ok(())
}

pub fn local_file_path(value: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(value);
    if !path.is_absolute()
        || value.len() > 32767
        || value.chars().any(char::is_control)
        || path.file_name().is_none()
    {
        return Err("请提供 OrbitShell 所在电脑上的完整文件路径，不支持相对路径或控制字符".into());
    }
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};
        if !matches!(path.components().next(), Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)))
        {
            return Err("本机路径只支持本地磁盘文件，不支持设备或网络共享路径".into());
        }
        if path.components().any(
            |part| matches!(part, Component::Normal(name) if name.to_string_lossy().contains(':')),
        ) {
            return Err("不支持 Windows 备用数据流路径".into());
        }
    }
    Ok(path)
}

pub async fn resolve_local_file(path: &Path, download: bool) -> anyhow::Result<PathBuf> {
    let prepared = match download {
        false => tokio::fs::canonicalize(path).await?,
        true => {
            match tokio::fs::symlink_metadata(path).await {
                Ok(_) => anyhow::bail!("本机已存在同名目标，请选择新的文件名；未覆盖原文件"),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            let parent = tokio::fs::canonicalize(
                path.parent()
                    .ok_or_else(|| anyhow::anyhow!("本机保存目录无效"))?,
            )
            .await?;
            if !tokio::fs::metadata(&parent).await?.is_dir() {
                anyhow::bail!("本机保存位置不是目录");
            }
            parent.join(
                path.file_name()
                    .ok_or_else(|| anyhow::anyhow!("本机保存文件名无效"))?,
            )
        }
    };
    // Recheck resolved paths so junctions cannot redirect local-only access to network/device paths.
    local_file_path(
        prepared
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("本机文件路径编码无效"))?,
    )
    .map_err(anyhow::Error::msg)?;
    Ok(prepared)
}
