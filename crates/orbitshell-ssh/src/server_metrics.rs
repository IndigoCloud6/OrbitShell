use std::sync::atomic::Ordering;

use anyhow::{anyhow, bail, Context, Result};
use russh::{client, Channel, ChannelMsg};
use tokio::time::{timeout_at, Duration, Instant};

use crate::session::LiveSession;

const PROBE: &str = include_str!("server_metrics.sh");
const MAX_OUTPUT: usize = 16 * 1024;

#[derive(Clone, Copy)]
pub struct CpuTicks {
    total: u64,
    busy: u64,
}

impl CpuTicks {
    pub fn usage_since(self, previous: Self) -> Option<f64> {
        let total = self.total.checked_sub(previous.total)?;
        let busy = self.busy.checked_sub(previous.busy)?;
        (total > 0).then(|| 100. * (busy.min(total) as f64 / total as f64))
    }
}

#[derive(Clone, Copy)]
pub struct Capacity {
    pub used_kib: u64,
    pub total_kib: u64,
}

#[derive(Default)]
pub struct Inventory {
    pub processor: Option<String>,
    pub threads: Option<u32>,
    pub gpu_count: Option<u32>,
    pub gpu_models: Vec<String>,
}

#[derive(Default)]
pub struct Sample {
    pub cpu: Option<CpuTicks>,
    pub memory: Option<Capacity>,
    pub memory_total_kib: Option<u64>,
    pub root: Option<Capacity>,
    pub inventory: Option<Inventory>,
}

pub async fn sample(live: &LiveSession, include_inventory: bool) -> Result<Sample> {
    if live.is_temporary || !live.connected.load(Ordering::Acquire) {
        bail!("当前 SSH 会话不支持采集服务器信息");
    }
    let connection = live
        .connection
        .get()
        .ok_or_else(|| anyhow!("SSH 尚未连接"))?;
    let deadline = Instant::now() + Duration::from_secs(4);
    let mut channel = tokio::select! {
        biased;
        _ = live.cancel.cancelled() => bail!("SSH 会话已关闭"),
        opened = timeout_at(deadline, connection.channel_open_session()) => {
            opened.context("采集通道打开超时")??
        }
    };
    let result = tokio::select! {
        biased;
        _ = live.cancel.cancelled() => Err(anyhow!("SSH 会话已关闭")),
        result = timeout_at(deadline, run_probe(&mut channel, include_inventory)) => {
            result.context("采集服务器信息超时").and_then(|value| value)
        }
    };
    // Close just this exec channel, including after a timeout or cancellation.
    drop(channel.into_stream());
    result
}

async fn run_probe(channel: &mut Channel<client::Msg>, inventory: bool) -> Result<Sample> {
    let command = if inventory {
        "/bin/sh -s -- inventory"
    } else {
        "/bin/sh -s"
    };
    channel.exec(true, command).await?;
    let mut output = Vec::new();
    let mut exit_status = None;
    let mut accepted = false;
    while let Some(message) = channel.wait().await {
        match message {
            ChannelMsg::Success if !accepted => {
                accepted = true;
                channel
                    .data_bytes(PROBE.replace("\r\n", "\n").into_bytes())
                    .await?;
                channel.eof().await?;
            }
            ChannelMsg::Data { data } => {
                if output.len() + data.len() > MAX_OUTPUT {
                    bail!("服务器返回的信息超过采集上限");
                }
                output.extend_from_slice(&data);
            }
            ChannelMsg::ExitStatus { exit_status: code } => exit_status = Some(code),
            ChannelMsg::Failure => bail!("服务器未允许执行信息采集"),
            ChannelMsg::Close => break,
            _ => {}
        }
    }
    if !accepted || exit_status != Some(0) {
        bail!("服务器未返回可用的系统信息");
    }
    parse_sample(&String::from_utf8_lossy(&output), inventory)
}

fn parse_sample(output: &str, include_inventory: bool) -> Result<Sample> {
    let mut lines = output
        .lines()
        .skip_while(|line| *line != "orbitshell-metrics-v1");
    if lines.next().is_none() {
        bail!("服务器未返回 Linux 系统数据");
    }
    let mut sample = Sample::default();
    let mut inventory = Inventory::default();
    let mut mem_available = None::<u64>;
    for line in lines {
        let Some((key, value)) = line.split_once(' ') else {
            continue;
        };
        match key {
            "cpu" => sample.cpu = parse_cpu(value),
            "MemTotal:" => {
                sample.memory_total_kib = value.trim().parse().ok().filter(|size| *size > 0)
            }
            "MemAvailable:" => mem_available = value.trim().parse().ok(),
            "disk" => {
                let mut fields = value.split_whitespace();
                if let (Some(total_kib), Some(used_kib)) = (
                    fields.next().and_then(|part| part.parse().ok()),
                    fields.next().and_then(|part| part.parse().ok()),
                ) {
                    sample.root = Some(Capacity {
                        used_kib,
                        total_kib,
                    });
                }
            }
            "model" if include_inventory => inventory.processor = clean_label(value),
            "threads" if include_inventory => {
                inventory.threads = value.trim().parse().ok().filter(|count| *count > 0)
            }
            "gpu-count" if include_inventory => inventory.gpu_count = value.trim().parse().ok(),
            "gpu" if include_inventory && inventory.gpu_models.len() < 16 => {
                inventory.gpu_models.extend(clean_label(value));
            }
            _ => {}
        }
    }
    if let (Some(total_kib), Some(available)) = (sample.memory_total_kib, mem_available) {
        sample.memory = Some(Capacity {
            used_kib: total_kib.saturating_sub(available),
            total_kib,
        });
    }
    if include_inventory {
        sample.inventory = Some(inventory);
    }
    if sample.cpu.is_none() && sample.memory_total_kib.is_none() && sample.root.is_none() {
        bail!("服务器的系统统计不可用");
    }
    Ok(sample)
}

fn parse_cpu(value: &str) -> Option<CpuTicks> {
    let mut fields = value.split_whitespace();
    let mut ticks = [0u64; 8];
    for (index, slot) in ticks.iter_mut().enumerate() {
        match fields.next() {
            Some(value) => *slot = value.parse().ok()?,
            None if index < 4 => return None,
            None => break,
        }
    }
    let [user, nice, system, idle, wait, irq, softirq, steal] = ticks;
    // Guest timers are already included in user and nice. Steal and I/O wait
    // do not represent work executing on this host's logical processors.
    let busy = [user, nice, system, irq, softirq]
        .into_iter()
        .try_fold(0u64, u64::checked_add)?;
    let total = [idle, wait, steal]
        .into_iter()
        .try_fold(busy, u64::checked_add)?;
    Some(CpuTicks { total, busy })
}

fn clean_label(input: &str) -> Option<String> {
    let label: String = input.trim().chars().filter(|character| {
        !character.is_control() && !matches!(character, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
    }).take(160).collect();
    (!label.is_empty()).then_some(label)
}
