use gpui::{
    Context, Div, ElementId, Entity, FontWeight, ImageSource, Keystroke, MouseDownEvent,
    PathBuilder, PathPromptOptions, Resource, SharedString, Stateful, Window, canvas, deferred,
    div, img, point, prelude::*, px,
};
use orbitshell_ssh::quick_command::QuickCommand;

use super::{LoginCredential, Tool, Workbench, control, icon_control, icon_control_with_size};
use crate::{
    BRAND_IMAGE,
    demo::Profile,
    editor_input::EditorInput,
    file_icons::file_icon,
    icons::{Icon, Tooltip, icon},
    ssh_backend::SavedLayout,
    terminal_view::DEFAULT_FONT_SIZE,
    text_field::TextField,
    theme::{self, UiTheme, color, primary_control, rgb, space, text},
};

// Header, borders, and the eight shortcut rows with their existing spacing.
const SETTINGS_DIALOG_HEIGHT: gpui::Pixels = px(412.);

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Page {
    Terminal,
    Ai,
    History,
    Preview,
    Editor,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Menu {
    File,
    Edit,
    View,
    Session,
    Help,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SettingsTab {
    Appearance,
    Shortcuts,
    Mcp,
    Bastion,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum LoginOrigin {
    User,
    Agent,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Dialog {
    Connections,
    ConnectionGroup,
    Profile(Option<usize>),
    Login(usize, LoginOrigin),
    HostKey(usize),
    Settings(SettingsTab),
    Search,
    Transfers,
    UploadConflict,
    DeleteRemoteFile,
    EditorConfirm,
    Exit,
    QuickCommands,
    QuickCommand,
    McpApproval,
    AiHandoff(usize),
    EndAiTerminal,
    About,
}

#[derive(Clone, Copy)]
enum Action {
    Show(Dialog),
    ToggleConnections,
    ToggleFiles,
    ToggleTools,
    Tool(Tool),
    History,
    Copy,
    Paste,
    Clear,
    Connect(bool),
    CloseTab,
    Hide,
    Exit,
}

pub(super) struct QuickCommandDraft {
    id: String,
    editing: bool,
    append_newline: bool,
    return_to_manager: bool,
    name: Entity<TextField>,
    group: Entity<TextField>,
    command: Entity<EditorInput>,
    error: Option<String>,
}

impl Workbench {
    pub(super) fn handle_workspace_key(
        &mut self,
        key: &Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.page == Page::Editor
            && self.dialog.is_none()
            && self.handle_editor_key(key, window, cx)
        {
            return true;
        }
        if key.key == "escape" {
            if self.dialog.is_some() {
                self.close_dialog(window, cx);
                return true;
            }
            if self.command_picker.dismiss_groups() {
                cx.notify();
                return true;
            }
            if self.file_menu.take().is_some() {
                cx.notify();
                return true;
            }
            if self.group_menu.take().is_some() {
                cx.notify();
                return true;
            }
            if self.menu.take().is_some() {
                cx.notify();
                return true;
            }
        }
        if let Some(dialog) = self.dialog {
            return match (dialog, key.key.as_str()) {
                (Dialog::Search, "enter") => {
                    self.find_terminal(key.modifiers.shift, cx);
                    true
                }
                (Dialog::ConnectionGroup, "enter") if self.group_editor_focused(window, cx) => {
                    self.save_group_change(window, cx);
                    true
                }
                (Dialog::Settings(SettingsTab::Bastion), "enter")
                    if self.bastion_link.read(cx).is_focused(window) =>
                {
                    self.submit_bastion_link(window, cx);
                    true
                }
                (Dialog::Profile(index), "enter")
                    if [
                        &self.profile_title,
                        &self.profile_group,
                        &self.profile_address,
                        &self.profile_port,
                        &self.profile_username,
                        &self.profile_key_path,
                    ]
                    .iter()
                    .any(|field| field.read(cx).is_focused(window)) =>
                {
                    self.save_ssh_profile(index, window, cx);
                    true
                }
                (
                    Dialog::Connections
                    | Dialog::ConnectionGroup
                    | Dialog::Profile(_)
                    | Dialog::Login(..)
                    | Dialog::EndAiTerminal,
                    "tab",
                ) => {
                    if key.modifiers.shift {
                        window.focus_prev();
                    } else {
                        window.focus_next();
                    }
                    true
                }
                (Dialog::Login(profile, origin), "enter")
                    if self.login_pending.is_none()
                        && (self.dialog_focus.is_focused(window)
                            || self.password.read(cx).is_focused(window)
                            || self.bastion_username.read(cx).is_focused(window)) =>
                {
                    self.start_remote(
                        profile,
                        origin,
                        self.login_credential == LoginCredential::Saved,
                        window,
                        cx,
                    );
                    true
                }
                _ => key.modifiers.control && key.key == "tab",
            };
        }
        if !key.modifiers.control || key.modifiers.alt {
            return false;
        }
        let action = match key.key.as_str() {
            "t" if key.modifiers.shift => Action::Show(Dialog::Profile(None)),
            "w" if key.modifiers.shift => Action::CloseTab,
            "b" => Action::ToggleConnections,
            "j" => Action::ToggleTools,
            "s" if self.page == Page::Editor && !key.modifiers.shift => {
                self.save_active_editor(window, cx);
                return true;
            }
            "," => Action::Show(Dialog::Settings(SettingsTab::Appearance)),
            "f" if self.page == Page::History => {
                self.history_filter.read(cx).focus(window);
                return true;
            }
            "f" if self.displayed_terminal().is_some() => Action::Show(Dialog::Search),
            _ => return false,
        };
        self.perform(action, window, cx);
        true
    }

    fn perform(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        self.notice = None;
        match action {
            Action::Show(dialog) => self.open_dialog(dialog, window, cx),
            Action::ToggleConnections => {
                self.show_connections = !self.show_connections;
                if !self.show_connections && self.connection_filter.read(cx).is_focused(window) {
                    self.focus_active(window, cx);
                }
            }
            Action::ToggleFiles => self.toggle_files(window, cx),
            Action::ToggleTools => {
                self.tool = if self.tool.is_some() {
                    None
                } else {
                    Some(Tool::Transfers)
                }
            }
            Action::Tool(tool) => self.tool = Some(tool),
            Action::History => self.show_history(window, cx),
            Action::Copy | Action::Paste | Action::Clear => {
                if let Some(terminal) = self.displayed_terminal() {
                    terminal.update(cx, |view, cx| match action {
                        Action::Copy => view.copy_visible(cx),
                        Action::Paste => view.paste(cx),
                        Action::Clear => view.clear_display(cx),
                        _ => unreachable!(),
                    });
                    self.focus_active(window, cx);
                }
            }
            Action::Connect(online) => {
                if let Some(session) = self.active_session() {
                    let profile = session.profile;
                    if let Some(remote) = &session.remote {
                        if online && remote.is_closed() {
                            self.connect_or_prompt(profile, window, cx);
                        } else if remote.awaiting_host() && online {
                            self.open_dialog(Dialog::HostKey(profile), window, cx);
                        } else if !online {
                            remote.live.cancel.cancel();
                        }
                    } else {
                        session
                            .terminal
                            .update(cx, |view, cx| view.set_input_enabled(online, cx));
                        self.log(format!(
                            "{}本地会话 · {}",
                            if online { "恢复" } else { "断开" },
                            self.profiles[profile].title
                        ));
                    }
                }
            }
            Action::CloseTab => self.close_current_tab(window, cx),
            Action::Hide => self.hide_desktop_window(window, cx),
            Action::Exit => self.request_desktop_exit(window, cx),
        }
        cx.notify();
    }

    pub(super) fn show_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.history_open = true;
        self.page = Page::History;
        self.notice = None;
        self.focus_active(window, cx);
        cx.notify();
    }

    fn close_current_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.page {
            Page::History | Page::Preview => self.close_page(self.page, window, cx),
            Page::Editor => self.close_active_editor(window, cx),
            Page::Ai => {
                if let Some(id) = self.active_ai.clone() {
                    self.close_ai_terminal(&id, window, cx);
                }
            }
            Page::Terminal => {
                if let Some(id) = self.active {
                    self.close_session(id, window, cx);
                }
            }
        }
    }

    pub(super) fn open_dialog(
        &mut self,
        dialog: Dialog,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if dialog == Dialog::Search && self.displayed_terminal().is_none() {
            return;
        }
        if self.dialog == Some(dialog) {
            return;
        }
        let previous = self.dialog;
        self.profile_key_picker = None;
        self.login_pending = None;
        if matches!(
            self.dialog,
            Some(
                Dialog::Login(..)
                    | Dialog::HostKey(_)
                    | Dialog::Search
                    | Dialog::ConnectionGroup
                    | Dialog::EndAiTerminal
                    | Dialog::DeleteRemoteFile
                    | Dialog::EditorConfirm
                    | Dialog::QuickCommand
            )
        ) {
            self.close_dialog(window, cx);
        }
        self.dialog = Some(dialog);
        self.menu = None;
        self.notice = None;
        match dialog {
            Dialog::Connections => {
                self.ssh_error = None;
                if !matches!(previous, Some(Dialog::Profile(Some(_)))) {
                    self.management_filter
                        .update(cx, |field, cx| field.set_value("", cx));
                }
                self.management_filter.read(cx).focus(window);
            }
            Dialog::Profile(index) => {
                self.ssh_error = None;
                self.populate_profile(index, cx);
                self.profile_title.read(cx).focus(window);
            }
            Dialog::Login(..) => {
                self.ssh_error = None;
                self.remember_password = false;
                self.login_credential = LoginCredential::Manual;
                self.password
                    .update(cx, |input, cx| input.set_value("", cx));
                self.password.read(cx).focus(window);
            }
            Dialog::Search => {
                self.search_result = None;
                self.search_field.read(cx).focus(window);
            }
            _ => self.dialog_focus.focus(window),
        }
        cx.notify();
    }

    pub(super) fn close_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dialog == Some(Dialog::Exit) && self.desktop.exit_saving.is_some() {
            return;
        }
        self.profile_key_picker = None;
        self.login_pending = None;
        match self.dialog.take() {
            Some(Dialog::Connections) => self.batch_group_selection = None,
            Some(Dialog::ConnectionGroup) => {
                if self.close_group_editor(window, cx) {
                    return;
                }
            }
            Some(Dialog::EndAiTerminal) => self.pending_ai_end = None,
            Some(Dialog::UploadConflict) => self.answer_upload(false),
            Some(Dialog::DeleteRemoteFile) => {
                self.file_delete = None;
            }
            Some(Dialog::EditorConfirm) => self.editors.cancel_confirmation(),
            Some(Dialog::Exit) => self.desktop.exit_error = None,
            Some(Dialog::QuickCommand) => {
                if self
                    .quick_command_draft
                    .take()
                    .is_some_and(|draft| draft.return_to_manager)
                {
                    self.dialog = Some(Dialog::QuickCommands);
                    self.command_manager.focus(window, cx);
                    cx.notify();
                    return;
                }
            }
            Some(Dialog::McpApproval) => {
                if let Some(request) = self.mcp_approval.take() {
                    request.resolve(false);
                }
            }
            Some(Dialog::Settings(SettingsTab::Bastion)) => self.clear_bastion_link(cx),
            Some(Dialog::HostKey(profile)) => self.answer_host(profile, false, window, cx),
            Some(Dialog::Login(..)) => {
                self.pending_bastion = None;
                self.bastion_username
                    .update(cx, |field, cx| field.set_value("", cx));
                self.password
                    .update(cx, |input, cx| input.set_value("", cx));
            }
            Some(Dialog::Search) => {
                if let Some(terminal) = self.displayed_terminal() {
                    terminal.update(cx, |view, cx| {
                        view.find("", false, cx);
                    });
                }
            }
            _ => {}
        }
        self.notice = None;
        self.focus_active(window, cx);
        cx.notify();
    }

    pub(super) fn find_terminal(&mut self, backwards: bool, cx: &mut Context<Self>) {
        let query = self.search_field.read(cx).value().to_owned();
        if let Some(terminal) = self.displayed_terminal() {
            let found = terminal.update(cx, |view, cx| view.find(&query, backwards, cx));
            self.search_result = (!query.is_empty()).then_some(found);
            cx.notify();
        }
    }

    pub(super) fn displayed_terminal(&self) -> Option<&Entity<crate::terminal_view::TerminalView>> {
        match self.page {
            Page::Terminal => self.active_session().map(|session| &session.terminal),
            Page::Ai => self.current_ai_view().map(|tab| &tab.view),
            _ => None,
        }
    }

    pub(super) fn draft_command(
        &mut self,
        command: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.active_session() else {
            self.notice = Some("请先打开终端会话。");
            cx.notify();
            return;
        };
        let inserted = session
            .terminal
            .update(cx, |view, cx| view.draft_command(command, cx));
        self.page = Page::Terminal;
        self.notice = if inserted {
            Some("已填入终端，可编辑后按 Enter 提交。")
        } else {
            Some("请回到可输入的空提示符，再插入快捷指令。")
        };
        self.focus_active(window, cx);
        cx.notify();
    }

    fn close_page(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        match page {
            Page::History => self.history_open = false,
            Page::Preview => self.preview = None,
            Page::Terminal | Page::Ai | Page::Editor => return,
        }
        if self.page == page {
            self.page = Page::Terminal;
            self.focus_active(window, cx);
        }
        cx.notify();
    }

    pub(super) fn utility_tab(
        &self,
        title: impl Into<SharedString>,
        page: Page,
        cx: &Context<Self>,
    ) -> Div {
        let (id, close_id) = if page == Page::History {
            ("history-tab", "close-history-tab")
        } else {
            ("preview-tab", "close-preview-tab")
        };
        div()
            .flex_none()
            .flex()
            .items_center()
            .rounded(space::CONTROL_RADIUS)
            .when(self.page == page, |tab| tab.bg(rgb(color::SELECTED)))
            .child(control(id, title, self.page == page).on_click(cx.listener(
                move |view, _, window, cx| {
                    view.page = page;
                    view.notice = None;
                    view.focus_active(window, cx);
                    cx.notify();
                },
            )))
            .child(
                icon_control(close_id, Icon::Close, "关闭标签", self.page == page)
                    .size(space::CONTROL_HEIGHT)
                    .on_click(
                        cx.listener(move |view, _, window, cx| view.close_page(page, window, cx)),
                    ),
            )
    }

    pub(super) fn menus(&self, cx: &Context<Self>) -> Div {
        let tabs = [
            (Menu::File, "menu-file", "文件"),
            (Menu::Edit, "menu-edit", "编辑"),
            (Menu::View, "menu-view", "视图"),
            (Menu::Session, "menu-session", "会话"),
            (Menu::Help, "menu-help", "帮助"),
        ];
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(2.))
            .children(tabs.map(|(menu, id, label)| {
                let selected = self.menu == Some(menu);
                div()
                    .relative()
                    .child(control(id, label, selected).on_click(cx.listener(
                        move |view, _, _, cx| {
                            view.menu = (!selected).then_some(menu);
                            cx.notify();
                        },
                    )))
                    .when(selected, |item| {
                        item.child(deferred(self.menu_items(menu, cx)))
                    })
            }))
    }

    fn menu_items(&self, menu: Menu, cx: &Context<Self>) -> Stateful<Div> {
        let terminal = self.page == Page::Terminal && self.active_session().is_some();
        let online = self
            .active_session()
            .is_some_and(|session| session.terminal.read(cx).input_enabled());
        let items = match menu {
            Menu::File => vec![
                ("新建连接", Action::Show(Dialog::Profile(None)), true),
                ("管理连接", Action::Show(Dialog::Connections), true),
                (
                    "会话链接与堡垒机",
                    Action::Show(Dialog::Settings(SettingsTab::Bastion)),
                    true,
                ),
                (
                    "设置",
                    Action::Show(Dialog::Settings(SettingsTab::Appearance)),
                    true,
                ),
                ("后台运行", Action::Hide, self.desktop.tray_available()),
                ("退出 OrbitShell", Action::Exit, true),
            ],
            Menu::Edit => vec![
                ("复制可见终端内容", Action::Copy, terminal),
                ("粘贴到终端", Action::Paste, terminal && online),
                ("查找终端内容", Action::Show(Dialog::Search), terminal),
                ("快捷指令", Action::Tool(Tool::Commands), true),
                ("清空终端显示", Action::Clear, terminal),
            ],
            Menu::View => vec![
                (
                    if self.show_connections {
                        "收起连接列表"
                    } else {
                        "显示连接列表"
                    },
                    Action::ToggleConnections,
                    true,
                ),
                (
                    if self.show_files {
                        "收起远程文件"
                    } else {
                        "显示远程文件"
                    },
                    Action::ToggleFiles,
                    true,
                ),
                (
                    if self.tool.is_some() {
                        "折叠工具面板"
                    } else {
                        "展开工具面板"
                    },
                    Action::ToggleTools,
                    true,
                ),
                ("执行历史", Action::History, true),
                ("Agent 连接状态", Action::Tool(Tool::Ai), true),
            ],
            Menu::Session => vec![
                ("打开会话", Action::Show(Dialog::Connections), true),
                ("重新连接", Action::Connect(true), terminal && !online),
                ("断开连接", Action::Connect(false), terminal && online),
                (
                    "关闭当前标签",
                    Action::CloseTab,
                    self.active.is_some() || self.page != Page::Terminal,
                ),
            ],
            Menu::Help => vec![
                (
                    "快捷键",
                    Action::Show(Dialog::Settings(SettingsTab::Shortcuts)),
                    true,
                ),
                ("关于 OrbitShell", Action::Show(Dialog::About), true),
            ],
        };
        div()
            .id("open-workbench-menu")
            .absolute()
            .top(space::CONTROL_HEIGHT)
            .left(px(0.))
            .w(px(186.))
            .p(space::XS)
            .bg(rgb(color::SURFACE))
            .border_1()
            .border_color(rgb(color::LINE))
            .rounded(space::CONTROL_RADIUS)
            .shadow_md()
            .occlude()
            .on_mouse_down_out(cx.listener(|view, event: &MouseDownEvent, _, cx| {
                if event.position.y >= space::TITLEBAR_HEIGHT {
                    view.menu = None;
                    cx.notify();
                }
            }))
            .children(
                items
                    .into_iter()
                    .enumerate()
                    .map(|(index, (title, action, enabled))| {
                        div()
                            .id(ElementId::named_usize("menu-item", index))
                            .h(space::ROW_HEIGHT)
                            .flex()
                            .items_center()
                            .px(space::SM)
                            .rounded(space::CONTROL_RADIUS)
                            .text_color(rgb(if enabled { color::TEXT } else { color::MUTED }))
                            .child(title)
                            .when(enabled, |item| {
                                item.cursor_pointer()
                                    .hover(|style| style.bg(rgb(color::HOVER)))
                                    .on_click(cx.listener(move |view, _, window, cx| {
                                        view.perform(action, window, cx)
                                    }))
                            })
                    }),
            )
    }

    pub(super) fn dialog_layer(
        &self,
        dialog: Dialog,
        window: &Window,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let connection_dialog = matches!(
            dialog,
            Dialog::Connections | Dialog::Profile(_) | Dialog::Login(..)
        );
        let keyboard_dialog = connection_dialog
            || matches!(dialog, Dialog::ConnectionGroup | Dialog::EndAiTerminal);
        let settings_dialog = matches!(dialog, Dialog::Settings(_));
        let fixed_height = settings_dialog || dialog == Dialog::QuickCommands;
        let (title, width) = match dialog {
            Dialog::Connections => ("管理连接", 550.),
            Dialog::ConnectionGroup => (self.group_editor_title(), 470.),
            Dialog::Profile(None) => ("新建 SSH 连接", 510.),
            Dialog::Profile(Some(_)) => ("编辑 SSH 连接", 510.),
            Dialog::Login(_, LoginOrigin::User) => ("确认 SSH 连接", 418.),
            Dialog::Login(_, LoginOrigin::Agent) => ("确认 Agent 发起的 SSH 连接", 418.),
            Dialog::HostKey(_) => ("确认服务器指纹", 470.),
            Dialog::Search => unreachable!(),
            Dialog::Settings(_) => ("设置", 610.),
            Dialog::Transfers => ("传输记录", 510.),
            Dialog::UploadConflict => ("覆盖远程文件", 460.),
            Dialog::DeleteRemoteFile => ("删除远程项目", 460.),
            Dialog::EditorConfirm => (self.editors.confirmation_title(), 460.),
            Dialog::Exit => ("退出 OrbitShell", 460.),
            Dialog::QuickCommands => ("管理快捷指令", 690.),
            Dialog::QuickCommand => {
                let title = match &self.quick_command_draft {
                    Some(draft) if draft.editing => "编辑快捷指令",
                    Some(draft) if draft.return_to_manager => "新建快捷指令",
                    _ => "保存为快捷指令",
                };
                (title, 490.)
            }
            Dialog::McpApproval => ("确认 MCP 操作", 570.),
            Dialog::AiHandoff(_) => ("交给 AI 接管原终端", 450.),
            Dialog::EndAiTerminal => ("结束 AI 终端", 450.),
            Dialog::About => ("关于 OrbitShell", 420.),
        };
        let content = match dialog {
            Dialog::Connections => self.connection_dialog(cx),
            Dialog::ConnectionGroup => self.group_editor(cx),
            Dialog::Profile(_) => self.profile_dialog(cx),
            Dialog::Login(profile, _) => self.login_dialog(profile, cx),
            Dialog::HostKey(profile) => self.host_dialog(profile, cx),
            Dialog::Search => unreachable!(),
            Dialog::Settings(tab) => self.settings(tab, cx),
            Dialog::Transfers => div()
                .p(space::LG)
                .child(
                    control("clear-finished-transfers", "清除已结束记录", false)
                        .on_click(cx.listener(|view, _, _, _| view.transfers.clear_finished())),
                )
                .child(self.transfer_rows(cx)),
            Dialog::UploadConflict => self.upload_conflict_dialog(cx),
            Dialog::DeleteRemoteFile => self.remote_delete_dialog(cx),
            Dialog::EditorConfirm => self.editor_confirmation(cx),
            Dialog::Exit => self.desktop_exit_dialog(cx),
            Dialog::QuickCommands => self.quick_command_manager(cx),
            Dialog::QuickCommand => self.quick_command_dialog(cx),
            Dialog::McpApproval => self.mcp_approval_dialog(cx),
            Dialog::AiHandoff(profile) => self.handoff_dialog(profile, cx),
            Dialog::EndAiTerminal => self.end_ai_terminal_dialog(cx),
            Dialog::About => div().p(space::LG).child("OrbitShell · GPUI 工作台").child(
                div()
                    .mt(space::SM)
                    .text_color(rgb(color::MUTED))
                    .child("原生桌面 SSH / SFTP 工作台。"),
            ),
        };
        let footer = match dialog {
            Dialog::Profile(index) => Some(self.profile_footer(index, cx)),
            Dialog::Login(profile, origin) => Some(self.login_footer(profile, origin, cx)),
            _ => None,
        };
        let max_height = if keyboard_dialog || fixed_height {
            (window.viewport_size().height - space::TITLEBAR_HEIGHT - space::MD * 2.)
                .min(px(430.))
                .max(px(0.))
        } else {
            px(430.)
        };
        let close = if keyboard_dialog {
            dialog_tab(
                icon_control_with_size(
                    "close-dialog",
                    Icon::Close,
                    "关闭",
                    false,
                    space::COMPACT_ICON_SIZE,
                )
                .size(space::CONTROL_HEIGHT),
                0,
            )
        } else {
            icon_control("close-dialog", Icon::Close, "关闭", false)
        }
        .on_click(cx.listener(|view, _, window, cx| view.close_dialog(window, cx)));
        div()
            .id("workbench-dialog-layer")
            .absolute()
            .inset_0()
            .top(space::TITLEBAR_HEIGHT)
            .flex()
            .items_center()
            .justify_center()
            .occlude()
            .bg(theme::modal_scrim())
            .track_focus(&self.dialog_focus)
            .child(
                div()
                    .id("workbench-dialog-shell")
                    .w(px(width))
                    .max_h(max_height)
                    .when(fixed_height, |panel| {
                        panel.h(SETTINGS_DIALOG_HEIGHT.min(max_height))
                    })
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .shadow_lg()
                    .rounded(space::WORKSPACE_RADIUS)
                    .bg(rgb(color::SURFACE))
                    .border_1()
                    .border_color(rgb(color::LINE))
                    .when(keyboard_dialog, |dialog| dialog.tab_group())
                    .child(
                        div()
                            .h(space::HEADER_HEIGHT)
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_between()
                            .px(space::LG)
                            .border_b_1()
                            .border_color(rgb(color::LINE))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(space::SM)
                                    .when(connection_dialog, |heading| {
                                        let symbol = if dialog == Dialog::Connections {
                                            Icon::Connections
                                        } else {
                                            Icon::Terminal
                                        };
                                        heading.child(
                                            icon(symbol, color::ACCENT)
                                                .size(space::COMPACT_ICON_SIZE),
                                        )
                                    })
                                    .child(title),
                            )
                            .child(close),
                    )
                    .when(dialog == Dialog::Connections, |panel| {
                        panel.child(self.connection_toolbar(cx))
                    })
                    .child(
                        div()
                            .id("dialog-content")
                            .min_h(px(0.))
                            .when(fixed_height, |body| body.flex_1().flex())
                            .when(!fixed_height, |body| {
                                body.flex_shrink().overflow_y_scroll()
                            })
                            .child(content),
                    )
                    .when_some(footer, |panel, footer| panel.child(footer)),
            )
    }

    fn connection_toolbar(&self, cx: &Context<Self>) -> Div {
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap(space::SM)
            .px(space::LG)
            .py(space::SM)
            .border_b_1()
            .border_color(rgb(color::LINE))
            .bg(rgb(color::TOOLBAR))
            .child(icon(Icon::Search, color::MUTED).size(space::COMPACT_ICON_SIZE))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .child(self.management_filter.clone()),
            )
            .when(self.management_filter.read(cx).has_value(), |row| {
                row.child(
                    dialog_tab(
                        icon_control_with_size(
                            "clear-management-search",
                            Icon::Close,
                            "清空搜索",
                            false,
                            space::COMPACT_ICON_SIZE,
                        )
                        .size(space::CONTROL_HEIGHT),
                        2,
                    )
                    .on_click(cx.listener(|view, _, window, cx| {
                        view.management_filter
                            .update(cx, |field, cx| field.set_value("", cx));
                        view.management_filter.read(cx).focus(window);
                    })),
                )
            })
            .child(self.batch_group_controls(cx))
            .when(self.batch_group_selection.is_none(), |row| {
                row.child(
                    dialog_tab(control("new-ssh-profile", "新建连接", true), 4)
                        .h(space::ROW_HEIGHT)
                        .px(space::MD)
                        .font_weight(FontWeight::SEMIBOLD)
                        .on_click(cx.listener(|view, _, window, cx| {
                            view.open_dialog(Dialog::Profile(None), window, cx)
                        })),
                )
            })
    }

    fn connection_dialog(&self, cx: &Context<Self>) -> Div {
        let query = self
            .management_filter
            .read(cx)
            .value()
            .trim()
            .to_lowercase();
        let searching = !query.is_empty();
        let ssh: Vec<_> = self
            .profiles
            .iter()
            .enumerate()
            .filter(|(_, profile)| !profile.temporary && profile.remote.is_some())
            .filter(|(_, profile)| {
                !searching
                    || profile.title.to_lowercase().contains(&query)
                    || profile.address.to_lowercase().contains(&query)
                    || self
                        .groups
                        .get(profile.group)
                        .is_some_and(|group| group.to_lowercase().contains(&query))
            })
            .collect();
        let mut rows = div()
            .px(space::LG)
            .py(space::SM)
            .flex()
            .flex_col()
            .gap(space::XS)
            .when_some(self.backend_error.clone(), |list, error| {
                list.child(
                    div()
                        .py(space::XS)
                        .text_size(text::DETAIL)
                        .text_color(rgb(color::ACCENT_TEXT))
                        .child(error),
                )
            })
            .when_some(self.ssh_error.clone(), |list, error| {
                list.child(
                    div()
                        .py(space::XS)
                        .text_size(text::DETAIL)
                        .text_color(rgb(color::ACCENT_TEXT))
                        .child(error),
                )
            });
        if searching && ssh.is_empty() {
            return rows.child(
                div()
                    .py(space::LG)
                    .text_color(rgb(color::MUTED))
                    .child("没有匹配的连接"),
            );
        }
        rows = rows.child(connection_section("SSH 连接"));
        if ssh.is_empty() && !searching {
            rows = rows.child(
                div()
                    .px(space::SM)
                    .py(space::MD)
                    .text_color(rgb(color::MUTED))
                    .child("暂无 SSH 连接"),
            );
        }
        rows.children(
            ssh.into_iter()
                .enumerate()
                .map(|(position, (index, profile))| {
                    self.connection_row(index, profile, 5 + position as isize * 2, cx)
                }),
        )
    }

    fn connection_row(
        &self,
        index: usize,
        profile: &Profile,
        tab_index: isize,
        cx: &Context<Self>,
    ) -> Div {
        let live_session = self.sessions.iter().find(|session| {
            session.profile == index
                && session
                    .remote
                    .as_ref()
                    .is_none_or(|remote| !remote.is_closed())
        });
        let open = live_session.is_some();
        let in_batch = self.batch_group_selection.is_some();
        let checked = self
            .batch_group_selection
            .as_ref()
            .is_some_and(|selection| selection.contains(&index));
        let remote = profile.remote.as_ref();
        let prompt_to_connect = remote.is_some() && !open;
        let selected = if in_batch {
            checked
        } else {
            self.active == Some(index)
        };
        let details = self
            .groups
            .get(profile.group)
            .filter(|group| remote.is_some() && group.as_str() != "未分组")
            .map_or_else(
                || profile.address.clone(),
                |group| format!("{} · {group}", profile.address),
            );
        div()
            .h(px(44.))
            .flex_none()
            .flex()
            .items_center()
            .gap(space::SM)
            .px(space::SM)
            .rounded(space::CONTROL_RADIUS)
            .border_b_1()
            .border_color(rgb(color::LINE))
            .when(selected, |row| row.bg(rgb(color::SELECTED)))
            .child(
                icon(
                    if remote.is_some() {
                        Icon::Connections
                    } else {
                        Icon::Terminal
                    },
                    color::MUTED,
                )
                .size(space::COMPACT_ICON_SIZE),
            )
            .child(
                div()
                    .min_w(px(0.))
                    .flex_1()
                    .child(
                        div()
                            .truncate()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(profile.title.clone()),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(text::DETAIL)
                            .text_color(rgb(color::MUTED))
                            .child(details),
                    ),
            )
            .when(in_batch, |row| {
                row.child(
                    dialog_tab(
                        control(
                            ElementId::named_usize("select-host-for-group", index),
                            if checked { "已选" } else { "选择" },
                            checked,
                        )
                        .w(px(52.))
                        .border_1()
                        .border_color(rgb(color::LINE)),
                        tab_index,
                    )
                    .on_click(cx.listener(move |view, _, _, cx| {
                        if let Some(selection) = &mut view.batch_group_selection {
                            if !selection.remove(&index) {
                                selection.insert(index);
                            }
                            cx.notify();
                        }
                    })),
                )
            })
            .when(!in_batch && remote.is_some() && self.can_edit_profile(index), |row| {
                row.child(
                    dialog_tab(
                        icon_control_with_size(
                            ElementId::named_usize("edit-profile", index),
                            Icon::Pencil,
                            "编辑连接",
                            false,
                            space::COMPACT_ICON_SIZE,
                        )
                        .size(space::CONTROL_HEIGHT),
                        tab_index,
                    )
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.open_dialog(Dialog::Profile(Some(index)), window, cx)
                    })),
                )
            })
            .when(!in_batch, |row| {
                row.child(
                    dialog_tab(
                        control(
                            ElementId::named_usize("open-profile", index),
                            if live_session.is_some_and(|session| session.show_tab) {
                                "切换"
                            } else if open {
                                "打开"
                            } else if remote.is_some() {
                                "连接"
                            } else {
                                "打开"
                            },
                            false,
                        )
                        .w(px(52.))
                        .border_1()
                        .border_color(rgb(color::LINE)),
                        tab_index + 1,
                    )
                    .on_click(cx.listener(move |view, _, window, cx| {
                        if prompt_to_connect {
                            view.connect_or_prompt(index, window, cx);
                        } else {
                            view.open_session(index, cx, window);
                        }
                    })),
                )
            })
    }

    fn profile_dialog(&self, cx: &Context<Self>) -> Div {
        let field_row = || div().flex().gap(space::MD);
        div()
            .p(space::LG)
            .flex()
            .flex_col()
            .gap(space::MD)
            .child(
                field_row()
                    .child(
                        profile_field("连接名称", &self.profile_title)
                            .min_w(px(0.))
                            .flex_1(),
                    )
                    .child(
                        profile_field("分组（可选）", &self.profile_group)
                            .w(px(148.))
                            .flex_none(),
                    ),
            )
            .child(
                field_row()
                    .child(
                        profile_field("服务器地址", &self.profile_address)
                            .min_w(px(0.))
                            .flex_1(),
                    )
                    .child(
                        profile_field("端口", &self.profile_port)
                            .w(px(80.))
                            .flex_none(),
                    ),
            )
            .child(
                field_row()
                    .child(
                        profile_field("用户名", &self.profile_username)
                            .min_w(px(0.))
                            .flex_1(),
                    )
                    .child(self.profile_auth(cx)),
            )
            .when(self.profile_uses_key, |form| {
                form.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(space::XS)
                        .child(profile_label("私钥文件"))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(space::SM)
                                .child(
                                    div()
                                        .min_w(px(0.))
                                        .flex_1()
                                        .child(self.profile_key_path.clone()),
                                )
                                .child(
                                    dialog_tab(
                                        control("browse-private-key", "选择文件…", false),
                                        9,
                                    )
                                    .h(space::ROW_HEIGHT)
                                    .border_1()
                                    .border_color(rgb(color::LINE))
                                    .on_click(cx.listener(
                                        |view, _, window, cx| view.pick_profile_key(window, cx),
                                    )),
                                ),
                        ),
                )
            })
    }

    fn profile_auth(&self, cx: &Context<Self>) -> Div {
        div()
            .w(px(148.))
            .flex_none()
            .flex()
            .flex_col()
            .gap(space::XS)
            .child(profile_label("认证方式"))
            .child(
                div()
                    .h(space::ROW_HEIGHT)
                    .flex()
                    .items_center()
                    .gap(space::XS)
                    .px(space::XS)
                    .rounded(space::CONTROL_RADIUS)
                    .border_1()
                    .border_color(rgb(color::LINE))
                    .bg(rgb(color::TOOLBAR))
                    .child(
                        dialog_tab(
                            control("ssh-auth-password", "密码", !self.profile_uses_key),
                            6,
                        )
                        .flex_1()
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.profile_uses_key = false;
                            view.ssh_error = None;
                            cx.notify();
                        })),
                    )
                    .child(
                        dialog_tab(control("ssh-auth-key", "私钥", self.profile_uses_key), 7)
                            .flex_1()
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.profile_uses_key = true;
                                view.ssh_error = None;
                                cx.notify();
                            })),
                    ),
            )
    }

    fn profile_footer(&self, index: Option<usize>, cx: &Context<Self>) -> Div {
        div()
            .flex_none()
            .flex()
            .flex_col()
            .gap(space::SM)
            .px(space::LG)
            .py(space::SM)
            .border_t_1()
            .border_color(rgb(color::LINE))
            .bg(rgb(color::TOOLBAR))
            .when_some(self.ssh_error.clone(), |footer, error| {
                footer.child(
                    div()
                        .text_size(text::DETAIL)
                        .text_color(rgb(color::ACCENT_TEXT))
                        .child(error),
                )
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(space::SM)
                    .child(
                        dialog_tab(control("cancel-profile", "取消", false), 10)
                            .h(space::ROW_HEIGHT)
                            .px(space::MD)
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.open_dialog(Dialog::Connections, window, cx)
                            })),
                    )
                    .child(
                        dialog_tab(control("save-profile", "保存连接", true), 11)
                            .h(space::ROW_HEIGHT)
                            .px(space::MD)
                            .font_weight(FontWeight::SEMIBOLD)
                            .on_click(cx.listener(move |view, _, window, cx| {
                                view.save_ssh_profile(index, window, cx)
                            })),
                    ),
            )
    }

    fn pick_profile_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.profile_key_picker.is_some() {
            return;
        }
        let selection = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("选择 SSH 私钥文件".into()),
        });
        self.profile_key_picker = Some(cx.spawn_in(window, async move |view, cx| {
            let result = selection.await;
            let _ = view.update_in(cx, |view, window, cx| {
                view.profile_key_picker = None;
                if !matches!(view.dialog, Some(Dialog::Profile(_))) || !view.profile_uses_key {
                    return;
                }
                match result {
                    Ok(Ok(Some(paths))) => {
                        if let Some(path) = paths.first() {
                            view.profile_key_path.update(cx, |field, cx| {
                                field.set_value(&path.to_string_lossy(), cx);
                            });
                            view.profile_key_path.read(cx).focus(window);
                            view.ssh_error = None;
                        }
                    }
                    Ok(Ok(None)) => {}
                    _ => view.ssh_error = Some("无法打开文件选择器，请手动填写私钥路径。".into()),
                }
                cx.notify();
            });
        }));
    }

    pub(super) fn search_layer(&self, cx: &Context<Self>) -> Stateful<Div> {
        div()
            .id("terminal-search-layer")
            .absolute()
            .top(space::TOOLBAR_HEIGHT + space::HEADER_HEIGHT + space::XS)
            .right(space::MD)
            .w(px(345.))
            .p(space::MD)
            .flex()
            .flex_col()
            .gap(space::SM)
            .occlude()
            .bg(rgb(color::SURFACE))
            .rounded(space::CONTROL_RADIUS)
            .border_1()
            .border_color(rgb(color::LINE))
            .shadow_md()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_size(text::DETAIL)
                    .text_color(rgb(color::MUTED))
                    .child("查找终端内容")
                    .child(
                        icon_control("close-search", Icon::Close, "关闭查找", false)
                            .size(space::CONTROL_HEIGHT)
                            .on_click(
                                cx.listener(|view, _, window, cx| view.close_dialog(window, cx)),
                            ),
                    ),
            )
            .child(self.search_field.clone())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(space::SM)
                    .child(
                        div()
                            .flex_1()
                            .text_color(rgb(color::MUTED))
                            .text_size(text::DETAIL)
                            .child(match self.search_result {
                                Some(true) => "已跳转到匹配位置",
                                Some(false) => "没有找到匹配",
                                None => "Enter 查找 · Shift+Enter 反向查找",
                            }),
                    )
                    .child(
                        control("find-previous", "上一个", false)
                            .on_click(cx.listener(|view, _, _, cx| view.find_terminal(true, cx))),
                    )
                    .child(
                        control("find-next", "下一个", true)
                            .on_click(cx.listener(|view, _, _, cx| view.find_terminal(false, cx))),
                    ),
            )
    }

    fn set_interface_theme(&mut self, mode: UiTheme, cx: &mut Context<Self>) {
        let saved = self.backend.as_ref().is_some_and(|backend| {
            backend
                .save_layout(SavedLayout {
                    ui_theme: Some(mode),
                    ..SavedLayout::default()
                })
                .is_ok()
        });
        self.appearance_error = (!saved).then_some("主题已应用，保存到本机失败。");
        theme::set_ui_theme(mode);
        cx.refresh_windows();
        cx.notify();
    }

    fn appearance_settings(&self, cx: &Context<Self>) -> Div {
        let appearance = *self.terminal_appearance.read(cx);
        let interface_theme = setting_choices().children(
            [
                ("interface-light", "浅色", UiTheme::Light),
                ("interface-dark", "深色", UiTheme::Dark),
            ]
            .map(|(id, label, mode)| {
                control(id, label, theme::ui_theme() == mode).on_click(cx.listener(
                    move |view, _, _, cx| view.set_interface_theme(mode, cx),
                ))
            }),
        );
        let font_controls = setting_choices()
            .child(
                control("settings-font-minus", "A−", false)
                    .tooltip(|_, cx| cx.new(|_| Tooltip("缩小终端字号 · Ctrl+-")).into())
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.terminal_appearance.update(cx, |appearance, cx| {
                            appearance.adjust_font_size(-1, cx);
                        });
                    })),
            )
            .child(
                control(
                    "settings-font-reset",
                    format!("{} px", appearance.font_size),
                    false,
                )
                .min_w(px(48.))
                .tooltip(|_, cx| cx.new(|_| Tooltip("恢复默认终端字号 · Ctrl+0")).into())
                .on_click(cx.listener(|view, _, _, cx| {
                    view.terminal_appearance.update(cx, |appearance, cx| {
                        appearance.set_font_size(DEFAULT_FONT_SIZE, cx);
                    });
                })),
            )
            .child(
                control("settings-font-plus", "A+", false)
                    .tooltip(|_, cx| cx.new(|_| Tooltip("放大终端字号 · Ctrl+=")).into())
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.terminal_appearance.update(cx, |appearance, cx| {
                            appearance.adjust_font_size(1, cx);
                        });
                    })),
            );
        let palette = setting_choices().children(
            [
                ("terminal-follow-interface", "跟随界面", None),
                ("terminal-light", "浅色", Some(UiTheme::Light)),
                ("terminal-dark", "深色", Some(UiTheme::Dark)),
            ]
            .map(|(id, label, palette)| {
                control(id, label, appearance.palette == palette).on_click(cx.listener(
                    move |view, _, _, cx| {
                        view.terminal_appearance.update(cx, |appearance, cx| {
                            if appearance.palette != palette {
                                appearance.palette = palette;
                                cx.notify();
                            }
                        });
                    },
                ))
            }),
        );
        let density = setting_choices().children(
            [
                ("settings-density-compact", "紧凑", true),
                ("settings-density-comfortable", "舒适", false),
            ]
            .map(|(id, label, compact)| {
                control(id, label, self.compact_files == compact).on_click(cx.listener(
                    move |view, _, _, cx| {
                        view.compact_files = compact;
                        cx.notify();
                    },
                ))
            }),
        );
        let hidden = setting_choices().children(
            [
                ("settings-hidden-off", "隐藏", false),
                ("settings-hidden-on", "显示", true),
            ]
            .map(|(id, label, show)| {
                control(id, label, self.show_hidden_files == show).on_click(cx.listener(
                    move |view, _, _, cx| {
                        if view.show_hidden_files != show {
                            view.toggle_hidden_files(cx);
                        }
                    },
                ))
            }),
        );
        div()
            .flex()
            .flex_col()
            .gap(space::LG)
            .child(
                setting_group("界面")
                    .child(
                        setting_row("主题")
                            .h(space::TOOLBAR_HEIGHT)
                            .child(interface_theme),
                    )
                    .when_some(self.appearance_error, |group, error| {
                        group.child(
                            div()
                                .text_size(text::DETAIL)
                                .text_color(rgb(color::DANGER))
                                .child(error),
                        )
                    }),
            )
            .child(
                setting_group("终端").child(
                    div()
                        .flex()
                        .flex_col()
                        .child(
                            setting_row("字号")
                                .h(space::TOOLBAR_HEIGHT)
                                .child(font_controls),
                        )
                        .child(setting_row("配色").h(space::TOOLBAR_HEIGHT).child(palette)),
                ),
            )
            .child(
                setting_group("远程文件").child(
                    div()
                        .flex()
                        .flex_col()
                        .child(
                            setting_row("列表密度")
                                .h(space::TOOLBAR_HEIGHT)
                                .child(density),
                        )
                        .child(
                            setting_row("隐藏文件")
                                .h(space::TOOLBAR_HEIGHT)
                                .child(hidden),
                        ),
                ),
            )
    }

    fn settings(&self, tab: SettingsTab, cx: &Context<Self>) -> Div {
        let body = div()
            .p(space::LG)
            .flex_none()
            .min_w(px(0.))
            .flex()
            .flex_col()
            .gap(space::SM);
        let body = match tab {
            SettingsTab::Appearance => body
                .child(self.appearance_settings(cx))
                .child(self.desktop_settings(cx)),
            SettingsTab::Shortcuts => body.children(
                [
                    ("新建连接", "Ctrl+Shift+T"),
                    ("关闭标签", "Ctrl+Shift+W"),
                    ("切换会话", "Ctrl+Tab"),
                    ("查找终端", "Ctrl+F"),
                    ("显示连接列表", "Ctrl+B"),
                    ("展开底部工具", "Ctrl+J"),
                    ("调整字号", "Ctrl+= / Ctrl+-"),
                    ("粘贴到终端", "Ctrl+V"),
                ]
                .map(|(name, keys)| {
                    setting_row(name).child(div().text_color(rgb(color::MUTED)).child(keys))
                }),
            ),
            SettingsTab::Mcp => body.child(self.mcp_settings(cx)),
            SettingsTab::Bastion => body.child(self.bastion_settings(cx)),
        };
        div()
            .flex()
            .flex_1()
            .min_w(px(0.))
            .min_h(px(0.))
            .items_start()
            .child(
                div()
                    .w(px(122.))
                    .h_full()
                    .flex_none()
                    .p(space::SM)
                    .bg(rgb(color::SIDEBAR))
                    .border_r_1()
                    .border_color(rgb(color::LINE))
                    .flex()
                    .flex_col()
                    .gap(space::XS)
                    .children(
                        [
                            (SettingsTab::Appearance, "settings-appearance", "外观"),
                            (SettingsTab::Shortcuts, "settings-shortcuts", "快捷键"),
                            (SettingsTab::Mcp, "settings-mcp", "AI 与 MCP"),
                            (SettingsTab::Bastion, "settings-bastion", "堡垒机联动"),
                        ]
                        .map(|(item, id, name)| {
                            control(id, name, tab == item)
                                .justify_start()
                                .px(space::MD)
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    if item != SettingsTab::Bastion {
                                        view.clear_bastion_link(cx);
                                    }
                                    view.dialog = Some(Dialog::Settings(item));
                                    cx.notify();
                                }))
                        }),
                    ),
            )
            .child(
                div()
                    .id(match tab {
                        SettingsTab::Appearance => "settings-appearance-content",
                        SettingsTab::Shortcuts => "settings-shortcuts-content",
                        SettingsTab::Mcp => "settings-mcp-content",
                        SettingsTab::Bastion => "settings-bastion-content",
                    })
                    .h_full()
                    .flex_1()
                    .min_w(px(0.))
                    .min_h(px(0.))
                    .flex()
                    .flex_col()
                    .overflow_y_scroll()
                    .child(body),
            )
    }

    pub(super) fn toggle_hidden_files(&mut self, cx: &mut Context<Self>) {
        self.show_hidden_files = !self.show_hidden_files;
        if !self.show_hidden_files {
            for session in &mut self.sessions {
                if let Some(remote) = &mut session.remote {
                    remote.hide_dot_selection();
                }
                if session
                    .selected_file
                    .and_then(|index| session.directory.entries().get(index))
                    .is_some_and(|entry| entry.name.starts_with('.'))
                {
                    session.selected_file = None;
                }
            }
        }
        cx.notify();
    }

    pub(super) fn history_page(&self, cx: &Context<Self>) -> Div {
        let filter = self.history_filter.read(cx).value().trim().to_lowercase();
        let rows: Vec<_> = self
            .history
            .iter()
            .rev()
            .filter(|entry| {
                filter.is_empty()
                    || entry.command.to_lowercase().contains(&filter)
                    || self.profiles[entry.profile]
                        .title
                        .to_lowercase()
                        .contains(&filter)
            })
            .collect();
        let ai_rows: Vec<_> = self
            .mcp_workspace
            .executions()
            .into_iter()
            .filter(|entry| {
                filter.is_empty()
                    || [&entry.command, &entry.session, &entry.agent]
                        .iter()
                        .any(|value| value.to_lowercase().contains(&filter))
            })
            .collect();
        div()
            .flex_1()
            .min_h(px(0.))
            .flex()
            .flex_col()
            .px(space::LG)
            .py(space::MD)
            .gap(space::SM)
            .child(
                div()
                    .text_color(rgb(color::MUTED))
                    .text_size(text::DETAIL)
                    .child("本地输入与 Agent 执行记录"),
            )
            .when_some(self.mcp_workspace.history_error(), |pane, message| {
                pane.child(
                    div()
                        .text_size(text::DETAIL)
                        .text_color(rgb(color::ACCENT_TEXT))
                        .child(message),
                )
            })
            .child(self.history_filter.clone())
            .child(
                div()
                    .id("history-list")
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .when(rows.is_empty() && ai_rows.is_empty(), |list| {
                        list.child(
                            div()
                                .p(space::SM)
                                .text_color(rgb(color::MUTED))
                                .child("暂无匹配记录"),
                        )
                    })
                    .children(ai_rows.into_iter().enumerate().map(|(index, entry)| {
                        let command = entry.command;
                        div()
                            .flex()
                            .items_center()
                            .gap(space::SM)
                            .py(space::XS)
                            .border_b_1()
                            .border_color(rgb(color::LINE))
                            .child(
                                div()
                                    .w(px(150.))
                                    .truncate()
                                    .text_color(rgb(color::MUTED))
                                    .child(format!("{} · {}", entry.agent, entry.session)),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .truncate()
                                    .font_family(text::MONO_FONT)
                                    .child(command.clone()),
                            )
                            .child(
                                control(
                                    ElementId::named_usize("copy-ai-command", index),
                                    "复制",
                                    false,
                                )
                                .on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                        command.clone(),
                                    ))
                                }),
                            )
                    }))
                    .children(rows.into_iter().enumerate().map(|(index, entry)| {
                        let command = entry.command.clone();
                        div()
                            .flex()
                            .items_center()
                            .gap(space::SM)
                            .py(space::XS)
                            .border_b_1()
                            .border_color(rgb(color::LINE))
                            .child(
                                div()
                                    .w(px(94.))
                                    .truncate()
                                    .text_color(rgb(color::MUTED))
                                    .child(self.profiles[entry.profile].title.clone()),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .truncate()
                                    .font_family(text::MONO_FONT)
                                    .child(entry.command.clone()),
                            )
                            .child(
                                control(
                                    ElementId::named_usize("copy-command", index),
                                    "复制",
                                    false,
                                )
                                .on_click(cx.listener(
                                    move |_, _, _, cx| {
                                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                            command.clone(),
                                        ))
                                    },
                                )),
                            )
                    })),
            )
    }

    pub(super) fn preview_page(&self, cx: &Context<Self>) -> Div {
        let page = div().flex_1().min_h(px(0.)).flex().flex_col();
        let Some(preview) = &self.preview else {
            return page;
        };
        let contents = preview.contents;
        page.child(
            div()
                .h(space::TOOLBAR_HEIGHT)
                .flex_none()
                .flex()
                .items_center()
                .gap(space::SM)
                .px(space::LG)
                .child(file_icon(preview.name, false))
                .child(div().flex_1().min_w(px(0.)).truncate().child(format!(
                    "{} · {}",
                    self.profiles[preview.profile].title, preview.path
                )))
                .child(
                    div()
                        .text_size(text::DETAIL)
                        .text_color(rgb(color::MUTED))
                        .child("本地只读预览"),
                )
                .child(control("copy-preview", "复制", false).on_click(cx.listener(
                    move |_, _, _, cx| {
                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(contents.into()))
                    },
                ))),
        )
        .child(
            div()
                .id("sample-file-preview")
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scroll()
                .px(space::LG)
                .py(space::SM)
                .font_family(text::MONO_FONT)
                .children(contents.lines().enumerate().map(|(index, line)| {
                    div()
                        .flex()
                        .items_center()
                        .h(px(24.))
                        .child(
                            div()
                                .w(px(34.))
                                .flex_none()
                                .text_color(rgb(color::MUTED))
                                .text_size(text::DETAIL)
                                .child((index + 1).to_string()),
                        )
                        .child(div().min_w(px(0.)).truncate().child(line))
                })),
        )
    }

    pub(super) fn start_page(&self, cx: &Context<Self>) -> Stateful<Div> {
        let has_recent = !self.recent.is_empty();
        let profiles = if has_recent {
            self.recent.clone()
        } else if !self.show_connections {
            self.profiles
                .iter()
                .enumerate()
                .filter(|(_, profile)| profile.remote.is_some() && !profile.temporary)
                .map(|(index, _)| index)
                .take(3)
                .collect()
        } else {
            Vec::new()
        };
        let content = div()
            .flex_none()
            .my_auto()
            .w_full()
            .max_w(px(420.))
            .flex()
            .flex_col()
            .gap(space::LG)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(space::SM)
                    .child(start_brand())
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(32.))
                            .child("OrbitShell"),
                    )
                    .child(
                        div().flex().gap(space::SM).mt(space::SM).children(
                            [
                                (
                                    "start-connect",
                                    Icon::Connections,
                                    "打开会话",
                                    Dialog::Connections,
                                    true,
                                ),
                                (
                                    "start-new-connection",
                                    Icon::Plus,
                                    "新建连接",
                                    Dialog::Profile(None),
                                    false,
                                ),
                            ]
                            .map(
                                |(id, glyph, label, dialog, primary)| {
                                    let tint = if primary { color::SURFACE } else { color::TEXT };
                                    div()
                                        .id(id)
                                        .h(space::ROW_HEIGHT)
                                        .flex_none()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .gap(space::SM)
                                        .px(space::MD)
                                        .rounded(space::CONTROL_RADIUS)
                                        .border_1()
                                        .border_color(rgb(if primary {
                                            color::ACCENT
                                        } else {
                                            color::LINE
                                        }))
                                        .bg(rgb(if primary {
                                            color::ACCENT
                                        } else {
                                            color::SURFACE
                                        }))
                                        .text_color(rgb(tint))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .cursor_pointer()
                                        .hover(move |style| {
                                            style.bg(rgb(if primary {
                                                color::ACCENT_TEXT
                                            } else {
                                                color::HOVER
                                            }))
                                        })
                                        .child(icon(glyph, tint).size(space::COMPACT_ICON_SIZE))
                                        .child(label)
                                        .on_click(cx.listener(move |view, _, window, cx| {
                                            view.open_dialog(dialog, window, cx)
                                        }))
                                },
                            ),
                        ),
                    ),
            )
            .when(!profiles.is_empty(), |page| {
                page.child(
                    div()
                        .mt(space::MD)
                        .flex()
                        .flex_col()
                        .gap(space::SM)
                        .child(
                            div()
                                .text_color(rgb(color::MUTED))
                                .text_size(text::DETAIL)
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(if has_recent {
                                    "最近打开"
                                } else {
                                    "已保存的连接"
                                }),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .border_t_1()
                                .border_color(rgb(color::LINE))
                                .children(profiles.into_iter().enumerate().map(
                                    |(position, id)| {
                                        let profile = &self.profiles[id];
                                        div()
                                            .id(ElementId::named_usize("recent-session", id))
                                            .h(px(44.))
                                            .px(space::MD)
                                            .flex()
                                            .flex_none()
                                            .items_center()
                                            .gap(space::SM)
                                            .cursor_pointer()
                                            .when(position > 0, |row| {
                                                row.border_t_1().border_color(rgb(color::LINE))
                                            })
                                            .hover(|row| row.bg(rgb(color::HOVER)))
                                            .child(
                                                icon(
                                                    if profile.remote.is_some() {
                                                        Icon::Connections
                                                    } else {
                                                        Icon::Terminal
                                                    },
                                                    color::MUTED,
                                                )
                                                .size(space::COMPACT_ICON_SIZE),
                                            )
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w(px(0.))
                                                    .child(
                                                        div()
                                                            .truncate()
                                                            .child(profile.title.clone()),
                                                    )
                                                    .child(
                                                        div()
                                                            .truncate()
                                                            .text_color(rgb(color::MUTED))
                                                            .text_size(text::DETAIL)
                                                            .child(profile.address.clone()),
                                                    ),
                                            )
                                            .child(
                                                icon(Icon::ChevronRight, color::MUTED)
                                                    .size(space::COMPACT_ICON_SIZE),
                                            )
                                            .on_click(cx.listener(move |view, _, window, cx| {
                                                view.open_session(id, cx, window)
                                            }))
                                    },
                                )),
                        ),
                )
            });
        div()
            .id("workspace-start")
            .flex_1()
            .min_h(px(0.))
            .flex()
            .flex_col()
            .items_center()
            .p(space::LG)
            .pb(space::LG + space::XL * 4.)
            .overflow_y_scroll()
            .child(content)
    }

    pub(super) fn open_quick_command_draft(
        &mut self,
        selection: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.backend.is_none() {
            self.notice = Some("工作区配置不可用，无法保存快捷指令。");
            cx.notify();
            return;
        }
        if selection.len() > 64 * 1024 {
            self.notice = Some("请缩小选区，快捷指令最多支持 64 KiB。");
            cx.notify();
            return;
        }
        self.open_command_form(
            QuickCommand {
                id: uuid::Uuid::new_v4().to_string(),
                name: String::new(),
                group: String::new(),
                command: selection.trim().to_owned(),
                append_newline: false,
            },
            false,
            window,
            cx,
        );
    }

    pub(super) fn open_command_form(
        &mut self,
        initial: QuickCommand,
        editing: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let return_to_manager = self.dialog == Some(Dialog::QuickCommands);
        self.open_dialog(Dialog::QuickCommand, window, cx);
        let draft = QuickCommandDraft {
            id: initial.id,
            editing,
            append_newline: initial.append_newline,
            return_to_manager,
            name: cx.new(|cx| TextField::new(cx, "输入指令名称").with_limit(160)),
            group: cx.new(|cx| TextField::new(cx, "未分组").with_limit(120)),
            command: cx.new(|cx| EditorInput::new(initial.command, false, cx)),
            error: None,
        };
        draft.name.update(cx, |field, cx| field.set_value(&initial.name, cx));
        draft.group.update(cx, |field, cx| field.set_value(&initial.group, cx));
        draft.name.read(cx).focus(window);
        self.quick_command_draft = Some(draft);
        cx.notify();
    }

    fn save_quick_command(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = &mut self.quick_command_draft else {
            return;
        };
        let Some(backend) = &self.backend else {
            draft.error = Some("工作区配置不可用，无法保存快捷指令。".into());
            cx.notify();
            return;
        };
        let command = QuickCommand {
            id: draft.id.clone(),
            name: draft.name.read(cx).value().to_owned(),
            group: draft.group.read(cx).value().to_owned(),
            command: draft.command.read(cx).text().to_owned(),
            append_newline: draft.append_newline,
        };
        match backend.save_quick_command(command, draft.editing) {
            Ok(commands) => {
                self.quick_commands = commands;
                self.close_dialog(window, cx);
                if self.dialog.is_none() {
                    self.notice = Some("已保存到快捷指令。");
                }
            }
            Err(error) => draft.error = Some(error),
        }
        cx.notify();
    }

    fn quick_command_dialog(&self, cx: &Context<Self>) -> Div {
        let Some(draft) = &self.quick_command_draft else {
            return div();
        };
        div()
            .flex()
            .flex_col()
            .gap(space::SM)
            .p(space::LG)
            .child(
                div()
                    .flex()
                    .gap(space::SM)
                    .child(div().flex_1().min_w(px(0.)).child(draft.name.clone()))
                    .child(div().w(px(138.)).child(draft.group.clone())),
            )
            .child(
                div()
                    .text_size(text::DETAIL)
                    .child("指令内容")
                    .text_color(rgb(color::MUTED)),
            )
            .child(
                div()
                    .h(px(130.))
                    .flex()
                    .flex_col()
                    .border_1()
                    .border_color(rgb(color::LINE))
                    .rounded(space::CONTROL_RADIUS)
                    .overflow_hidden()
                    .child(draft.command.clone()),
            )
            .child(
                div()
                    .text_size(text::DETAIL)
                    .text_color(rgb(color::MUTED))
                    .child("请勿在快捷指令中保存密码或访问令牌。"),
            )
            .when_some(draft.error.clone(), |dialog, error| {
                dialog.child(div().text_color(rgb(color::DANGER)).child(error))
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(space::SM)
                    .child(
                        control("cancel-quick-command", "取消", false).on_click(
                            cx.listener(|view, _, window, cx| view.close_dialog(window, cx)),
                        ),
                    )
                    .child(primary_control("save-quick-command", "保存指令").on_click(
                        cx.listener(|view, _, window, cx| view.save_quick_command(window, cx)),
                    )),
            )
    }
}

fn start_brand() -> Div {
    div()
        .relative()
        .w(px(176.))
        .h(px(100.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .child(
            canvas(
                |_, _, _| (),
                |bounds, (), window, _| {
                    for (radius_x, radius_y, rotation, tint) in [
                        (px(68.), px(32.), 22., color::LINE),
                        (px(76.), px(27.), 340., color::SELECTED_HOVER),
                    ] {
                        let left = point(-radius_x, px(0.));
                        let right = point(radius_x, px(0.));
                        let radii = point(radius_x, radius_y);
                        let mut orbit = PathBuilder::stroke(px(1.));
                        orbit.move_to(left);
                        orbit.arc_to(radii, px(0.), false, true, right);
                        orbit.arc_to(radii, px(0.), false, true, left);
                        orbit.close();
                        orbit.rotate(rotation);
                        orbit.translate(bounds.center());
                        if let Ok(path) = orbit.build() {
                            window.paint_path(path, rgb(tint));
                        }
                    }
                },
            )
            .absolute()
            .inset_0(),
        )
        .child(
            div()
                .absolute()
                .right(px(14.))
                .top(px(21.))
                .size(px(6.))
                .rounded_full()
                .bg(rgb(color::ACCENT)),
        )
        .child(
            div()
                .size(px(76.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(rgb(color::SELECTED))
                .child(
                    img(ImageSource::Resource(Resource::Embedded(
                        BRAND_IMAGE.into(),
                    )))
                    .size(px(60.))
                    .flex_none(),
                ),
        )
}

fn profile_label(label: &'static str) -> Div {
    div()
        .text_size(text::DETAIL)
        .text_color(rgb(color::MUTED))
        .child(label)
}

fn profile_field(label: &'static str, input: &Entity<TextField>) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(space::XS)
        .child(profile_label(label))
        .child(input.clone())
}

fn connection_section(label: &'static str) -> Div {
    div()
        .px(space::SM)
        .py(space::XS)
        .text_color(rgb(color::MUTED))
        .text_size(text::DETAIL)
        .font_weight(FontWeight::SEMIBOLD)
        .child(label)
}

pub(super) fn dialog_tab(button: Stateful<Div>, index: isize) -> Stateful<Div> {
    button
        .tab_index(index)
        .focus(|style| style.border_1().border_color(rgb(color::ACCENT)))
}

pub(super) fn setting_row(title: &'static str) -> Div {
    div()
        .h(space::ROW_HEIGHT)
        .flex()
        .items_center()
        .gap(space::SM)
        .border_b_1()
        .border_color(rgb(color::LINE))
        .child(div().flex_1().min_w(px(0.)).truncate().child(title))
}

pub(super) fn setting_group(title: &'static str) -> Div {
    div().flex().flex_col().gap(space::SM).child(
        div()
            .text_size(text::HEADING)
            .font_weight(FontWeight::SEMIBOLD)
            .child(title),
    )
}

pub(super) fn setting_choices() -> Div {
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(2.))
        .p(px(2.))
        .rounded(space::CONTROL_RADIUS)
        .border_1()
        .border_color(rgb(color::LINE))
        .bg(rgb(color::TOOLBAR))
}

pub(super) fn setting_action(id: &'static str, label: &'static str) -> Stateful<Div> {
    control(id, label, false)
        .h(space::ROW_HEIGHT)
        .px(space::MD)
        .border_1()
        .border_color(rgb(color::LINE))
}
