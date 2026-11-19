use std::{
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};

use gpui::{Context, Div, Render, Stateful, Window, div, prelude::*, px};
use orbitshell_ssh::server_metrics::{Capacity, Inventory, Sample};

use super::{Page, RemoteSession, Workbench};
use crate::theme::{color, rgb, space, text};

const SAMPLE_INTERVAL: Duration = Duration::from_secs(5);
const RETRY_DELAY: Duration = Duration::from_secs(30);
const FRESH_FOR: Duration = Duration::from_secs(15);

#[derive(Default)]
pub(super) struct RemoteMetrics {
    last_sample: Option<(Instant, Sample)>,
    usage: Option<f64>,
    inventory: Option<Inventory>,
    retry_at: Option<Instant>,
}

impl RemoteSession {
    pub(super) fn supports_metrics(&self) -> bool {
        !self.live.is_temporary
            && self.live.connected.load(Ordering::Acquire)
            && !self.live.cancel.is_cancelled()
    }
}

impl Workbench {
    pub(super) fn observe_server_metrics(&self, window: &Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |view, cx| {
            loop {
                let pending = view.update_in(cx, |view, window, _| {
                    if view.page != Page::Terminal || !window.is_window_active() {
                        return None;
                    }
                    let remote = view.active_session()?.remote.as_ref()?;
                    if !remote.supports_metrics()
                        || remote
                            .metrics
                            .retry_at
                            .is_some_and(|until| until > Instant::now())
                    {
                        return None;
                    }
                    let live = remote.live.clone();
                    let probe = view
                        .backend
                        .as_ref()?
                        .server_metrics(live.clone(), remote.metrics.inventory.is_none());
                    Some((live, probe))
                });
                match pending {
                    Ok(Some((live, probe))) => {
                        let result = probe.await.ok().and_then(Result::ok);
                        if view
                            .update_in(cx, |view, _, cx| {
                                if let Some(remote) = view
                                    .sessions
                                    .iter_mut()
                                    .filter_map(|session| session.remote.as_mut())
                                    .find(|remote| {
                                        Arc::ptr_eq(&remote.live, &live)
                                            && remote.supports_metrics()
                                    })
                                {
                                    remote.metrics.update(result);
                                    cx.notify();
                                }
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                    Ok(None) => {}
                    Err(_) => break,
                }
                cx.background_executor().timer(SAMPLE_INTERVAL).await;
            }
        })
        .detach();
    }
}

impl RemoteMetrics {
    fn update(&mut self, result: Option<Sample>) {
        match result {
            Some(mut sample) => {
                self.usage = self.last_sample.as_ref().and_then(|(received, previous)| {
                    if received.elapsed() > FRESH_FOR {
                        return None;
                    }
                    sample.cpu?.usage_since(previous.cpu?)
                });
                if let Some(inventory) = sample.inventory.take() {
                    self.inventory = Some(inventory);
                }
                self.last_sample = Some((Instant::now(), sample));
                self.retry_at = None;
            }
            None => {
                self.usage = None;
                self.last_sample = None;
                self.retry_at = Some(Instant::now() + RETRY_DELAY);
            }
        }
    }

    pub(super) fn strip(&self) -> Stateful<Div> {
        let strip = div()
            .id("server-metrics")
            .flex_1()
            .min_w(px(0.))
            .pl(space::SM)
            .border_l_1()
            .border_color(rgb(color::LINE))
            .text_size(text::DETAIL)
            .text_color(rgb(color::MUTED))
            .truncate();
        let Some((received, sample)) = &self.last_sample else {
            return strip.child(if self.retry_at.is_some() {
                "服务器信息暂不可用"
            } else {
                "采集服务器信息…"
            });
        };
        if received.elapsed() > FRESH_FOR {
            return strip.child("刷新服务器信息…");
        }

        let inventory = self.inventory.as_ref();
        let mut figures = Vec::new();
        if let Some(usage) = self.usage {
            figures.push(format!("CPU {usage:.0}%"));
        } else if let Some(threads) = inventory.and_then(|hardware| hardware.threads) {
            figures.push(format!("CPU {threads} 线程"));
        }
        if let Some(memory) = sample.memory {
            figures.push(format!("内存 {}", format_capacity(memory)));
        } else if let Some(total) = sample.memory_total_kib {
            figures.push(format!("内存 {}", format_size(total)));
        }
        if let Some(root) = sample.root {
            figures.push(format!("根分区 {}", format_capacity(root)));
        }
        if let Some(count) = inventory.and_then(|hardware| hardware.gpu_count) {
            figures.push(format!("GPU {count} 张"));
        }
        if figures.is_empty() {
            figures.push("采集服务器信息…".into());
        }

        let summary = figures.join("  ·  ");
        let mut details = Vec::new();
        if let Some(hardware) = inventory {
            if let Some(processor) = &hardware.processor {
                details.push(match hardware.threads {
                    Some(threads) => format!("CPU {processor} · {threads} 个逻辑处理器"),
                    None => format!("CPU {processor}"),
                });
            }
            match hardware.gpu_count {
                None => details.push("GPU 信息不可用".into()),
                Some(0) => details.push("未检测到 PCI 显示设备".into()),
                Some(count) => {
                    details.push(format!("检测到 {count} 个 PCI 显示设备"));
                    details.extend(hardware.gpu_models.iter().cloned());
                }
            }
        }
        let tooltip = MetricsTooltip {
            hardware: details,
            live: figures,
        };
        strip
            .child(summary)
            .tooltip(move |_, cx| cx.new(|_| tooltip.clone()).into())
    }
}

fn format_capacity(capacity: Capacity) -> String {
    format!(
        "{}/{}",
        format_size(capacity.used_kib),
        format_size(capacity.total_kib)
    )
}

fn format_size(kib: u64) -> String {
    const MIB: u64 = 1024;
    const GIB: u64 = 1024 * MIB;
    const TIB: u64 = 1024 * GIB;
    match kib {
        TIB.. => format!("{:.1} TiB", kib as f64 / TIB as f64),
        GIB.. => format!("{:.1} GiB", kib as f64 / GIB as f64),
        _ => format!("{:.0} MiB", kib as f64 / MIB as f64),
    }
}

#[derive(Clone)]
struct MetricsTooltip {
    hardware: Vec<String>,
    live: Vec<String>,
}

impl Render for MetricsTooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let has_hardware = !self.hardware.is_empty();
        div()
            // A definite width lets GPUI measure wrapped text on the first layout pass.
            .w(px(410.))
            .flex()
            .flex_col()
            .gap(space::SM)
            .px(space::MD)
            .py(space::SM)
            .rounded(space::CONTROL_RADIUS)
            .border_1()
            .border_color(rgb(color::LINE))
            .bg(rgb(color::SURFACE))
            .text_size(text::DETAIL)
            .line_height(px(18.))
            .text_color(rgb(color::TEXT))
            .when(has_hardware, |panel| {
                panel.child(metric_rows(&self.hardware))
            })
            .child(
                div()
                    .w_full()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .gap(space::XS)
                    .when(has_hardware, |section| {
                        section
                            .pt(space::SM)
                            .border_t_1()
                            .border_color(rgb(color::LINE))
                    })
                    .child(
                        div()
                            .text_color(rgb(color::MUTED))
                            .child("实时指标 · 已用 / 总量"),
                    )
                    .child(metric_rows(&self.live)),
            )
    }
}

fn metric_rows(items: &[String]) -> Div {
    div()
        .w_full()
        .flex_none()
        .flex()
        .flex_col()
        .gap(space::XS)
        .children(
            items
                .iter()
                .cloned()
                .map(|item| div().w_full().flex_none().whitespace_normal().child(item)),
        )
}
