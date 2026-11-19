use std::{sync::Arc, time::Duration};

use gpui::{Context, Div, ElementId, Entity, MouseButton, Window, div, prelude::*, px};

use super::{
    Dialog, Page, SettingsTab, Workbench, actions::dialog_tab, control, dot, icon_control,
    icon_control_with_size,
};
use crate::{
    icons::{Icon, Tooltip, icon},
    mcp_terminal::Dedicated,
    terminal_view::TerminalView,
    theme::{color, danger_control, rgb, space, text},
};

pub(super) struct AiView {
    pub(super) terminal: Arc<Dedicated>,
    pub(super) view: Entity<TerminalView>,
    cursor: Option<u64>,
}

impl Workbench {
    pub(super) fn observe_ai(&self, window: &Window, cx: &mut Context<Self>) {
        let mut updates = self.mcp_workspace.subscribe();
        cx.spawn_in(window, async move |view, cx| {
            while updates.changed().await.is_ok() {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                updates.borrow_and_update();
                if view
                    .update_in(cx, |view, window, cx| {
                        view.sync_ai_views(window, cx);
                        view.update_ai_output(cx);
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

    fn sync_ai_views(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let terminals = self.mcp_workspace.terminals();
        self.ai_views.retain(|tab| {
            terminals
                .iter()
                .any(|terminal| terminal.id == tab.terminal.id)
        });
        self.dismissed_ai
            .retain(|id| terminals.iter().any(|terminal| terminal.id == *id));
        for terminal in terminals {
            if terminal.agent.connected() && terminal.shell.read_only_authorized() {
                self.dismissed_ai.retain(|id| *id != terminal.id);
            }
            if self.dismissed_ai.contains(&terminal.id)
                || self
                    .ai_views
                    .iter()
                    .any(|tab| tab.terminal.id == terminal.id)
            {
                continue;
            }
            let view = self.new_terminal("", window, cx);
            view.update(cx, |view, cx| {
                view.enable_ai_view(terminal.shell.clone(), cx)
            });
            let auto_select = self.page == Page::Terminal
                && self.active_session().is_none()
                && self.sessions.iter().any(|session| {
                    !session.show_tab
                        && session
                            .remote
                            .as_ref()
                            .is_some_and(|remote| remote.id == terminal.session_id)
                });
            if auto_select {
                self.active_ai = Some(terminal.id.clone());
                self.page = Page::Ai;
            }
            self.ai_views.push(AiView {
                terminal,
                view,
                cursor: None,
            });
            if auto_select && self.dialog.is_none() {
                self.focus_active(window, cx);
            }
        }
        if self.page == Page::Ai && self.current_ai_view().is_none() {
            self.active_ai = self.ai_views.first().map(|tab| tab.terminal.id.clone());
            if self.active_ai.is_none() {
                self.page = Page::Terminal;
            }
        }
    }

    fn update_ai_output(&mut self, cx: &mut Context<Self>) {
        if self.page != Page::Ai {
            return;
        }
        let Some(tab) = self
            .ai_views
            .iter_mut()
            .find(|tab| self.active_ai.as_deref() == Some(tab.terminal.id.as_str()))
        else {
            return;
        };
        let mut bytes = Vec::new();
        let mut truncated = false;
        for _ in 0..16 {
            let output = tab.terminal.shell.read(tab.cursor, 65536);
            tab.cursor = Some(output.cursor);
            bytes.extend_from_slice(&output.bytes);
            truncated |= output.truncated;
            if !output.has_more {
                break;
            }
        }
        tab.view.update(cx, |view, cx| {
            if truncated {
                view.receive_remote(
                    b"\x1bc\r\n[OrbitShell] Older output is no longer retained.\r\n",
                    cx,
                );
            }
            if !bytes.is_empty() {
                view.receive_remote(&bytes, cx);
            }
            cx.notify();
        });
    }

    pub(super) fn current_ai_view(&self) -> Option<&AiView> {
        let id = self.active_ai.as_deref()?;
        self.ai_views.iter().find(|view| view.terminal.id == id)
    }

    pub(super) fn open_ai_terminal(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismissed_ai.retain(|dismissed| dismissed != id);
        self.sync_ai_views(window, cx);
        if !self.ai_views.iter().any(|view| view.terminal.id == id) {
            return;
        }
        self.active_ai = Some(id.into());
        self.page = Page::Ai;
        self.update_ai_output(cx);
        self.focus_active(window, cx);
        cx.notify();
    }

    pub(super) fn close_ai_terminal(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.revoke_ai_read_only(id, cx);
        self.dismissed_ai.push(id.into());
        self.ai_views.retain(|view| view.terminal.id != id);
        if self.active_ai.as_deref() == Some(id) {
            self.active_ai = self.ai_views.last().map(|tab| tab.terminal.id.clone());
            if self.page == Page::Ai && (self.active.is_some() || self.active_ai.is_none()) {
                self.page = Page::Terminal;
            }
            self.update_ai_output(cx);
            self.focus_active(window, cx);
        }
        cx.notify();
    }

    fn revoke_ai_read_only(&self, id: &str, cx: &mut Context<Self>) {
        if let Some(tab) = self.ai_views.iter().find(|tab| tab.terminal.id == id) {
            tab.terminal.shell.revoke_read_only();
            cx.notify();
        }
    }

    pub(super) fn ai_tabs(&self, cx: &Context<Self>) -> Vec<Div> {
        self.ai_views
            .iter()
            .enumerate()
            .map(|(index, tab)| {
                let id = tab.terminal.id.clone();
                let close = id.clone();
                let middle = id.clone();
                let revoke = id.clone();
                let active =
                    self.page == Page::Ai && self.active_ai.as_deref() == Some(id.as_str());
                let tint = if active {
                    color::ACCENT_TEXT
                } else {
                    color::MUTED
                };
                let running = tab.terminal.shell.status().status == "running";
                let read_only =
                    tab.terminal.agent.connected() && tab.terminal.shell.read_only_authorized();
                div()
                    .h(space::CONTROL_HEIGHT)
                    .flex()
                    .flex_none()
                    .items_center()
                    .rounded(space::CONTROL_RADIUS)
                    .when(active, |row| row.bg(rgb(color::SELECTED)))
                    .on_mouse_down(
                        MouseButton::Middle,
                        cx.listener(move |view, _, window, cx| {
                            view.close_ai_terminal(&middle, window, cx);
                            cx.stop_propagation();
                        }),
                    )
                    .child(
                        div()
                            .id(ElementId::named_usize("select-ai", index))
                            .h_full()
                            .pl(space::SM)
                            .pr(space::XS)
                            .flex()
                            .items_center()
                            .gap(space::XS)
                            .rounded(space::CONTROL_RADIUS)
                            .cursor_pointer()
                            .text_color(rgb(tint))
                            .hover(move |style| {
                                style.bg(rgb(if active {
                                    color::SELECTED_HOVER
                                } else {
                                    color::HOVER
                                }))
                            })
                            .child(icon(Icon::Bot, tint).size(space::COMPACT_ICON_SIZE))
                            .child(tab.terminal.session_name.clone())
                            .child(dot(if running {
                                color::ACCENT
                            } else {
                                color::SESSION_IDLE
                            }))
                            .tooltip(|_, cx| cx.new(|_| Tooltip("独立 AI 终端")).into())
                            .on_click(cx.listener(
                                move |view, _, window, cx| view.open_ai_terminal(&id, window, cx),
                            )),
                    )
                    .when(read_only, |row| {
                        row.child(
                            control(
                                ElementId::named_usize("revoke-ai-read-only", index),
                                "撤销只读授权",
                                false,
                            )
                            .text_size(text::DETAIL)
                            .on_click(cx.listener(
                                move |view, _, _, cx| {
                                    view.revoke_ai_read_only(&revoke, cx);
                                },
                            )),
                        )
                    })
                    .child(
                        icon_control_with_size(
                            ElementId::named_usize("close-ai", index),
                            Icon::Close,
                            "关闭 AI 标签",
                            active,
                            space::COMPACT_ICON_SIZE,
                        )
                        .size(space::CONTROL_HEIGHT)
                        .on_click(cx.listener(
                            move |view, _, window, cx| view.close_ai_terminal(&close, window, cx),
                        )),
                    )
            })
            .collect()
    }

    pub(super) fn ai_terminal_panel(&self, cx: &Context<Self>) -> Div {
        let Some(tab) = self.current_ai_view() else {
            return div();
        };
        let state = tab.terminal.shell.status();
        let ended = tab.terminal.shell.cancel.is_cancelled();
        let mut label = format!(
            "{} · {} · {}",
            tab.terminal.agent.name, tab.terminal.session_name, state.message
        );
        if let Some(exit) = state.exit_code {
            label.push_str(&format!(
                " · 退出码 {exit} · {:.2} 秒",
                state.elapsed() as f64 / 1000.
            ));
        }
        let search = icon_control(
            "find-in-ai-terminal",
            Icon::Search,
            "查找终端内容 · Ctrl+F",
            false,
        )
        .size(space::CONTROL_HEIGHT)
        .on_click(cx.listener(|view, _, window, cx| view.open_dialog(Dialog::Search, window, cx)));
        div()
            .flex_1()
            .min_h(px(0.))
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(space::SM)
                    .px(space::LG)
                    .py(space::XS)
                    .border_b_1()
                    .border_color(rgb(color::LINE))
                    .text_size(text::DETAIL)
                    .child(div().flex_1().min_w(px(0.)).truncate().child(label))
                    .when_some(
                        state.started.filter(|_| !ended && state.status == "running"),
                        |row, started| {
                            if state.interrupt_pending {
                                return row.child(
                                    div()
                                        .flex_none()
                                        .text_color(rgb(color::MUTED))
                                        .child("正在停止…"),
                                );
                            }
                            let shell = tab.terminal.shell.clone();
                            row.child(
                                control("stop-ai-command", "停止当前命令", false)
                                    .flex_none()
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        if let Err(error) = shell.interrupt(started) {
                                            view.notice = Some(error);
                                            cx.notify();
                                        }
                                    })),
                            )
                        },
                    )
                    .when(!ended, |row| {
                        let terminal = tab.terminal.clone();
                        row.child(
                            danger_control("end-ai-terminal", "结束 AI 终端")
                                .flex_none()
                                .on_click(cx.listener(move |view, _, window, cx| {
                                    if !terminal.shell.cancel.is_cancelled() {
                                        view.open_dialog(Dialog::EndAiTerminal, window, cx);
                                        view.pending_ai_end = Some(terminal.clone());
                                        cx.notify();
                                    }
                                })),
                        )
                    })
                    .child(search),
            )
            .when_some(state.command, |pane, command| {
                pane.child(
                    div()
                        .px(space::LG)
                        .py(space::XS)
                        .truncate()
                        .font_family(text::MONO_FONT)
                        .text_size(text::DETAIL)
                        .child(command),
                )
            })
            .child(tab.view.clone())
    }

    pub(super) fn end_ai_terminal_dialog(&self, cx: &Context<Self>) -> Div {
        let Some(terminal) = &self.pending_ai_end else {
            return div();
        };
        div()
            .p(space::LG)
            .flex()
            .flex_col()
            .gap(space::MD)
            .child(format!("{} · {}", terminal.agent.name, terminal.session_name))
            .child(
                div()
                    .text_color(rgb(color::MUTED))
                    .child("结束后会关闭此独立终端，运行中的命令可能中断。"),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(space::SM)
                    .child(
                        dialog_tab(control("cancel-end-ai", "取消", false), 1).on_click(
                            cx.listener(|view, _, window, cx| view.close_dialog(window, cx)),
                        ),
                    )
                    .child(
                        dialog_tab(danger_control("confirm-end-ai", "结束终端"), 2).on_click(
                            cx.listener(|view, _, window, cx| {
                                if let Some(terminal) = view.pending_ai_end.take() {
                                    terminal.shell.end();
                                }
                                view.close_dialog(window, cx);
                            }),
                        ),
                    ),
            )
    }

    pub(super) fn agent_panel(&self, cx: &Context<Self>) -> Div {
        let agents = self.mcp_workspace.agents();
        let connected = agents.iter().filter(|agent| agent.connected()).count();
        let terminals = self.mcp_workspace.terminals();
        div()
            .flex()
            .flex_col()
            .gap(space::XS)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(format!("{connected} 个 Agent 已连接"))
                    .child(
                        control("open-mcp-settings", "AI 与 MCP 设置", false).on_click(
                            cx.listener(|view, _, window, cx| {
                                view.open_dialog(Dialog::Settings(SettingsTab::Mcp), window, cx);
                            }),
                        ),
                    ),
            )
            .children(agents.iter().map(|agent| {
                let ownership = self
                    .sessions
                    .iter()
                    .filter_map(|session| {
                        session
                            .terminal
                            .read(cx)
                            .mcp_binding()
                            .filter(|binding| binding.agent == agent.id)
                            .map(|_| self.profiles[session.profile].title.as_str())
                    })
                    .collect::<Vec<_>>()
                    .join("、");
                let label = if ownership.is_empty() {
                    format!(
                        "{} {} · {}",
                        agent.name,
                        agent.version,
                        if agent.connected() {
                            "已连接"
                        } else {
                            "已断开"
                        }
                    )
                } else {
                    format!("{} · 接管 {ownership}", agent.name)
                };
                div()
                    .flex()
                    .items_center()
                    .gap(space::SM)
                    .child(dot(if agent.connected() {
                        color::ACCENT
                    } else {
                        color::SESSION_IDLE
                    }))
                    .child(div().truncate().child(label))
            }))
            .children(terminals.iter().enumerate().map(|(index, terminal)| {
                let id = terminal.id.clone();
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(space::SM)
                    .child(div().truncate().child(format!(
                        "{} · {} · {}",
                        terminal.agent.name,
                        terminal.session_name,
                        terminal.shell.status().message
                    )))
                    .child(
                        control(
                            ElementId::named_usize("show-agent-shell", index),
                            "查看终端",
                            false,
                        )
                        .on_click(cx.listener(
                            move |view, _, window, cx| view.open_ai_terminal(&id, window, cx),
                        )),
                    )
            }))
    }

    pub(super) fn handoff_dialog(&self, profile: usize, cx: &Context<Self>) -> Div {
        let agents: Vec<_> = self
            .mcp_workspace
            .agents()
            .into_iter()
            .filter(|agent| agent.connected())
            .collect();
        div()
            .p(space::LG)
            .flex()
            .flex_col()
            .gap(space::SM)
            .child(self.profiles[profile].title.clone())
            .when(agents.is_empty(), |panel| panel.child("暂无已连接的 Agent"))
            .children(agents.into_iter().enumerate().map(|(index, agent)| {
                control(
                    ElementId::named_usize("handoff-agent", index),
                    format!("交给 {}", agent.name),
                    false,
                )
                .on_click(cx.listener(move |view, _, window, cx| {
                    let handed = view
                        .sessions
                        .iter()
                        .find(|session| session.profile == profile)
                        .ok_or("SSH 会话已关闭")
                        .and_then(|session| session.terminal.read(cx).handoff_mcp(&agent));
                    view.close_dialog(window, cx);
                    view.notice = handed.err();
                    cx.notify();
                }))
            }))
    }
}
