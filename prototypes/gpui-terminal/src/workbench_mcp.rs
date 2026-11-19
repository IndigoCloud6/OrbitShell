use gpui::{ClipboardItem, Context, Div, FontWeight, Window, div, prelude::*, px};
use tokio::sync::mpsc;

use super::{
    Dialog, LoginOrigin, Workbench,
    actions::{setting_action, setting_choices, setting_row},
    control, dot,
};
use crate::{
    mcp,
    theme::{color, primary_control, rgb, space, text},
};

impl Workbench {
    pub(super) fn observe_mcp(
        &self,
        mut requests: mpsc::Receiver<mcp::DesktopRequest>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(mut updates) = self
            .backend
            .as_ref()
            .map(|backend| backend.control_updates())
        {
            cx.spawn(async move |view, cx| {
                while updates.changed().await.is_ok() {
                    if view
                        .update(cx, |view, cx| {
                            for session in &view.sessions {
                                session
                                    .terminal
                                    .update(cx, |terminal, cx| terminal.control_changed(cx));
                            }
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
        cx.spawn_in(window, async move |view, cx| {
            while let Some(request) = requests.recv().await {
                if view
                    .update_in(cx, |view, window, cx| {
                        if view.mcp_requests.len() >= 8 {
                            return;
                        }
                        view.mcp_requests.push_back(request);
                        view.show_desktop_window(window);
                        cx.notify();
                        cx.spawn(async |view, cx| {
                            cx.background_executor().timer(mcp::APPROVAL_TIMEOUT).await;
                            let _ = view.update(cx, |_, cx| cx.notify());
                        })
                        .detach();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    pub(super) fn show_mcp_request(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dialog == Some(Dialog::McpApproval)
            && self
                .mcp_approval
                .as_ref()
                .is_none_or(|request| !request.is_open())
        {
            self.close_dialog(window, cx);
        }
        self.mcp_requests.retain(|request| match request {
            mcp::DesktopRequest::Approval(request) => request.is_open(),
            mcp::DesktopRequest::OpenConnection {
                client, service, ..
            } => !client.is_cancelled() && !service.is_cancelled(),
        });
        if self.dialog.is_some() {
            return;
        }
        match self.mcp_requests.pop_front() {
            Some(mcp::DesktopRequest::Approval(request)) => {
                self.mcp_approval = Some(request);
                self.open_dialog(Dialog::McpApproval, window, cx);
            }
            Some(mcp::DesktopRequest::OpenConnection { id, credential, .. }) => {
                if let Some(profile) = self.profiles.iter().position(|profile| {
                    !profile.temporary
                        && profile
                            .remote
                            .as_ref()
                            .is_some_and(|profile| profile.id == id)
                }) {
                    if let Some(credential) = credential {
                        self.open_dialog(Dialog::Login(profile, LoginOrigin::Agent), window, cx);
                        self.password
                            .update(cx, |input, cx| input.set_value(&credential, cx));
                        self.dialog_focus.focus(window);
                    } else {
                        self.connect_or_prompt_from(profile, LoginOrigin::Agent, window, cx);
                    }
                }
                cx.notify();
            }
            None => {}
        }
    }

    pub(super) fn mcp_approval_dialog(&self, cx: &Context<Self>) -> Div {
        let Some(request) = &self.mcp_approval else {
            return div();
        };
        let id = request.id;
        let can_grant_read_only = request.can_grant_read_only();
        let read_only_hint = if can_grant_read_only {
            Some("只读授权仅用于当前客户端的这个独立 AI 终端。关闭终端或断开会话时失效，也可在终端标签上撤销。")
        } else if request.title == mcp::DEDICATED_COMMAND_APPROVAL_TITLE {
            Some("会话只读授权仅适用于符合规则的单条诊断命令。本条命令需要单独确认。")
        } else {
            None
        };
        div()
            .p(space::LG)
            .flex()
            .flex_col()
            .gap(space::SM)
            .child(div().font_weight(FontWeight::SEMIBOLD).child(request.title))
            .child(
                div()
                    .text_color(rgb(color::MUTED))
                    .child(request.target.clone()),
            )
            .child(
                div()
                    .text_size(text::DETAIL)
                    .child(format!("请求客户端 · {}", request.client_name)),
            )
            .when_some(request.detail.as_ref(), |panel, detail| {
                panel.child(
                    div()
                        .id("mcp-approval-preview")
                        .max_h(px(180.))
                        .overflow_y_scroll()
                        .p(space::SM)
                        .rounded(space::CONTROL_RADIUS)
                        .bg(rgb(color::TOOLBAR))
                        .font_family(text::MONO_FONT)
                        .child(detail.as_str().to_owned()),
                )
            })
            .when_some(read_only_hint, |panel, hint| {
                panel.child(
                    div()
                        .text_size(text::DETAIL)
                        .text_color(rgb(color::MUTED))
                        .child(hint),
                )
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(space::SM)
                    .child(
                        control("deny-mcp-action", "拒绝", false).on_click(
                            cx.listener(|view, _, window, cx| view.close_dialog(window, cx)),
                        ),
                    )
                    .child(
                        control("approve-mcp-action", "仅本次", true).on_click(cx.listener(
                            move |view, _, window, cx| {
                                view.resolve_mcp_approval(id, false, window, cx);
                            },
                        )),
                    )
                    .when(can_grant_read_only, |row| row.child(
                        primary_control("approve-mcp-read-only", "允许本会话只读").on_click(
                            cx.listener(move |view, _, window, cx| {
                                view.resolve_mcp_approval(id, true, window, cx);
                            }),
                        ),
                    )),
            )
    }

    fn resolve_mcp_approval(
        &mut self,
        id: uuid::Uuid,
        read_only: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .mcp_approval
            .as_ref()
            .is_none_or(|request| request.id != id)
        {
            return;
        }
        let Some(request) = self.mcp_approval.take() else {
            return;
        };
        let accepted = if read_only {
            request.resolve_read_only()
        } else {
            request.resolve(true);
            true
        };
        self.close_dialog(window, cx);
        if !accepted {
            self.notice = Some("只读授权未生效，请重新提交命令。");
            cx.notify();
        }
    }

    pub(super) fn mcp_settings(&self, cx: &Context<Self>) -> Div {
        let server = self
            .mcp_server
            .as_ref()
            .filter(|server| server.is_running());
        let running = server.is_some();
        let starting = self.mcp_start.is_some();
        let (status, tint) = if running {
            ("运行中", color::ACCENT)
        } else if starting {
            ("正在启动", color::ACCENT)
        } else {
            ("已关闭", color::SESSION_IDLE)
        };
        let endpoint = server.map_or_else(|| mcp::ENDPOINT.to_owned(), mcp::RunningMcp::endpoint);
        let service_button = if running {
            setting_action("toggle-gpui-mcp", "关闭 MCP")
        } else {
            primary_control("toggle-gpui-mcp", "启用 MCP")
                .h(space::ROW_HEIGHT)
                .px(space::MD)
        }
        .when(starting, |button| button.opacity(0.5))
        .when(!starting, |button| {
            button.on_click(cx.listener(|view, _, window, cx| view.toggle_mcp(window, cx)))
        });

        let startup_choices = setting_choices()
            .when(self.backend.is_none(), |choices| choices.opacity(0.5))
            .children(
                [
                    ("mcp-auto-start-off", "关闭", false),
                    ("mcp-auto-start-on", "开启", true),
                ]
                .map(|(id, label, enabled)| {
                    control(id, label, self.mcp_auto_start == enabled).when(
                        self.backend.is_some() && self.mcp_auto_start != enabled,
                        |button| {
                            button.on_click(
                                cx.listener(|view, _, _, cx| view.toggle_mcp_auto_start(cx)),
                            )
                        },
                    )
                }),
            );

        div()
            .flex()
            .flex_col()
            .gap(space::MD)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(space::SM)
                    .child(
                        div()
                            .text_size(text::HEADING)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("本机 MCP 服务"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(space::XS)
                            .text_color(rgb(color::MUTED))
                            .text_size(text::DETAIL)
                            .child(dot(tint))
                            .child(status),
                    )
                    .child(div().flex_1())
                    .child(service_button),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(space::SM)
                    .child(
                        div()
                            .text_size(text::DETAIL)
                            .text_color(rgb(color::MUTED))
                            .child("连接端点"),
                    )
                    .child(
                        div()
                            .h(space::ROW_HEIGHT)
                            .flex()
                            .items_center()
                            .px(space::SM)
                            .rounded(space::CONTROL_RADIUS)
                            .border_1()
                            .border_color(rgb(color::LINE))
                            .bg(rgb(color::TOOLBAR))
                            .child(
                                div()
                                    .min_w(px(0.))
                                    .truncate()
                                    .font_family(text::MONO_FONT)
                                    .text_size(text::DETAIL)
                                    .child(endpoint),
                            ),
                    )
                    .when(running, |section| {
                        section.child(
                            div()
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .gap(space::SM)
                                .child(
                                    primary_control("copy-mcp-json", "复制 MCP JSON")
                                        .h(space::ROW_HEIGHT)
                                        .px(space::MD)
                                        .on_click(
                                            cx.listener(|view, _, _, cx| view.copy_mcp_json(cx)),
                                        ),
                                )
                                .child(setting_action("copy-mcp-endpoint", "复制端点").on_click(
                                    cx.listener(|view, _, _, cx| {
                                        if let Some(server) = view
                                            .mcp_server
                                            .as_ref()
                                            .filter(|server| server.is_running())
                                        {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                server.endpoint(),
                                            ));
                                        }
                                    }),
                                ))
                                .child(setting_action("copy-mcp-token", "复制访问令牌").on_click(
                                    cx.listener(|view, _, _, cx| {
                                        if let Some(server) = view
                                            .mcp_server
                                            .as_ref()
                                            .filter(|server| server.is_running())
                                        {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                server.token().to_owned(),
                                            ));
                                        }
                                    }),
                                )),
                        )
                    }),
            )
            .child(
                div()
                    .text_size(text::DETAIL)
                    .text_color(rgb(color::MUTED))
                    .child("接管和发送终端输入需在桌面确认。独立 AI 终端可以单独授权只读排查。"),
            )
            .when_some(self.mcp_error.clone(), |panel, error| {
                panel.child(
                    div()
                        .text_color(rgb(color::DANGER))
                        .text_size(text::DETAIL)
                        .child(error),
                )
            })
            .child(
                div()
                    .border_t_1()
                    .border_color(rgb(color::LINE))
                    .pt(space::SM)
                    .child(
                        setting_row("启动程序时自动开启 MCP")
                            .h(space::TOOLBAR_HEIGHT)
                            .border_b_0()
                            .child(startup_choices),
                    ),
            )
            .when_some(self.mcp_preference_error.clone(), |panel, error| {
                panel.child(
                    div()
                        .text_color(rgb(color::DANGER))
                        .text_size(text::DETAIL)
                        .child(error),
                )
            })
    }

    fn copy_mcp_json(&self, cx: &Context<Self>) {
        let Some(server) = self
            .mcp_server
            .as_ref()
            .filter(|server| server.is_running())
        else {
            return;
        };
        let config = serde_json::json!({
            "mcpServers": {
                "OrbitShell": {
                    "type": "http",
                    "url": server.endpoint(),
                    "headers": {"Authorization": format!("Bearer {}", server.token())},
                },
            },
        });
        if let Ok(json) = serde_json::to_string_pretty(&config) {
            cx.write_to_clipboard(ClipboardItem::new_string(json));
        }
    }

    fn toggle_mcp(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.mcp_start.is_some() {
            return;
        }
        if self
            .mcp_server
            .take()
            .is_some_and(|server| server.is_running())
        {
            self.mcp_error = None;
            cx.notify();
            return;
        }
        self.start_mcp(mcp::PORT, window, cx);
    }

    fn toggle_mcp_auto_start(&mut self, cx: &mut Context<Self>) {
        let Some(backend) = self.backend.as_ref() else {
            return;
        };
        let enabled = !self.mcp_auto_start;
        match backend.set_mcp_auto_start(enabled) {
            Ok(()) => {
                self.mcp_auto_start = enabled;
                self.mcp_preference_error = None;
            }
            Err(error) => self.mcp_preference_error = Some(format!("{error:#}")),
        }
        cx.notify();
    }

    pub(super) fn start_mcp(&mut self, port: u16, window: &Window, cx: &mut Context<Self>) {
        let Some(backend) = self.backend.as_ref() else {
            self.mcp_error = Some("本机配置暂不可用".into());
            cx.notify();
            return;
        };
        self.mcp_error = None;
        let task = backend.runtime_handle().spawn(mcp::start(
            backend.mcp_catalog(),
            self.transfers.clone(),
            self.mcp_desktop.clone(),
            self.mcp_workspace.clone(),
            port,
        ));
        self.mcp_start = Some(cx.spawn_in(window, async move |view, cx| {
            let result = task
                .await
                .unwrap_or_else(|_| Err("MCP 启动任务已中断".into()));
            let _ = view.update_in(cx, |view, _, cx| {
                view.mcp_start = None;
                match result {
                    Ok(server) => view.mcp_server = Some(server),
                    Err(error) => view.mcp_error = Some(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    #[cfg(test)]
    pub(crate) fn show_mcp_smoke(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.tool = Some(super::Tool::Ai);
        self.open_dialog(Dialog::Settings(super::SettingsTab::Mcp), window, cx);
        self.start_mcp(0, window, cx);
    }

    #[cfg(test)]
    pub(crate) fn mcp_smoke_state(&self) -> Option<(u16, zeroize::Zeroizing<String>)> {
        let server = self
            .mcp_server
            .as_ref()
            .filter(|server| server.is_running())?;
        Some((
            server.port(),
            zeroize::Zeroizing::new(server.token().to_owned()),
        ))
    }
}
