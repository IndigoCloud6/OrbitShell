use std::{path::PathBuf, sync::Arc, time::Duration};

use gpui::{
    Context, Div, ElementId, Empty, ExternalPaths, PathPromptOptions, Window, div, prelude::*, px,
};
use orbitshell_ssh::{session::LiveSession, transfer};
use tokio::sync::{mpsc, oneshot};

use super::{Dialog, Tool, Workbench, control, ssh::file_size};
use crate::theme::{color, rgb, space, text};
use crate::transfer_queue::{Direction, TransferJob, TransferStatus, UploadConflict};

#[derive(Clone)]
pub(super) struct UploadTarget {
    pub profile: usize,
    pub live: Arc<LiveSession>,
    pub directory: String,
}

#[derive(Clone)]
pub(super) struct DownloadTarget {
    pub live: Arc<LiveSession>,
    pub path: String,
    pub name: String,
}

struct DragPreview;

impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

impl Workbench {
    pub(super) fn observe_transfers(
        &self,
        mut conflicts: mpsc::UnboundedReceiver<UploadConflict>,
        cx: &mut Context<Self>,
    ) {
        let mut updates = self.transfers.subscribe();
        cx.spawn(async move |view, cx| {
            while updates.changed().await.is_ok() {
                if view.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        })
        .detach();
        cx.spawn(async move |view, cx| {
            while let Some(conflict) = conflicts.recv().await {
                if view
                    .update(cx, |view, cx| {
                        view.upload_conflicts.push_back(conflict);
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    pub(super) fn choose_upload(
        &mut self,
        target: UploadTarget,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let selection = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("选择要上传的文件".into()),
        });
        cx.spawn_in(window, async move |view, cx| {
            let choice = selection.await;
            let _ = view.update_in(cx, |view, _, cx| match choice {
                Ok(Ok(Some(paths))) => view.upload_paths(target, paths, cx),
                Ok(Ok(None)) => {}
                _ => view.transfer_notice("无法打开文件选择器", cx),
            });
        })
        .detach();
    }

    pub(super) fn drop_upload(
        &mut self,
        target: UploadTarget,
        paths: &ExternalPaths,
        cx: &mut Context<Self>,
    ) {
        self.upload_paths(target, paths.paths().to_vec(), cx);
    }

    fn upload_paths(&mut self, target: UploadTarget, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        if target.live.cancel.is_cancelled() {
            self.transfer_notice("SSH 会话已断开", cx);
            return;
        }
        let Some(runtime) = self
            .backend
            .as_ref()
            .map(|backend| backend.runtime_handle())
        else {
            return;
        };
        for path in paths {
            let Some(name) = path
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_owned)
            else {
                self.transfer_notice("本机文件名无效", cx);
                continue;
            };
            let Some(job) =
                self.start_file_transfer(Direction::Upload, name.clone(), &target.live, cx)
            else {
                break;
            };
            let target = target.clone();
            let destination = target.clone();
            let conflicts = self.upload_conflict_sender.clone();
            let transfer = runtime.spawn(async move {
                let result = transfer::upload_with_confirmation(
                    &destination.live,
                    &path,
                    &destination.directory,
                    &name,
                    |progress| job.progress(progress),
                    |remote_path| {
                        let job = job.clone();
                        async move {
                            let (answer, decision) = oneshot::channel();
                            job.message("等待确认覆盖");
                            conflicts
                                .send(UploadConflict {
                                    job: job.clone(),
                                    remote_path,
                                    answer,
                                })
                                .map_err(|_| anyhow::anyhow!("上传确认窗口已关闭"))?;
                            let approved = tokio::time::timeout(Duration::from_secs(900), decision)
                                .await
                                .map_err(|_| anyhow::anyhow!("等待覆盖确认超时"))?
                                .map_err(|_| anyhow::anyhow!("上传确认已关闭"))?;
                            if !approved {
                                job.cancel.cancel();
                            }
                            Ok(approved)
                        }
                    },
                    || job.message("正在提交覆盖文件"),
                    &job.cancel,
                )
                .await;
                let uploaded = result.is_ok();
                job.finish(
                    result
                        .map(|outcome| Some(outcome.bytes))
                        .map_err(|error| format!("{error:#}")),
                    destination.live.cancel.is_cancelled(),
                );
                uploaded
            });
            cx.spawn(async move |view, cx| {
                if matches!(transfer.await, Ok(true)) {
                    let _ =
                        view.update(cx, |view, cx| view.refresh_uploaded_directory(&target, cx));
                }
            })
            .detach();
        }
    }

    pub(super) fn choose_download(
        &mut self,
        target: DownloadTarget,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let directory = std::env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .map(|home| {
                let downloads = home.join("Downloads");
                if downloads.is_dir() { downloads } else { home }
            })
            .unwrap_or_else(std::env::temp_dir);
        #[cfg(windows)]
        let name =
            orbitshell_ssh::file_drag::filename(&target.path).unwrap_or_else(|_| "download".into());
        #[cfg(not(windows))]
        let name = target.name.clone();
        let selection = cx.prompt_for_new_path(&directory, Some(&name));
        cx.spawn_in(window, async move |view, cx| {
            let choice = selection.await;
            let _ = view.update_in(cx, |view, _, cx| match choice {
                Ok(Ok(Some(path))) => view.download_to(target, path, cx),
                Ok(Ok(None)) => {}
                _ => view.transfer_notice("无法打开文件保存窗口", cx),
            });
        })
        .detach();
    }

    fn download_to(&mut self, target: DownloadTarget, path: PathBuf, cx: &mut Context<Self>) {
        if target.live.cancel.is_cancelled() {
            self.transfer_notice("SSH 会话已断开", cx);
            return;
        }
        let Some(runtime) = self
            .backend
            .as_ref()
            .map(|backend| backend.runtime_handle())
        else {
            return;
        };
        let Some(job) =
            self.start_file_transfer(Direction::Download, target.name, &target.live, cx)
        else {
            return;
        };
        runtime.spawn(async move {
            // The system save dialog handles overwrite confirmation.
            let result = transfer::download(
                &target.live,
                &target.path,
                &path,
                true,
                |progress| job.progress(progress),
                &job.cancel,
            )
            .await;
            job.finish(
                result
                    .map(|outcome| Some(outcome.bytes))
                    .map_err(|error| format!("{error:#}")),
                target.live.cancel.is_cancelled(),
            );
        });
    }

    pub(super) fn drag_download_row(
        &self,
        row: gpui::Stateful<Div>,
        target: DownloadTarget,
        cx: &Context<Self>,
    ) -> gpui::Stateful<Div> {
        let view = cx.weak_entity();
        row.on_drag(target, move |target, _, window, cx| {
            let _ = view.update(cx, |view, cx| {
                view.start_native_download(target.clone(), window, cx)
            });
            cx.new(|_| DragPreview)
        })
    }

    #[cfg(windows)]
    fn start_native_download(
        &mut self,
        target: DownloadTarget,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        use orbitshell_ssh::file_drag::{self, DragLease, DragOptions};
        let Some(runtime) = self
            .backend
            .as_ref()
            .map(|backend| backend.runtime_handle())
        else {
            return;
        };
        let (lease, name) = match DragLease::acquire()
            .and_then(|lease| file_drag::filename(&target.path).map(|name| (lease, name)))
        {
            Ok(value) => value,
            Err(error) => {
                self.transfer_notice(error, cx);
                return;
            }
        };
        let Some(job) =
            self.start_file_transfer(Direction::Download, name.clone(), &target.live, cx)
        else {
            return;
        };
        job.message("等待拖放到本机文件夹");
        let scale = f64::from(window.scale_factor());
        cx.spawn_in(window, async move |view, cx| {
            // OLE pumps window messages, so enter it outside any GPUI entity update.
            let _lease = lease;
            let progress = job.clone();
            let result = file_drag::run(
                target.live.clone(),
                target.path,
                name,
                DragOptions {
                    runtime,
                    scale,
                    cancel: job.cancel.clone(),
                    progress: Box::new(move |event| progress.progress(event)),
                    should_stop: Box::new(|| false),
                },
            );
            job.finish(result, target.live.cancel.is_cancelled());
            let _ = view.update_in(cx, |_, window, cx| {
                cx.stop_active_drag(window);
            });
        })
        .detach();
    }

    #[cfg(not(windows))]
    fn start_native_download(&mut self, _: DownloadTarget, _: &Window, cx: &mut Context<Self>) {
        self.transfer_notice("当前平台请使用下载按钮", cx);
    }

    fn start_file_transfer(
        &mut self,
        direction: Direction,
        name: String,
        live: &LiveSession,
        cx: &mut Context<Self>,
    ) -> Option<Arc<TransferJob>> {
        match self.transfers.start(
            direction,
            name,
            live.profile.name.clone(),
            live.is_temporary,
        ) {
            Ok(job) => {
                self.transfer_error = None;
                self.tool = Some(Tool::Transfers);
                cx.notify();
                Some(job)
            }
            Err(error) => {
                self.transfer_notice(error, cx);
                None
            }
        }
    }

    fn transfer_notice(&mut self, error: impl Into<String>, cx: &mut Context<Self>) {
        self.transfer_error = Some(error.into());
        self.tool = Some(Tool::Transfers);
        cx.notify();
    }

    pub(super) fn show_pending_upload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.upload_conflicts
            .retain(|conflict| !conflict.answer.is_closed());
        if self.dialog.is_none() && !self.upload_conflicts.is_empty() {
            self.open_dialog(Dialog::UploadConflict, window, cx);
        } else if self.dialog == Some(Dialog::UploadConflict) && self.upload_conflicts.is_empty() {
            self.close_dialog(window, cx);
        }
    }

    pub(super) fn answer_upload(&mut self, overwrite: bool) {
        if let Some(conflict) = self.upload_conflicts.pop_front() {
            let _ = conflict.answer.send(overwrite);
        }
    }

    pub(super) fn upload_conflict_dialog(&self, cx: &Context<Self>) -> Div {
        let Some(conflict) = self.upload_conflicts.front() else {
            return div();
        };
        let transfer = conflict.job.snapshot();
        let id = transfer.id;
        div()
            .p(space::LG)
            .flex()
            .flex_col()
            .gap(space::MD)
            .child("目标目录已有同名文件。")
            .child(
                div()
                    .text_size(text::DETAIL)
                    .text_color(rgb(color::MUTED))
                    .child(transfer.session),
            )
            .child(
                div()
                    .font_family(text::MONO_FONT)
                    .text_size(text::DETAIL)
                    .child(conflict.remote_path.clone()),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(space::SM)
                    .child(
                        control(
                            ElementId::Name(format!("cancel-overwrite-{id}").into()),
                            "取消上传",
                            false,
                        )
                        .on_click(cx.listener(|view, _, window, cx| view.close_dialog(window, cx))),
                    )
                    .child(
                        control(
                            ElementId::Name(format!("confirm-overwrite-{id}").into()),
                            "覆盖文件",
                            true,
                        )
                        .on_click(cx.listener(
                            move |view, _, window, cx| {
                                if !view
                                    .upload_conflicts
                                    .front()
                                    .is_some_and(|pending| pending.job.snapshot().id == id)
                                {
                                    return;
                                }
                                view.answer_upload(true);
                                view.dialog = None;
                                view.focus_active(window, cx);
                                cx.notify();
                            },
                        )),
                    ),
            )
    }

    pub(super) fn transfer_rows(&self, cx: &Context<Self>) -> Div {
        let transfers = self.transfers.snapshots();
        div()
            .flex()
            .flex_col()
            .gap(space::XS)
            .when_some(self.transfer_error.clone(), |list, error| {
                list.child(
                    div()
                        .py(space::XS)
                        .text_color(rgb(color::ACCENT_TEXT))
                        .child(error),
                )
            })
            .when(transfers.is_empty(), |list| list.child("暂无传输任务"))
            .children(transfers.into_iter().map(|transfer| {
                let progress = match transfer.total {
                    Some(total) => format!("{} / {}", file_size(transfer.bytes), file_size(total)),
                    None => file_size(transfer.bytes),
                };
                let running = transfer.status == TransferStatus::Running;
                let tint = if transfer.status == TransferStatus::Error {
                    color::ACCENT_TEXT
                } else {
                    color::MUTED
                };
                let id = transfer.id;
                div()
                    .h(space::HEADER_HEIGHT)
                    .flex()
                    .items_center()
                    .gap(space::MD)
                    .border_b_1()
                    .border_color(rgb(color::LINE))
                    .child(div().flex_none().child(match transfer.direction {
                        Direction::Upload => "上传",
                        Direction::Download => "下载",
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .child(
                                div()
                                    .truncate()
                                    .text_color(rgb(color::TEXT))
                                    .child(transfer.name),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_color(rgb(tint))
                                    .child(if let Some(agent) = transfer.agent {
                                        format!("{} · {} · {}", agent.agent_name, transfer.session, transfer.message)
                                    } else { format!("{} · {}", transfer.session, transfer.message) }),
                            ),
                    )
                    .child(
                        div()
                            .flex_none()
                            .child(if running && transfer.bytes_per_second > 0 {
                                format!("{progress} · {}/s", file_size(transfer.bytes_per_second))
                            } else {
                                progress
                            }),
                    )
                    .when(running, |row| {
                        row.child(
                            control(
                                ElementId::Name(format!("cancel-transfer-{id}").into()),
                                "取消",
                                false,
                            )
                            .on_click(cx.listener(move |view, _, _, _| view.transfers.cancel(id))),
                        )
                    })
            }))
    }
}
