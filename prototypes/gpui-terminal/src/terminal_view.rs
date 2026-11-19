use std::{ops::Range, time::Duration};

use alacritty_terminal::term::TermMode;
use gpui::{
    Bounds, Context, ElementInputHandler, Entity, EntityInputHandler, EventEmitter, FocusHandle,
    KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, Point, ScrollWheelEvent,
    Subscription, Task, UTF16Selection, Window, canvas, div, prelude::*, px, size,
};

#[path = "terminal_directory.rs"]
mod directory;
#[path = "terminal_menu.rs"]
mod menu;
use directory::{DirectoryQuery, PromptInput};
use orbitshell_ssh::ai_shell::AiShell;
use std::sync::Arc;
use unicode_width::UnicodeWidthStr as _;
use zeroize::Zeroizing;

use crate::{
    ssh_backend::TerminalConnection,
    ssh_keys,
    terminal::{BURST_LINES, GridSize, Terminal},
    theme::{self, UiTheme, color, rgb, space, text},
    viewport::Viewport,
};

pub const DEFAULT_FONT_SIZE: u8 = 13;
const MIN_FONT_SIZE: u8 = 11;
const MAX_FONT_SIZE: u8 = 24;

#[derive(Clone, Copy)]
pub struct TerminalAppearance {
    pub font_size: u8,
    pub palette: Option<UiTheme>,
}

impl Default for TerminalAppearance {
    fn default() -> Self {
        Self {
            font_size: DEFAULT_FONT_SIZE,
            palette: None,
        }
    }
}

impl TerminalAppearance {
    pub fn set_font_size(&mut self, size: u8, cx: &mut Context<Self>) {
        let size = size.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE);
        if self.font_size != size {
            self.font_size = size;
            cx.notify();
        }
    }

    pub fn adjust_font_size(&mut self, offset: i8, cx: &mut Context<Self>) {
        self.set_font_size(self.font_size.saturating_add_signed(offset), cx);
    }
}

pub struct TerminalView {
    terminal: Terminal,
    viewport: Viewport,
    appearance: Entity<TerminalAppearance>,
    _appearance_subscription: Subscription,
    focus: FocusHandle,
    burst_task: Option<Task<()>>,
    composition: Composition,
    input_enabled: bool,
    remote: Option<TerminalConnection>,
    ai_shell: Option<Arc<AiShell>>,
    remote_status: String,
    surface_bounds: Option<Bounds<Pixels>>,
    selecting: bool,
    menu: Option<menu::TerminalMenu>,
    prompt_input: PromptInput,
    directory_query: Option<DirectoryQuery>,
}

pub struct SubmittedLine(pub String);
pub struct FindSelection(pub String);
pub struct SaveSelection(pub String);

impl EventEmitter<SubmittedLine> for TerminalView {}
impl EventEmitter<FindSelection> for TerminalView {}
impl EventEmitter<SaveSelection> for TerminalView {}

impl TerminalView {
    pub fn new(
        window: &Window,
        cx: &mut Context<Self>,
        address: &str,
        appearance: Entity<TerminalAppearance>,
    ) -> Self {
        let appearance_subscription = cx.observe(&appearance, |_, _, cx| cx.notify());
        Self {
            terminal: Terminal::new(address),
            viewport: Viewport::new(window, appearance.read(cx).font_size),
            appearance,
            _appearance_subscription: appearance_subscription,
            focus: cx.focus_handle(),
            burst_task: None,
            composition: Composition::default(),
            input_enabled: true,
            remote: None,
            ai_shell: None,
            remote_status: String::new(),
            surface_bounds: None,
            selecting: false,
            menu: None,
            prompt_input: PromptInput::Ready,
            directory_query: None,
        }
    }

    pub fn focus(&self, window: &mut Window) {
        self.focus.focus(window);
    }

    pub fn input_enabled(&self) -> bool {
        self.input_enabled && self.mcp_binding().is_none() && self.directory_query.is_none()
    }

    pub fn mcp_binding(&self) -> Option<crate::mcp_terminal::Binding> {
        self.remote.as_ref()?.control.binding()
    }

    pub fn release_mcp(&self, id: &str) {
        if let Some(remote) = &self.remote {
            remote.control.release(id);
        }
    }

    pub fn handoff_mcp(&self, agent: &crate::mcp_terminal::AgentInfo) -> Result<(), &'static str> {
        let remote = self.remote.as_ref().ok_or("此会话没有 SSH 终端")?;
        if !remote
            .live
            .connected
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return Err("SSH 会话尚未连接");
        }
        remote
            .control
            .claim(agent.binding(&remote.live))
            .map(|_| ())
    }

    pub fn enable_ai_view(&mut self, shell: Arc<AiShell>, cx: &mut Context<Self>) {
        let size = self.terminal.size;
        self.terminal = Terminal::empty(size, None);
        self.ai_shell = Some(shell);
        self.set_input_enabled(false, cx);
    }

    pub fn control_changed(&mut self, cx: &mut Context<Self>) {
        if self.mcp_binding().is_some() {
            self.composition = Composition::default();
            self.finish_directory(Err("终端已交给 AI，目录查询已结束"));
        }
        cx.notify();
    }

    pub fn pty_size(&self) -> (u32, u32) {
        (
            self.terminal.size.columns.clamp(20, 500) as u32,
            self.terminal.size.rows.clamp(5, 300) as u32,
        )
    }

    pub fn attach(&mut self, remote: TerminalConnection, cx: &mut Context<Self>) {
        self.stop_burst();
        let (cols, rows) = self.pty_size();
        self.terminal = Terminal::empty(
            GridSize::new(cols as usize, rows as usize),
            Some(remote.input.clone()),
        );
        self.remote = Some(remote);
        self.prompt_input = PromptInput::Ready;
        self.set_remote_status("正在连接 SSH…".into(), false, cx);
    }

    pub fn set_remote_status(&mut self, message: String, connected: bool, cx: &mut Context<Self>) {
        self.remote_status = message;
        self.set_input_enabled(connected, cx);
    }

    fn resize_grid(&mut self, size: GridSize, cx: &mut Context<Self>) {
        if let Some(shell) = &self.ai_shell {
            shell.resize(size.columns as u32, size.rows as u32);
        }
        if size != self.terminal.size {
            self.terminal.resize(size);
            cx.notify();
        }
    }

    pub fn receive_remote(&mut self, data: &[u8], cx: &mut Context<Self>) {
        self.terminal.receive(data);
        if self.remote.is_some() && !matches!(self.prompt_input, PromptInput::Ready) {
            self.prompt_input
                .received(data, self.terminal.at_shell_prompt());
        }
        if let Some(result) = self
            .directory_query
            .as_mut()
            .and_then(|query| query.receive(data))
        {
            self.finish_directory(result);
        }
        cx.notify();
    }

    fn send_remote(&mut self, data: Vec<u8>, cx: &mut Context<Self>) -> bool {
        if !self.input_enabled() {
            return false;
        }
        if let Some(remote) = &self.remote {
            let mut prompt = self.prompt_input;
            prompt.sent(&data);
            if let Err(error) = remote.input.try_send(Zeroizing::new(data)) {
                self.remote_status =
                    if matches!(error, tokio::sync::mpsc::error::TrySendError::Full(_)) {
                        "终端输入队列已满，请重试".into()
                    } else {
                        "连接已关闭".into()
                    };
                cx.notify();
            } else {
                self.prompt_input = prompt;
                self.terminal.clear_selection();
                self.terminal.scroll_to_bottom();
                cx.notify();
                return true;
            }
        }
        false
    }

    pub fn set_input_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if !enabled {
            self.stop_burst();
            self.composition = Composition::default();
            self.finish_directory(Err("会话已断开，无法定位终端目录"));
        }
        self.input_enabled = enabled;
        cx.notify();
    }

    pub fn request_working_directory(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Result<tokio::sync::oneshot::Receiver<Result<String, &'static str>>, &'static str> {
        if !self.input_enabled() {
            return Err("终端暂不可输入，无法查询目录");
        }
        let remote = self.remote.as_ref().ok_or("请先连接 SSH")?;
        if !self.composition.text.is_empty()
            || !matches!(self.prompt_input, PromptInput::Ready)
            || !self.terminal.at_shell_prompt()
        {
            return Err("请先等待命令结束，并回到没有未提交内容的 Shell 提示符");
        }
        let (query, command, result) = DirectoryQuery::start();
        remote
            .input
            .try_send(Zeroizing::new(command))
            .map_err(|_| "终端输入队列不可用，请重试")?;
        let id = query.id;
        self.directory_query = Some(query);
        cx.spawn(async move |view, cx| {
            cx.background_executor().timer(Duration::from_secs(8)).await;
            let _ = view.update(cx, |view, cx| {
                if view
                    .directory_query
                    .as_ref()
                    .is_some_and(|query| query.id == id)
                {
                    view.finish_directory(Err("查询终端目录超时，文件面板保持原位置"));
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
        Ok(result)
    }

    fn finish_directory(&mut self, result: Result<String, &'static str>) {
        if let Some(query) = self.directory_query.take() {
            let _ = query.sender.send(result);
        }
    }

    pub fn find(&mut self, query: &str, backwards: bool, cx: &mut Context<Self>) -> bool {
        let found = self.terminal.find(query, backwards);
        cx.notify();
        found
    }

    pub fn copy_visible(&self, cx: &mut Context<Self>) {
        let text = self
            .terminal
            .selected_text()
            .unwrap_or_else(|| self.terminal.visible_text());
        if !text.is_empty() {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
        }
    }

    fn begin_selection(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus.focus(window);
        self.menu = None;
        let Some(bounds) = self.surface_bounds else {
            return;
        };
        let (cell, side) = self
            .viewport
            .selection_position(bounds, event.position, &self.terminal);
        self.terminal
            .start_selection(cell, side, event.click_count, event.modifiers.shift);
        self.selecting = true;
        cx.notify();
    }

    fn drag_selection(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selecting || !event.dragging() {
            return;
        }
        let Some(bounds) = self.surface_bounds else {
            return;
        };
        if event.position.y < bounds.origin.y {
            self.terminal.scroll(1);
        } else if event.position.y > bounds.origin.y + bounds.size.height {
            self.terminal.scroll(-1);
        }
        let (cell, side) = self
            .viewport
            .selection_position(bounds, event.position, &self.terminal);
        self.terminal.extend_selection(cell, side);
        cx.notify();
    }

    pub fn paste(&mut self, cx: &mut Context<Self>) {
        if self.remote.is_some() {
            if !self.input_enabled() {
                return;
            }
            if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                let text = Zeroizing::new(text);
                if text.len() > 1024 * 1024 {
                    self.remote_status = "单次粘贴过大，请分批输入".into();
                    cx.notify();
                    return;
                }
                let bracketed = self.terminal.mode().contains(TermMode::BRACKETED_PASTE);
                let text = Zeroizing::new(text.replace("\r\n", "\r").replace('\n', "\r"));
                let mut data = Vec::with_capacity(text.len() + if bracketed { 12 } else { 0 });
                if bracketed {
                    data.extend_from_slice(b"\x1b[200~");
                }
                data.extend(text.bytes().filter(|byte| !bracketed || *byte != 0x1b));
                if bracketed {
                    data.extend_from_slice(b"\x1b[201~");
                }
                if !text.is_empty() {
                    self.send_remote(data, cx);
                }
            }
            return;
        }
        if self.input_enabled
            && !self.burst_running()
            && !self.terminal.in_alternate_screen()
            && let Some(text) = cx.read_from_clipboard().and_then(|item| item.text())
        {
            self.terminal.insert_text(&text);
            cx.notify();
        }
    }

    pub fn draft_command(&mut self, command: &str, cx: &mut Context<Self>) -> bool {
        if !self.input_enabled() || self.burst_running() || self.terminal.in_alternate_screen() {
            return false;
        }
        if self.remote.is_some() {
            if !matches!(self.prompt_input, PromptInput::Ready)
                || !self.composition.text.is_empty()
                || !self.terminal.at_shell_prompt()
                || command.chars().any(|c| c.is_control() && c != '\n')
            {
                return false;
            }
            let bracketed = self.terminal.mode().contains(TermMode::BRACKETED_PASTE);
            if command.contains('\n') && !bracketed {
                return false;
            }
            let data = if bracketed {
                format!("\x1b[200~{command}\x1b[201~").into_bytes()
            } else {
                command.as_bytes().to_vec()
            };
            return self.send_remote(data, cx);
        }
        if self.ai_shell.is_some() || self.terminal.has_input() {
            return false;
        }
        self.terminal.insert_text(command);
        cx.notify();
        true
    }

    pub fn clear_display(&mut self, cx: &mut Context<Self>) {
        self.terminal.clear_selection();
        if self.remote.is_some() || self.ai_shell.is_some() {
            self.terminal.clear_remote_display();
            cx.notify();
            return;
        }
        if !self.burst_running() && !self.terminal.in_alternate_screen() {
            self.terminal.clear_display();
            cx.notify();
        }
    }

    fn burst_running(&self) -> bool {
        self.burst_task.is_some() && self.terminal.burst_lines < BURST_LINES
    }

    fn toggle_burst(&mut self, cx: &mut Context<Self>) {
        if self.burst_running() {
            self.stop_burst();
        } else {
            if self.terminal.in_alternate_screen() {
                self.terminal.toggle_alternate_screen();
            }
            self.composition = Composition::default();
            self.terminal.begin_burst();
            self.burst_task = Some(cx.spawn(async move |view, cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(16))
                        .await;
                    let more = view.update(cx, |view, cx| {
                        let more = view.terminal.burst_batch();
                        if !more {
                            view.terminal.end_burst();
                        }
                        cx.notify();
                        more
                    });
                    if !matches!(more, Ok(true)) {
                        break;
                    }
                }
            }));
        }
        cx.notify();
    }

    fn stop_burst(&mut self) {
        if self.burst_running() {
            self.burst_task.take();
            self.terminal.end_burst();
        }
    }

    fn toggle_alternate_screen(&mut self, cx: &mut Context<Self>) {
        self.stop_burst();
        self.composition = Composition::default();
        self.terminal.toggle_alternate_screen();
        cx.notify();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.handle_terminal_menu_key(event, window, cx) {
            cx.stop_propagation();
            return;
        }
        let key = event.keystroke.key.as_str();
        let modifiers = event.keystroke.modifiers;
        if modifiers.control && !modifiers.alt {
            if key == "c" && (modifiers.shift || self.terminal.selected_text().is_some()) {
                self.copy_visible(cx);
                cx.stop_propagation();
                return;
            }
            let adjustment = match key {
                "=" | "+" => Some(1),
                "-" | "_" => Some(-1),
                "0" => Some(0),
                _ => None,
            };
            if let Some(offset) = adjustment {
                self.appearance.update(cx, |appearance, cx| {
                    if offset == 0 {
                        appearance.set_font_size(DEFAULT_FONT_SIZE, cx);
                    } else {
                        appearance.adjust_font_size(offset, cx);
                    }
                });
                cx.stop_propagation();
                return;
            }
        }
        if self.remote.is_some() {
            let handled = if modifiers.control && !modifiers.alt && key == "v" {
                self.paste(cx);
                true
            } else if let Some(bytes) = ssh_keys::encode(&event.keystroke, self.terminal.mode()) {
                self.send_remote(bytes, cx);
                true
            } else {
                false
            };
            if handled {
                cx.stop_propagation();
            }
            return;
        }
        let alternate = self.terminal.in_alternate_screen();
        match key {
            "f6" if !event.is_held && self.input_enabled => self.toggle_alternate_screen(cx),
            "f7" if !event.is_held && self.input_enabled => self.toggle_burst(cx),
            "escape" if alternate => self.toggle_alternate_screen(cx),
            "up" if alternate => self.terminal.move_selection(-1),
            "down" if alternate => self.terminal.move_selection(1),
            "pageup" if !alternate => self.terminal.scroll(self.terminal.size.rows as i32),
            "pagedown" if !alternate => self.terminal.scroll(-(self.terminal.size.rows as i32)),
            "enter" if self.input_enabled && !alternate && !self.burst_running() => {
                let input = self.terminal.submit_line();
                if !input.trim().is_empty() {
                    cx.emit(SubmittedLine(input));
                }
            }
            "backspace" if self.input_enabled && !alternate && !self.burst_running() => {
                self.terminal.backspace()
            }
            "v" if !alternate
                && !self.burst_running()
                && event.keystroke.modifiers.control
                && !event.keystroke.modifiers.alt =>
            {
                self.paste(cx);
            }
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn on_scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.terminal.in_alternate_screen()
            || self.remote.is_some() && self.terminal.mode().contains(TermMode::ALT_SCREEN)
        {
            return;
        }
        let delta =
            event.delta.pixel_delta(self.viewport.line_height).y / self.viewport.line_height;
        if delta != 0. {
            self.terminal
                .scroll((delta.abs().ceil() as i32) * delta.signum() as i32);
            cx.notify();
        }
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let appearance = *self.appearance.read(cx);
        self.viewport.set_font_size(window, appearance.font_size);
        self.viewport.dark = appearance.palette.unwrap_or_else(theme::ui_theme) == UiTheme::Dark;
        let alternate = self.terminal.in_alternate_screen();
        let running = self.burst_running();
        let dark = self.viewport.dark;
        let local = self.remote.is_none() && self.ai_shell.is_none();
        let lines = self.terminal.burst_lines;
        let grid = self.terminal.size;
        let measure_view = cx.entity();
        let paint_view = measure_view.clone();
        let terminal_surface = canvas(
            move |bounds, window, cx| {
                let view = measure_view.read(cx);
                let mut grid_size = view.viewport.grid_size(bounds);
                if view.remote.is_some() || view.ai_shell.is_some() {
                    grid_size = GridSize::new(
                        grid_size.columns.clamp(20, 500),
                        grid_size.rows.clamp(5, 300),
                    );
                }
                // The release/cancel redraw measures the final bounds even when the local
                // grid already has that size. Keep intermediate drag frames off bastion PTYs.
                if let Some(remote) = &view.remote
                    && !(remote.live.is_temporary && cx.has_active_drag())
                {
                    let measured = (grid_size.columns as u32, grid_size.rows as u32);
                    remote.size.send_if_modified(|current| {
                        if *current == measured {
                            return false;
                        }
                        *current = measured;
                        true
                    });
                }
                if grid_size != view.terminal.size {
                    let view = measure_view.downgrade();
                    window.on_next_frame(move |_, cx| {
                        let _ = view.update(cx, |view, cx| {
                            view.resize_grid(grid_size, cx);
                        });
                    });
                }
            },
            move |bounds, (), window, cx| {
                paint_view.update(cx, |view, cx| {
                    view.surface_bounds = Some(bounds);
                    window.handle_input(
                        &view.focus,
                        ElementInputHandler::new(bounds, cx.entity()),
                        cx,
                    );
                    let focused = window.is_window_active() && view.focus.is_focused(window);
                    view.viewport.paint(
                        bounds,
                        &view.terminal,
                        focused,
                        &view.composition.text,
                        window,
                        cx,
                    );
                });
            },
        );

        let hint = if let Some(shell) = &self.ai_shell {
            shell.status().message.into()
        } else if self.mcp_binding().is_some() {
            "MCP 已接管 · 可查看输出或交回终端".into()
        } else if self.directory_query.is_some() {
            "正在查询终端当前目录…".into()
        } else if !local {
            self.remote_status.clone()
        } else if !self.input_enabled {
            "本地会话已断开 · 可滚动或复制已有内容".into()
        } else if alternate {
            "↑ ↓ 或 j / k 移动 · Esc 或 q 返回 · F6 切换".into()
        } else if running {
            "滚轮或 Page Up / Down 查看历史 · F7 停止".into()
        } else {
            "Enter 换行 · Ctrl+V 粘贴".into()
        };
        let output = if lines == 0 || alternate {
            String::new()
        } else {
            format!("{lines} / {BURST_LINES} 行 · ")
        };

        div()
            .id("terminal-view")
            .flex_1()
            .min_h(px(0.))
            .min_w(px(0.))
            .flex()
            .flex_col()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key_down))
            .on_mouse_move(cx.listener(Self::drag_selection))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|view, _, _, _| view.selecting = false),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|view, _, _, _| view.selecting = false),
            )
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_hidden()
                    .when(dark, |surface| {
                        surface
                            .mx(space::LG)
                            .mt(space::MD)
                            .rounded(space::CONTROL_RADIUS)
                    })
                    .p(space::LG)
                    .bg(gpui::rgb(self.viewport.background()))
                    .cursor(gpui::CursorStyle::IBeam)
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::begin_selection))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|view, event: &MouseDownEvent, window, cx| {
                            view.open_terminal_menu(event.position, false, window, cx);
                            cx.stop_propagation();
                        }),
                    )
                    .on_scroll_wheel(cx.listener(Self::on_scroll_wheel))
                    .child(terminal_surface.size_full()),
            )
            .when_some(self.menu.as_ref(), |root, menu| {
                root.child(gpui::deferred(self.render_terminal_menu(menu, cx)))
            })
            .child(
                div()
                    .h(space::FOOTER_HEIGHT)
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(space::SM)
                    .px(space::LG)
                    .text_size(text::DETAIL)
                    .text_color(rgb(color::MUTED))
                    .child(div().flex_1().min_w(px(0.)).truncate().child(hint))
                    .when(local, |footer| {
                        footer.child(
                            div()
                                .flex_none()
                                .flex()
                                .items_center()
                                .gap(space::XS)
                                .child(
                                    theme::control(
                                        "alternate-screen",
                                        if alternate {
                                            "返回回显"
                                        } else {
                                            "全屏 F6"
                                        },
                                        alternate,
                                    )
                                    .on_click(cx.listener(
                                        |view, _, window, cx| {
                                            view.focus.focus(window);
                                            if view.input_enabled {
                                                view.toggle_alternate_screen(cx);
                                            }
                                        },
                                    )),
                                )
                                .child(
                                    theme::control(
                                        "output-burst",
                                        if running { "停止 F7" } else { "高频 F7" },
                                        running,
                                    )
                                    .on_click(cx.listener(
                                        |view, _, window, cx| {
                                            view.focus.focus(window);
                                            if view.input_enabled {
                                                view.toggle_burst(cx);
                                            }
                                        },
                                    )),
                                ),
                        )
                    })
                    .child(
                        div()
                            .flex_none()
                            .child(format!("{output}{} 列 × {} 行", grid.columns, grid.rows)),
                    ),
            )
    }
}

#[derive(Default)]
struct Composition {
    text: String,
    selection: Range<usize>,
}

impl EntityInputHandler for TerminalView {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let text = &self.composition.text;
        let start = byte_offset(text, range.start);
        let end = byte_offset(text, range.end).max(start);
        *adjusted = Some(text[..start].encode_utf16().count()..text[..end].encode_utf16().count());
        Some(text[start..end].into())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.composition.selection.clone(),
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        (!self.composition.text.is_empty()).then(|| 0..self.composition.text.encode_utf16().count())
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.composition = Composition::default();
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.composition = Composition::default();
        if self.input_enabled() && !self.burst_running() && !text.is_empty() {
            if self.remote.is_some() {
                self.send_remote(text.as_bytes().to_vec(), cx);
            } else {
                self.terminal.insert_text(text);
            }
        }
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        selection: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.input_enabled() || self.burst_running() || self.terminal.in_alternate_screen() {
            return;
        }
        self.terminal.scroll_to_bottom();
        let text: String = text
            .chars()
            .filter(|character| !character.is_control())
            .collect();
        let end = text.encode_utf16().count();
        let selection =
            selection.map_or(end..end, |range| range.start.min(end)..range.end.min(end));
        self.composition = Composition { text, selection };
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let cursor = self
            .viewport
            .cursor_bounds(bounds, &self.terminal)
            .unwrap_or_else(|| Bounds::new(bounds.origin, size(px(1.), self.viewport.line_height)));
        let text = &self.composition.text;
        let start = byte_offset(text, range.start);
        let end = byte_offset(text, range.end).max(start);
        Some(
            self.viewport
                .range_at_cursor(cursor, text[..start].width(), text[start..end].width()),
        )
    }

    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

fn byte_offset(text: &str, utf16_offset: usize) -> usize {
    let mut position = 0;
    for (index, character) in text.char_indices() {
        position += character.len_utf16();
        if position > utf16_offset {
            return index;
        }
    }
    text.len()
}
