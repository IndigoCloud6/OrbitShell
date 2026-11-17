use crate::ssh::LiveSession;
use anyhow::{anyhow, bail, Context, Result};
use russh_sftp::protocol::OpenFlags;
use serde::Serialize;
use std::{future::Future, path::{Path, PathBuf}, time::{Duration, Instant}};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferProgress { pub name: String, pub bytes: u64, pub total: Option<u64> }

pub struct Outcome { pub name: String, pub bytes: u64, pub local_path: PathBuf, pub remote_path: String }

async fn connected<T>(live: &LiveSession, cancel: &CancellationToken, action: impl Future<Output = Result<T>>) -> Result<T> {
    tokio::select! {
        biased;
        _ = live.cancel.cancelled() => Err(anyhow!("连接已关闭，传输已停止")),
        _ = cancel.cancelled() => Err(anyhow!("传输已取消")),
        result = action => result,
    }
}

async fn copy<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(source: &mut R, destination: &mut W,
    name: &str, total: Option<u64>, report: &impl Fn(TransferProgress)) -> Result<u64> {
    let mut buffer = vec![0u8; 64 * 1024];
    let mut bytes = 0u64;
    let mut last = Instant::now();
    let progress = |bytes| report(TransferProgress { name: name.into(), bytes, total });
    progress(0);
    loop {
        let count = timeout(Duration::from_secs(60), source.read(&mut buffer)).await.context("等待文件数据超时")??;
        if count == 0 { break; }
        timeout(Duration::from_secs(60), destination.write_all(&buffer[..count])).await.context("写入文件超时")??;
        bytes += count as u64;
        if last.elapsed() >= Duration::from_millis(100) { progress(bytes); last = Instant::now(); }
    }
    if total.is_some_and(|total| total != bytes) { bail!("文件大小在传输过程中发生变化，请确认文件稳定后重试"); }
    progress(bytes);
    Ok(bytes)
}

pub async fn download(live: &LiveSession, remote: &str, destination: &Path, overwrite: bool,
    report: impl Fn(TransferProgress), cancel: &CancellationToken) -> Result<Outcome> {
    let name = remote.rsplit('/').next().filter(|name| !name.is_empty() && *name != "." && *name != "..")
        .ok_or_else(|| anyhow!("远程文件路径无效"))?.to_owned();
    connected(live, cancel, async {
        let sftp = live.sftp().await?;
        let before = timeout(Duration::from_secs(60), sftp.metadata(remote)).await.context("读取远程文件信息超时")??;
        if before.permissions.is_some_and(|mode| mode & 0o170000 != 0 && mode & 0o170000 != 0o100000) {
            bail!("只支持下载普通文件，不支持目录、设备或管道");
        }
        let mut source = timeout(Duration::from_secs(60), sftp.open(remote)).await.context("打开远程文件超时")??;
        let metadata = timeout(Duration::from_secs(60), source.metadata()).await.context("读取远程文件信息超时")??;
        if metadata.permissions.is_some_and(|mode| mode & 0o170000 != 0 && mode & 0o170000 != 0o100000) {
            bail!("只支持下载普通文件，不支持目录、设备或管道");
        }
        let parent = destination.parent().ok_or_else(|| anyhow!("本机保存目录无效"))?;
        let temporary = tempfile::NamedTempFile::new_in(parent)?;
        let mut output = tokio::fs::File::from_std(temporary.reopen()?);
        let bytes = copy(&mut source, &mut output, &name, metadata.size, &report).await?;
        output.flush().await?;
        output.sync_all().await?;
        drop(output);
        timeout(Duration::from_secs(60), source.close()).await.context("关闭远程文件超时")??;
        if live.cancel.is_cancelled() || cancel.is_cancelled() { bail!("传输已停止，未提交下载文件"); }
        // The desktop save dialog may approve replacement; MCP always uses no-clobber.
        if overwrite { temporary.persist(destination).map_err(|error| error.error)?; }
        else { temporary.persist_noclobber(destination).map_err(|error| error.error)
            .context("无法提交下载文件；目标可能已存在，未覆盖原文件")?; }
        Ok(Outcome { name, bytes, local_path: destination.into(), remote_path: remote.into() })
    }).await
}

pub async fn upload(live: &LiveSession, source: &Path, directory: &str, filename: &str,
    report: impl Fn(TransferProgress), cancel: &CancellationToken) -> Result<Outcome> {
    if filename.is_empty() || matches!(filename, "." | "..") || filename.chars().any(|c| c.is_control() || matches!(c, '/' | '\\')) {
        bail!("本机文件名无效");
    }
    let name = filename.to_owned();
    let (sftp, path, mut input, total) = connected(live, cancel, async {
        let sftp = live.sftp().await?;
        let directory = timeout(Duration::from_secs(60), sftp.canonicalize(directory)).await.context("读取远程目录超时")??;
        let mut path = directory.trim_end_matches('/').to_owned();
        path.push('/'); path.push_str(&name);
        if !tokio::fs::metadata(source).await?.is_file() { bail!("只支持上传本机普通文件"); }
        let input = tokio::fs::File::open(source).await?;
        let metadata = input.metadata().await?;
        if !metadata.is_file() { bail!("只支持上传本机普通文件"); }
        Ok((sftp, path, input, metadata.len()))
    }).await?;
    // Never truncate or replace an existing remote file, including a symlink.
    let mut output = connected(live, cancel, async {
        Ok(timeout(Duration::from_secs(60), sftp.open_with_flags(&path, OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::EXCLUDE)).await
            .context("创建远程文件超时，结果未确认，请先核对目标文件")?
            .context("无法创建远程文件；若存在同名文件，请先重命名")?)
    }).await?;
    let copied = connected(live, cancel, async {
        let bytes = copy(&mut input, &mut output, &name, Some(total), &report).await?;
        timeout(Duration::from_secs(60), output.flush()).await.context("刷新远程文件超时")??;
        Ok(bytes)
    }).await;
    // close consumes the SFTP file. Close exactly once after releasing the copy borrow.
    let close_wait = Duration::from_secs(if copied.is_ok() { 60 } else { 5 });
    let closed = connected(live, cancel, async {
        timeout(close_wait, output.close()).await.context("关闭远程文件超时")??;
        Ok(())
    }).await;
    let result = copied.and_then(|bytes| closed.map(|()| bytes));
    match result {
        Ok(bytes) => Ok(Outcome { name, bytes, local_path: source.into(), remote_path: path }),
        Err(error) => {
            // Only clean up the new file created by this attempt, never an existing destination.
            let _ = timeout(Duration::from_secs(5), sftp.remove_file(path)).await;
            Err(error)
        }
    }
}
