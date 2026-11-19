use gpui::{Context, Div, Window, div, prelude::*, px};
use tokio::sync::mpsc;

use super::{
    Dialog, Workbench,
    actions::{setting_choices, setting_group, setting_row},
    control,
};
use crate::theme::{color, danger_control, primary_control, rgb, space, text};

pub(crate) enum DesktopAction {
    Close,
    Show,
    Hide,
    Exit,
}

pub(super) struct DesktopState {
    close_to_tray: bool,
    #[cfg(windows)]
    tray: Option<crate::windows_tray::Tray>,
    tray_error: Option<String>,
    preference_error: Option<String>,
    pub(super) exit_saving: Option<String>,
    pub(super) exit_error: Option<String>,
}

impl Default for DesktopState {
    fn default() -> Self {
        Self {
            close_to_tray: true,
            #[cfg(windows)]
            tray: None,
            tray_error: None,
            preference_error: None,
            exit_saving: None,
            exit_error: None,
        }
    }
}

impl DesktopState {
    pub(super) fn tray_available(&self) -> bool {
        #[cfg(windows)]
        {
            self.tray.is_some()
        }
        #[cfg(not(windows))]
        {
            false
        }
    }
}

impl Workbench {
    pub(crate) fn install_desktop(&mut self, window: &Window, cx: &mut Context<Self>) {
        if let Some(backend) = &self.backend {
            match backend.close_to_tray() {
                Ok(preference) => self.desktop.close_to_tray = preference,
                Err(_) => self.desktop.preference_error = Some("无法读取窗口关闭行为。".into()),
            }
        }
        let (actions, mut receiver) = mpsc::unbounded_channel();
        let requests = actions.clone();
        window.on_window_should_close(cx, move |_, _| requests.send(DesktopAction::Close).is_err());
        #[cfg(windows)]
        match crate::windows_tray::Tray::new(actions) {
            Ok(tray) => self.desktop.tray = Some(tray),
            Err(_) => {
                self.desktop.tray_error =
                    Some("系统托盘初始化失败，关闭窗口时将执行退出流程。".into())
            }
        }
        #[cfg(not(windows))]
        drop(actions);
        cx.spawn_in(window, async move |view, cx| {
            while let Some(action) = receiver.recv().await {
                if view
                    .update_in(cx, |view, window, cx| match action {
                        DesktopAction::Close => view.close_desktop_window(window, cx),
                        DesktopAction::Show => view.show_desktop_window(window),
                        DesktopAction::Hide => view.hide_desktop_window(window, cx),
                        DesktopAction::Exit => view.request_desktop_exit(window, cx),
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    pub(super) fn show_desktop_window(&self, window: &Window) {
        #[cfg(windows)]
        {
            crate::windows_tray::show(window);
            if let Some(tray) = &self.desktop.tray {
                tray.reset_tooltip();
            }
        }
        #[cfg(not(windows))]
        window.activate_window();
    }

    pub(super) fn hide_desktop_window(&mut self, window: &Window, cx: &mut Context<Self>) {
        #[cfg(windows)]
        let result = self
            .desktop
            .tray
            .as_ref()
            .ok_or("系统托盘不可用，窗口已保留。")
            .and_then(|tray| tray.hide(window));
        #[cfg(not(windows))]
        let result: Result<(), &'static str> = Err("当前平台不支持隐藏到系统托盘。");
        #[cfg(not(windows))]
        let _ = window;
        match result {
            Ok(()) => self.desktop.tray_error = None,
            Err(error) => {
                self.desktop.tray_error = Some(error.into());
                self.notice = Some(error);
            }
        }
        cx.notify();
    }

    pub(super) fn close_desktop_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.desktop.close_to_tray
            && self.desktop.tray_available()
            && self.dialog != Some(Dialog::Exit)
        {
            self.hide_desktop_window(window, cx);
        } else {
            self.request_desktop_exit(window, cx);
        }
    }

    pub(super) fn request_desktop_exit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_desktop_window(window);
        if self.dialog == Some(Dialog::Exit) {
            return;
        }
        if self.editors_busy() || !self.exit_drafts(cx).is_empty() {
            if self.dialog.is_some() {
                self.close_dialog(window, cx);
            }
            self.open_dialog(Dialog::Exit, window, cx);
        } else {
            self.quit_desktop(window, cx);
        }
    }

    fn quit_desktop(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(windows)]
        self.desktop.tray.take();
        self.mcp_start.take();
        self.mcp_server.take();
        for session in &self.sessions {
            if let Some(remote) = &session.remote {
                remote.live.cancel.cancel();
            }
        }
        window.remove_window();
        cx.quit();
    }

    fn save_all_and_exit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dialog != Some(Dialog::Exit)
            || self.editors_busy()
            || self.desktop.exit_saving.is_some()
        {
            return;
        }
        let drafts = self.exit_drafts(cx);
        if drafts.iter().any(|draft| !draft.can_save) {
            self.desktop.exit_error = Some("有草稿暂不能保存，请取消退出后处理。".into());
            cx.notify();
            return;
        }
        self.desktop.exit_error = None;
        if let Some(draft) = drafts.first() {
            self.desktop.exit_saving = Some(draft.id.clone());
            if !self.save_exit_editor(&draft.id, window, cx) {
                self.desktop.exit_saving = None;
                self.desktop.exit_error = Some("文件暂不能保存，请返回编辑器后重试。".into());
            }
            cx.notify();
        } else {
            self.quit_desktop(window, cx);
        }
    }

    pub(super) fn finish_exit_save(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.desktop.exit_saving.as_deref() != Some(id) {
            return;
        }
        self.desktop.exit_saving = None;
        if let Some(draft) = self
            .exit_drafts(cx)
            .into_iter()
            .find(|draft| draft.id == id)
        {
            self.desktop.exit_error = Some(format!(
                "{} 保存未完成。取消退出后可查看编辑器中的提示。",
                draft.path
            ));
        } else {
            self.save_all_and_exit(window, cx);
        }
        cx.notify();
    }

    fn set_close_behavior(&mut self, background: bool, cx: &mut Context<Self>) {
        if background && !self.desktop.tray_available() {
            return;
        }
        let result = self
            .backend
            .as_ref()
            .ok_or("工作区配置不可用。")
            .and_then(|backend| {
                backend
                    .set_close_to_tray(background)
                    .map_err(|_| "窗口关闭行为未保存，请重试。")
            });
        match result {
            Ok(()) => {
                self.desktop.close_to_tray = background;
                self.desktop.preference_error = None;
            }
            Err(error) => self.desktop.preference_error = Some(error.into()),
        }
        cx.notify();
    }

    pub(super) fn desktop_settings(&self, cx: &Context<Self>) -> Div {
        let available = self.desktop.tray_available();
        let background = available && self.desktop.close_to_tray;
        setting_group("窗口与后台运行")
            .mt(space::MD)
            .child(
                setting_row("关闭窗口时").child(
                    setting_choices()
                        .when(available, |row| {
                            row.child(control("close-to-tray", "后台运行", background).on_click(
                                cx.listener(|view, _, _, cx| view.set_close_behavior(true, cx)),
                            ))
                        })
                        .child(control("close-exit-app", "退出应用", !background).on_click(
                            cx.listener(|view, _, _, cx| view.set_close_behavior(false, cx)),
                        )),
                ),
            )
            .child(
                div()
                    .text_size(text::DETAIL)
                    .text_color(rgb(color::MUTED))
                    .child(if available {
                        "后台运行会保留会话、传输任务和文件草稿。点击托盘图标可恢复窗口。"
                    } else {
                        "系统托盘不可用时，关闭窗口会进入退出流程。"
                    }),
            )
            .children(
                [&self.desktop.tray_error, &self.desktop.preference_error]
                    .into_iter()
                    .flatten()
                    .map(|error| {
                        div()
                            .text_size(text::DETAIL)
                            .text_color(rgb(color::DANGER))
                            .child(error.clone())
                    }),
            )
    }

    pub(super) fn desktop_exit_dialog(&self, cx: &Context<Self>) -> Div {
        let drafts = self.exit_drafts(cx);
        let has_drafts = !drafts.is_empty();
        let blocked = self.editors_busy() || self.desktop.exit_saving.is_some();
        let can_save = drafts.iter().all(|draft| draft.can_save);
        div()
            .p(space::LG)
            .flex()
            .flex_col()
            .gap(space::MD)
            .child(div().child(if blocked {
                "正在处理文件，请等待操作完成。"
            } else if !has_drafts {
                "退出会结束当前会话和后台任务。"
            } else {
                "退出前可以保存草稿。保存前会记录远程原文件的快照。"
            }))
            .when(has_drafts, |body| {
                body.child(
                    div()
                        .id("exit-draft-list")
                        .max_h(px(135.))
                        .overflow_y_scroll()
                        .text_color(rgb(color::MUTED))
                        .children(
                            drafts
                                .into_iter()
                                .map(|draft| div().py(space::XS).truncate().child(draft.path)),
                        ),
                )
            })
            .when_some(self.desktop.exit_error.clone(), |body, error| {
                body.child(
                    div()
                        .text_size(text::DETAIL)
                        .text_color(rgb(color::DANGER))
                        .child(error),
                )
            })
            .when(!blocked && !can_save, |body| {
                body.child(
                    div()
                        .text_size(text::DETAIL)
                        .text_color(rgb(color::MUTED))
                        .child("有草稿暂不能保存，可取消退出后处理。"),
                )
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(space::SM)
                    .when(!blocked, |row| {
                        row.child(control("cancel-desktop-exit", "取消", false).on_click(
                            cx.listener(|view, _, window, cx| view.close_dialog(window, cx)),
                        ))
                        .when(has_drafts, |row| {
                            row.child(
                                danger_control("discard-desktop-drafts", "不保存并退出").on_click(
                                    cx.listener(|view, _, window, cx| {
                                        view.quit_desktop(window, cx)
                                    }),
                                ),
                            )
                        })
                        .when(can_save, |row| {
                            row.child(
                                primary_control(
                                    "confirm-desktop-exit",
                                    if has_drafts {
                                        "全部保存并退出"
                                    } else {
                                        "退出应用"
                                    },
                                )
                                .on_click(cx.listener(
                                    |view, _, window, cx| view.save_all_and_exit(window, cx),
                                )),
                            )
                        })
                    }),
            )
    }
}
