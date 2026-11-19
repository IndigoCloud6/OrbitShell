use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use tokio::time::timeout;

use crate::session::LiveSession;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteEntry {
    pub name: String,
    pub path: String,
    pub is_directory: bool,
    pub is_symlink: bool,
    pub size: u64,
    #[serde(skip)]
    pub has_size: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified_at: Option<u32>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteDirectory {
    pub path: String,
    pub entries: Vec<RemoteEntry>,
}

pub async fn list(live: &LiveSession, path: &str) -> Result<RemoteDirectory> {
    tokio::select! {
        _ = live.cancel.cancelled() => Err(anyhow!("连接已关闭")),
        result = async {
            if let Some(trace) = live.diagnostics.get() {
                trace.sftp(None, "directory_list_requested", None);
            }
            let sftp = live.sftp().await?;
            timeout(Duration::from_secs(30), async {
                let path = sftp.canonicalize(if path.is_empty() { "." } else { path }).await?;
                let mut entries: Vec<_> = sftp.read_dir(&path).await?.map(|entry| {
                    let meta = entry.metadata();
                    RemoteEntry {
                        name: entry.file_name(),
                        path: entry.path(),
                        is_directory: meta.is_dir(),
                        is_symlink: meta.is_symlink(),
                        size: meta.len(),
                        has_size: meta.size.is_some(),
                        modified_at: meta.mtime,
                    }
                }).collect();
                entries.sort_by(|left, right| right.is_directory.cmp(&left.is_directory).then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase())));
                Ok::<_, anyhow::Error>(RemoteDirectory { path, entries })
            }).await.context("读取目录超时，请重试")?
        } => result,
    }
}

pub async fn delete_entry(
    live: &LiveSession,
    directory: &str,
    path: &str,
    is_directory: bool,
    is_symlink: bool,
) -> Result<(), String> {
    let prefix = format!("{}/", directory.trim_end_matches('/'));
    let valid_name = path
        .strip_prefix(prefix.as_str())
        .is_some_and(|name| !name.is_empty() && !matches!(name, "." | "..") && !name.contains('/'));
    if !directory.starts_with('/')
        || directory.split('/').any(|part| matches!(part, "." | ".."))
        || !valid_name
        || path.len() > 4096
        || path.chars().any(char::is_control)
    {
        return Err("只能删除当前目录中的单个有效项目".into());
    }
    let sftp = live.sftp().await.map_err(|error| format!("{error:#}"))?;
    let preflight = async {
        let actual_directory = sftp.canonicalize(directory).await?;
        if actual_directory.trim_end_matches('/') != directory.trim_end_matches('/') {
            anyhow::bail!("远程目录的实际路径已改变，请刷新列表后重新确认");
        }
        let metadata = sftp.symlink_metadata(path).await?;
        if metadata.is_dir() != is_directory || metadata.is_symlink() != is_symlink {
            anyhow::bail!("远程项目的类型已改变，请刷新列表后重新确认");
        }
        if !metadata.is_regular() && !is_directory && !is_symlink {
            anyhow::bail!("此类型的远程项目不支持删除");
        }
        Ok::<_, anyhow::Error>(())
    };
    tokio::select! {
        biased;
        _ = live.cancel.cancelled() => return Err("连接已关闭，未发起删除".into()),
        result = timeout(Duration::from_secs(30), preflight) => {
            result.map_err(|_| "读取远程项目信息超时，未发起删除")?
                .map_err(|error| format!("{error:#}"))?;
        }
    }

    // RMDIR is non-recursive. REMOVE unlinks the entry itself, including symlinks.
    let remove = async {
        if is_directory && !is_symlink {
            sftp.remove_dir(path)
                .await
                .context("无法删除目录，请确认目录为空且拥有删除权限")
        } else {
            sftp.remove_file(path)
                .await
                .context("远程服务器未确认删除成功")
        }
    };
    tokio::select! {
        biased;
        _ = live.cancel.cancelled() => Err("连接已断开，无法确认删除结果。请重新连接后刷新目录。".into()),
        result = timeout(Duration::from_secs(30), remove) => result
            .map_err(|_| "删除响应超时，无法确认结果。请刷新目录后查看。".to_string())?
            .map_err(|error| format!("{error:#}")),
    }
}
