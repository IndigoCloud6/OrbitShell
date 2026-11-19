use orbitshell_ssh::launch::PendingLaunch;
use orbitshell_ssh::channel_diagnostics::{ChannelDiagnostics, FilePanelEvent};
use std::{
    collections::{HashSet, VecDeque},
    sync::Arc,
};
use tokio::sync::mpsc;

use gpui::{
    Context, CursorStyle, Div, DragMoveEvent, ElementId, Entity, FocusHandle, FontWeight,
    ImageSource, KeyDownEvent, MouseButton, MouseDownEvent, MouseUpEvent, Pixels, Resource, Stateful,
    Task, Window, div, img, prelude::*, px, relative,
};

use crate::{
    BRAND_IMAGE,
    demo::{self, Directory, Profile},
    file_icons::file_icon,
    icons::{Icon, Tooltip, icon},
    ssh_backend::{Backend, QuickCommands, SavedLayout},
    terminal_view::{
        FindSelection, SaveSelection, SubmittedLine, TerminalAppearance, TerminalView,
    },
    text_field::TextField,
    theme::{self, ThemeColor, color, control, rgb, space, text},
    transfer_queue::{TransferQueue, UploadConflict},
    window_frame::{DragArea, WindowFrame, toggle_maximize},
};

#[path = "workbench_actions.rs"]
mod actions;
use actions::{Dialog, LoginOrigin, Menu, Page, QuickCommandDraft, SettingsTab};
#[path = "workbench_quick_commands.rs"]
mod command_management;
#[path = "workbench_connection_groups.rs"]
mod connection_groups;
use connection_groups::{GroupDraft, GroupMenu, GroupMenuTarget};
#[path = "workbench_ssh.rs"]
mod ssh;
use ssh::{FileSort, LoginCredential, RemoteSession};
#[path = "workbench_ai.rs"]
mod ai;
#[path = "workbench_bastion.rs"]
mod bastion;
#[path = "workbench_desktop.rs"]
mod desktop;
#[path = "workbench_editor.rs"]
mod editor;
#[cfg(windows)]
pub(crate) use desktop::DesktopAction;
#[path = "workbench_file_menu.rs"]
mod file_menu;
#[path = "workbench_mcp.rs"]
mod mcp_settings;
#[path = "workbench_metrics.rs"]
mod metrics;
#[path = "workbench_transfers.rs"]
mod transfers;

const RAIL_WIDTH: Pixels = px(51.);
const CONNECTIONS_MIN: Pixels = px(184.);
const FILES_MIN: Pixels = px(226.);
const CENTER_MIN: Pixels = px(376.);
const CONNECTIONS_MAX: Pixels = px(340.);
const TOOLS_DEFAULT_HEIGHT: Pixels = px(190.);
const TOOLS_MIN_HEIGHT: Pixels = px(132.);
const CENTER_MIN_HEIGHT_WITH_TOOLS: Pixels = px(220.);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tool {
    Logs,
    Transfers,
    Commands,
    Ai,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Splitter {
    Connections,
    Files,
}

struct PanelResize {
    side: Splitter,
    diagnostics: Option<Arc<ChannelDiagnostics>>,
}

#[derive(Clone, Copy)]
struct ToolsSplitter;

struct ResizePreview;

impl Render for ResizePreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size(px(0.))
    }
}

struct Session {
    profile: usize,
    show_tab: bool,
    terminal: Entity<TerminalView>,
    directory: Directory,
    selected_file: Option<usize>,
    remote: Option<RemoteSession>,
}

struct FilePreview {
    profile: usize,
    name: &'static str,
    path: String,
    contents: &'static str,
}

struct HistoryEntry {
    profile: usize,
    command: String,
}

pub struct Workbench {
    window_frame: WindowFrame,
    desktop: desktop::DesktopState,
    backend: Option<Backend>,
    backend_error: Option<String>,
    ssh_error: Option<String>,
    sessions: Vec<Session>,
    profiles: Vec<Profile>,
    recent: Vec<usize>,
    terminal_appearance: Entity<TerminalAppearance>,
    appearance_error: Option<&'static str>,
    active: Option<usize>,
    groups: Vec<String>,
    group_open: Vec<bool>,
    show_connections: bool,
    show_files: bool,
    connections_width: Pixels,
    files_width: Pixels,
    resizing_panel: Option<PanelResize>,
    tool: Option<Tool>,
    tools_height: Pixels,
    transfers: Arc<TransferQueue>,
    upload_conflict_sender: mpsc::UnboundedSender<UploadConflict>,
    upload_conflicts: VecDeque<UploadConflict>,
    transfer_error: Option<String>,
    mcp_server: Option<crate::mcp::RunningMcp>,
    mcp_start: Option<Task<()>>,
    mcp_error: Option<String>,
    mcp_auto_start: bool,
    mcp_preference_error: Option<String>,
    mcp_desktop: mpsc::Sender<crate::mcp::DesktopRequest>,
    mcp_requests: VecDeque<crate::mcp::DesktopRequest>,
    mcp_approval: Option<crate::mcp::Approval>,
    mcp_workspace: Arc<crate::mcp_terminal::AiWorkspace>,
    ai_views: Vec<ai::AiView>,
    active_ai: Option<String>,
    pending_ai_end: Option<Arc<crate::mcp_terminal::Dedicated>>,
    dismissed_ai: Vec<String>,
    menu: Option<Menu>,
    dialog: Option<Dialog>,
    page: Page,
    preview: Option<FilePreview>,
    history_open: bool,
    history: VecDeque<HistoryEntry>,
    notice: Option<&'static str>,
    show_hidden_files: bool,
    file_sort: FileSort,
    compact_files: bool,
    connection_filter: Entity<TextField>,
    management_filter: Entity<TextField>,
    group_menu: Option<GroupMenu>,
    group_draft: Option<GroupDraft>,
    batch_group_selection: Option<HashSet<usize>>,
    profile_title: Entity<TextField>,
    profile_address: Entity<TextField>,
    profile_group: Entity<TextField>,
    profile_port: Entity<TextField>,
    profile_username: Entity<TextField>,
    profile_key_path: Entity<TextField>,
    profile_uses_key: bool,
    profile_key_picker: Option<Task<()>>,
    password: Entity<TextField>,
    remember_password: bool,
    login_credential: LoginCredential,
    login_pending: Option<Task<()>>,
    pending_bastion: Option<PendingLaunch>,
    #[cfg(windows)]
    external_launches: VecDeque<Result<PendingLaunch, String>>,
    bastion_registration: Option<String>,
    bastion_username: Entity<TextField>,
    bastion_link: Entity<TextField>,
    bastion_decode: bool,
    bastion_error: Option<String>,
    search_field: Entity<TextField>,
    search_result: Option<bool>,
    history_filter: Entity<TextField>,
    logs: VecDeque<String>,
    focus: FocusHandle,
    dialog_focus: FocusHandle,
    file_focus: FocusHandle,
    file_menu: Option<file_menu::FileMenu>,
    file_delete: Option<file_menu::RemoteDelete>,
    editors: editor::RemoteEditors,
    quick_commands: QuickCommands,
    command_manager: command_management::CommandManager,
    command_picker: command_management::CommandPicker,
    quick_command_draft: Option<QuickCommandDraft>,
}

impl Workbench {
    fn new_terminal(
        &self,
        address: &str,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Entity<TerminalView> {
        let terminal =
            cx.new(|cx| TerminalView::new(window, cx, address, self.terminal_appearance.clone()));
        cx.subscribe_in(
            &terminal,
            window,
            |view, terminal, FindSelection(text), window, cx| {
                if view.displayed_terminal() != Some(terminal) {
                    return;
                }
                view.open_dialog(Dialog::Search, window, cx);
                view.search_field
                    .update(cx, |field, cx| field.set_value(text, cx));
                view.find_terminal(false, cx);
            },
        )
        .detach();
        cx.subscribe_in(
            &terminal,
            window,
            |view, terminal, SaveSelection(text), window, cx| {
                if view.displayed_terminal() == Some(terminal) {
                    view.open_quick_command_draft(text, window, cx);
                }
            },
        )
        .detach();
        terminal
    }

    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::with_backend(Backend::open(), window, cx)
    }

    pub(crate) fn with_backend(
        backend: anyhow::Result<Backend>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (backend, mut ssh_error) = match backend {
            Ok(backend) => (Some(backend), None),
            Err(error) => (None, Some(format!("{error:#}"))),
        };
        let saved_profiles = match backend.as_ref().map(Backend::profiles).transpose() {
            Ok(profiles) => profiles.unwrap_or_default(),
            Err(error) => {
                ssh_error = Some(format!("{error:#}"));
                Vec::new()
            }
        };
        let (upload_conflict_sender, upload_conflicts) = mpsc::unbounded_channel();
        let (mcp_desktop, mcp_requests) = mpsc::channel(8);
        let (mcp_auto_start, mcp_preference_error) =
            match backend.as_ref().map(Backend::mcp_auto_start).transpose() {
                Ok(enabled) => (enabled.unwrap_or(false), None),
                Err(error) => (false, Some(format!("无法读取 MCP 自动开启设置。{error:#}"))),
            };
        let mcp_workspace = Arc::new(crate::mcp_terminal::AiWorkspace::open(
            backend.as_ref().map(Backend::history_path),
        ));
        let (saved_layout, layout_notice) =
            match backend.as_ref().map(Backend::layout).transpose() {
                Ok(layout) => (layout.unwrap_or_default(), None),
                Err(_) => (SavedLayout::default(), Some("无法读取已保存的外观与布局设置。")),
            };
        theme::set_ui_theme(saved_layout.ui_theme.unwrap_or_default());
        let mut workbench = Self {
            window_frame: WindowFrame::new(window),
            desktop: desktop::DesktopState::default(),
            quick_commands: backend
                .as_ref()
                .and_then(|backend| backend.quick_commands().ok())
                .unwrap_or_default(),
            command_manager: command_management::CommandManager::new(cx),
            command_picker: command_management::CommandPicker::new(cx),
            backend,
            backend_error: ssh_error,
            ssh_error: None,
            sessions: Vec::new(),
            profiles: Vec::new(),
            recent: Vec::new(),
            terminal_appearance: cx.new(|_| TerminalAppearance::default()),
            appearance_error: None,
            active: None,
            groups: Vec::new(),
            group_open: Vec::new(),
            show_connections: true,
            show_files: false,
            connections_width: saved_layout
                .connections
                .filter(|width| width.is_finite() && *width > 0.)
                .map(px)
                .unwrap_or(CONNECTIONS_MIN),
            files_width: saved_layout
                .files
                .filter(|width| width.is_finite() && *width > 0.)
                .map(px)
                .unwrap_or(px(320.)),
            resizing_panel: None,
            tool: None,
            tools_height: TOOLS_DEFAULT_HEIGHT,
            transfers: Arc::new(TransferQueue::default()),
            upload_conflict_sender,
            upload_conflicts: VecDeque::new(),
            transfer_error: None,
            mcp_server: None,
            mcp_start: None,
            mcp_error: None,
            mcp_auto_start,
            mcp_preference_error,
            mcp_desktop,
            mcp_requests: VecDeque::new(),
            mcp_approval: None,
            mcp_workspace,
            ai_views: Vec::new(),
            active_ai: None,
            pending_ai_end: None,
            dismissed_ai: Vec::new(),
            menu: None,
            dialog: None,
            page: Page::Terminal,
            preview: None,
            history_open: false,
            history: VecDeque::new(),
            notice: layout_notice,
            show_hidden_files: false,
            file_sort: FileSort::default(),
            compact_files: true,
            connection_filter: cx.new(|cx| TextField::new(cx, "搜索连接…")),
            management_filter: cx
                .new(|cx| TextField::new(cx, "搜索名称、地址或分组…").with_tab_index(1)),
            group_menu: None,
            group_draft: None,
            batch_group_selection: None,
            profile_title: cx.new(|cx| TextField::new(cx, "例如 开发环境").with_tab_index(1)),
            profile_address: cx.new(|cx| TextField::new(cx, "IP 地址或域名").with_tab_index(3)),
            profile_group: cx.new(|cx| TextField::new(cx, "未分组").with_tab_index(2)),
            profile_port: cx.new(|cx| TextField::new(cx, "22").with_tab_index(4)),
            profile_username: cx.new(|cx| {
                TextField::new(cx, "SSH 用户名")
                    .with_limit(4096)
                    .with_tab_index(5)
            }),
            profile_key_path: cx.new(|cx| {
                TextField::new(cx, "OpenSSH 或 PEM 私钥路径")
                    .with_limit(32_767)
                    .with_tab_index(8)
            }),
            profile_uses_key: false,
            profile_key_picker: None,
            password: cx.new(|cx| TextField::password(cx).with_tab_index(2)),
            remember_password: false,
            login_credential: LoginCredential::Manual,
            login_pending: None,
            pending_bastion: None,
            #[cfg(windows)]
            external_launches: VecDeque::new(),
            bastion_registration: None,
            bastion_username: cx.new(|cx| {
                TextField::new(cx, "SSH 用户名")
                    .with_limit(4096)
                    .with_tab_index(1)
            }),
            bastion_link: cx
                .new(|cx| TextField::secret(cx, "粘贴 SSH 或堡垒机会话链接", 32 * 1024)),
            bastion_decode: false,
            bastion_error: None,
            search_field: cx.new(|cx| TextField::new(cx, "查找终端内容…")),
            search_result: None,
            history_filter: cx.new(|cx| TextField::new(cx, "筛选命令、连接…")),
            logs: VecDeque::new(),
            focus: cx.focus_handle(),
            dialog_focus: cx.focus_handle(),
            file_focus: cx.focus_handle(),
            file_menu: None,
            file_delete: None,
            editors: editor::RemoteEditors::default(),
            quick_command_draft: None,
        };
        for profile in saved_profiles {
            let group = workbench.add_group(&profile.group, true);
            workbench.profiles.push(Profile::from_ssh(profile, group));
        }
        for field in [
            &workbench.connection_filter,
            &workbench.management_filter,
            &workbench.profile_title,
            &workbench.profile_address,
            &workbench.history_filter,
        ] {
            cx.observe(field, |_, _, cx| cx.notify()).detach();
        }
        cx.observe(&workbench.search_field, |view, _, cx| {
            view.search_result = None;
            cx.notify();
        })
        .detach();
        cx.observe(&workbench.terminal_appearance, |_, _, cx| cx.notify())
            .detach();
        workbench.observe_transfers(upload_conflicts, cx);
        workbench.observe_mcp(mcp_requests, window, cx);
        workbench.observe_ai(window, cx);
        workbench.observe_server_metrics(window, cx);
        if workbench.mcp_auto_start {
            workbench.start_mcp(crate::mcp::PORT, window, cx);
        }
        #[cfg(windows)]
        workbench.listen_external_links(window, cx);
        workbench.focus.focus(window);
        workbench
    }

    fn active_session(&self) -> Option<&Session> {
        self.visible_sessions()
            .find(|session| Some(session.profile) == self.active)
    }

    fn visible_sessions(&self) -> impl Iterator<Item = &Session> {
        self.sessions.iter().filter(|session| session.show_tab)
    }

    fn active_session_mut(&mut self) -> Option<&mut Session> {
        let active = self.active?;
        self.sessions
            .iter_mut()
            .find(|session| session.show_tab && session.profile == active)
    }

    fn open_session(&mut self, profile: usize, cx: &mut Context<Self>, window: &mut Window) {
        if self.dialog.is_some() {
            self.close_dialog(window, cx);
        }
        self.page = Page::Terminal;
        self.dialog = None;
        self.notice = None;
        if self.profiles[profile].remote.is_some() {
            let Some(session) = self
                .sessions
                .iter_mut()
                .find(|session| session.profile == profile)
            else {
                self.connect_or_prompt(profile, window, cx);
                return;
            };
            if !session.show_tab
                && session.remote.as_ref().is_some_and(RemoteSession::is_closed)
            {
                self.connect_or_prompt(profile, window, cx);
                return;
            }
            session.show_tab = true;
            let awaiting_host = session
                .remote
                .as_ref()
                .is_some_and(RemoteSession::awaiting_host);
            self.active = Some(profile);
            if awaiting_host {
                self.open_dialog(Dialog::HostKey(profile), window, cx);
            } else {
                self.focus_active(window, cx);
            }
            cx.notify();
            return;
        }
        if !self
            .sessions
            .iter()
            .any(|session| session.profile == profile)
        {
            let terminal = self.new_terminal(&self.profiles[profile].address, window, cx);
            cx.subscribe(&terminal, move |view, _, SubmittedLine(command), cx| {
                if view.history.len() == 100 {
                    view.history.pop_front();
                }
                view.history.push_back(HistoryEntry {
                    profile,
                    command: command.clone(),
                });
                cx.notify();
            })
            .detach();
            self.sessions.push(Session {
                profile,
                show_tab: true,
                terminal,
                directory: Directory::Home,
                selected_file: None,
                remote: None,
            });
            self.recent.retain(|id| *id != profile);
            self.recent.insert(0, profile);
            self.recent.truncate(6);
            self.log(format!("打开本地会话 · {}", self.profiles[profile].title));
        }
        self.active = Some(profile);
        self.focus_active(window, cx);
        cx.notify();
    }

    fn close_session(&mut self, profile: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self
            .sessions
            .iter()
            .position(|session| session.profile == profile)
        else {
            return;
        };
        if let Some(remote) = &self.sessions.remove(index).remote {
            remote.live.cancel.cancel();
        }
        self.log(format!("关闭会话 · {}", self.profiles[profile].title));
        if self.active == Some(profile) {
            self.active = self
                .sessions
                .iter()
                .skip(index)
                .find(|session| session.show_tab)
                .or_else(|| self.visible_sessions().last())
                .map(|session| session.profile);
            self.focus_active(window, cx);
        }
        cx.notify();
    }

    fn focus_active(&self, window: &mut Window, cx: &Context<Self>) {
        if self.page == Page::Editor
            && let Some(editor) = self.editors.active()
        {
            editor.focus(window, cx);
        } else if self.page == Page::Ai
            && let Some(terminal) = self.current_ai_view()
        {
            terminal.view.read(cx).focus(window);
        } else if self.page == Page::Terminal
            && let Some(session) = self.active_session()
        {
            session.terminal.read(cx).focus(window);
        } else {
            self.focus.focus(window);
        }
    }

    fn log(&mut self, message: String) {
        if self.logs.len() == 32 {
            self.logs.pop_front();
        }
        self.logs.push_back(message);
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = &event.keystroke;
        if self.handle_workspace_key(key, window, cx) {
            cx.stop_propagation();
            return;
        }
        if key.key == "tab"
            && key.modifiers.control
            && !key.modifiers.alt
            && self.switch_session(key.modifiers.shift, window, cx)
        {
            cx.stop_propagation();
        }
    }

    fn switch_session(
        &mut self,
        backwards: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let visible_profiles: Vec<_> = self
            .visible_sessions()
            .map(|session| session.profile)
            .collect();
        let ssh_count = visible_profiles.len();
        let count = ssh_count + self.ai_views.len();
        if count < 2 {
            return false;
        }
        let current = if self.page == Page::Ai {
            self.ai_views
                .iter()
                .position(|tab| self.active_ai.as_deref() == Some(tab.terminal.id.as_str()))
                .map(|index| index + ssh_count)
        } else {
            visible_profiles
                .iter()
                .position(|profile| Some(*profile) == self.active)
        };
        let index = current.map_or(if backwards { count - 1 } else { 0 }, |index| {
            (index + if backwards { count - 1 } else { 1 }) % count
        });
        if index < ssh_count {
            self.open_session(visible_profiles[index], cx, window);
        } else {
            let id = self.ai_views[index - ssh_count].terminal.id.clone();
            self.open_ai_terminal(&id, window, cx);
        }
        true
    }

    fn active_terminal_diagnostics(&self) -> Option<&Arc<ChannelDiagnostics>> {
        if self.page != Page::Terminal {
            return None;
        }
        self.active_session()?
            .remote
            .as_ref()?
            .live
            .diagnostics
            .get()
    }

    fn toggle_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_files = !self.show_files;
        if let Some(trace) = self.active_terminal_diagnostics() {
            trace.file_panel(if self.show_files {
                FilePanelEvent::Opened
            } else {
                FilePanelEvent::Closed
            });
        }
        if !self.show_files && self.file_focus.contains_focused(window, cx) {
            self.focus_active(window, cx);
        }
        cx.notify();
    }

    fn select_file(
        &mut self,
        index: usize,
        enter: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(session) = self.active_session_mut() {
            session.selected_file = Some(index);
        }
        if enter {
            self.open_selected_entry(window, cx);
        }
        cx.notify();
    }

    fn open_selected_entry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.active_session() else {
            return;
        };
        let Some(entry) = session
            .selected_file
            .and_then(|index| session.directory.entries().get(index))
        else {
            return;
        };
        if let Some(directory) = entry.opens {
            if let Some(session) = self.active_session_mut() {
                session.directory = directory;
                session.selected_file = None;
            }
        } else {
            self.preview = Some(FilePreview {
                profile: session.profile,
                name: entry.name,
                path: format!(
                    "{}/{}",
                    session.directory.path(self.profiles[session.profile].home),
                    entry.name
                ),
                contents: entry.contents(),
            });
            self.page = Page::Preview;
            self.focus_active(window, cx);
        }
        cx.notify();
    }

    fn go_to_home(&mut self, cx: &mut Context<Self>) {
        if let Some(session) = self.active_session_mut() {
            session.directory = Directory::Home;
            session.selected_file = None;
            cx.notify();
        }
    }

    fn panel_widths(&self, width: Pixels) -> (Pixels, Pixels) {
        let available = width - RAIL_WIDTH - space::MD - CENTER_MIN;
        let splitter = space::SPLITTER_WIDTH;
        let reserved_for_files = if self.show_files {
            FILES_MIN + splitter
        } else {
            px(0.)
        };
        let connections = if self.show_connections {
            self.connections_width.clamp(
                CONNECTIONS_MIN,
                (available - reserved_for_files - splitter).clamp(CONNECTIONS_MIN, CONNECTIONS_MAX),
            )
        } else {
            px(0.)
        };
        let reserved_for_connections = if self.show_connections {
            connections + splitter
        } else {
            px(0.)
        };
        let files = self
            .files_width
            .min(available - reserved_for_connections - splitter)
            .max(FILES_MIN);
        (connections, files)
    }

    fn resize_panel(
        &mut self,
        event: &DragMoveEvent<Splitter>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let side = *event.drag(cx);
        let viewport_width = window.viewport_size().width;
        let position = event.event.position.x - event.bounds.origin.x;
        let half_handle = space::SPLITTER_WIDTH / 2.;
        let (connections, files) = self.panel_widths(viewport_width);
        let panels_width =
            viewport_width - RAIL_WIDTH - space::MD - CENTER_MIN - space::SPLITTER_WIDTH;
        let (width, resized_width) = match side {
            Splitter::Connections => {
                let other = if self.show_files {
                    files + space::SPLITTER_WIDTH
                } else {
                    px(0.)
                };
                let width = (position - RAIL_WIDTH - half_handle).clamp(
                    CONNECTIONS_MIN,
                    (panels_width - other).clamp(CONNECTIONS_MIN, CONNECTIONS_MAX),
                );
                (&mut self.connections_width, width)
            }
            Splitter::Files => {
                let other = if self.show_connections {
                    connections + space::SPLITTER_WIDTH
                } else {
                    px(0.)
                };
                let width = (viewport_width - space::MD - position - half_handle)
                    .min(panels_width - other)
                    .max(FILES_MIN);
                (&mut self.files_width, width)
            }
        };
        if *width != resized_width {
            *width = resized_width;
            if self.resizing_panel.as_ref().map(|drag| drag.side) != Some(side) {
                let diagnostics = match side {
                    Splitter::Files => self.active_terminal_diagnostics().cloned(),
                    Splitter::Connections => None,
                };
                if let Some(trace) = &diagnostics {
                    trace.file_panel(FilePanelEvent::DragStarted);
                }
                self.resizing_panel = Some(PanelResize { side, diagnostics });
            }
            cx.notify();
        }
    }

    fn finish_panel_resize(
        &mut self,
        event: &MouseUpEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.button != MouseButton::Left {
            return;
        }
        let Some(drag) = self.resizing_panel.take() else {
            return;
        };
        if let Some(trace) = drag.diagnostics {
            trace.file_panel(FilePanelEvent::DragFinished);
        }
        let mut layout = SavedLayout::default();
        match drag.side {
            Splitter::Connections => layout.connections = Some(self.connections_width.into()),
            Splitter::Files => layout.files = Some(self.files_width.into()),
        }
        if !self
            .backend
            .as_ref()
            .is_some_and(|backend| backend.save_layout(layout).is_ok())
        {
            self.notice = Some("面板宽度未保存，请重新拖动后松开。");
            cx.notify();
        }
    }

    fn resize_tools(
        &mut self,
        event: &DragMoveEvent<ToolsSplitter>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let viewport_height = window.viewport_size().height;
        let position = event.event.position.y - event.bounds.origin.y;
        let bottom = viewport_height - space::MD - px(1.);
        let height = (bottom - position + space::SPLITTER_WIDTH / 2.)
            .clamp(TOOLS_MIN_HEIGHT, tools_max_height(viewport_height));
        if height != self.tools_height {
            self.tools_height = height;
            cx.notify();
        }
    }

    fn titlebar(&self, window: &Window, cx: &Context<Self>) -> Div {
        div()
            .h(space::TITLEBAR_HEIGHT)
            .flex_none()
            .flex()
            .items_center()
            .gap(space::SM)
            .pl(space::LG)
            .child(
                self.window_frame.drag_area(
                    DragArea::Brand,
                    div()
                        .h_full()
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(space::SM)
                        .min_w(px(0.))
                        .child(
                            img(ImageSource::Resource(Resource::Embedded(
                                BRAND_IMAGE.into(),
                            )))
                            .size(px(20.))
                            .flex_none(),
                        )
                        .child(
                            div()
                                .text_size(text::BRAND)
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("OrbitShell"),
                        ),
                ),
            )
            .child(self.menus(cx))
            .child(
                self.window_frame.drag_area(
                    DragArea::Space,
                    div()
                        .h_full()
                        .flex_1()
                        .min_w(px(0.))
                        .flex()
                        .items_center()
                        .child(
                            div()
                                .ml(space::SM)
                                .text_size(text::DETAIL)
                                .text_color(rgb(color::MUTED))
                                .child("SSH / SFTP"),
                        ),
                ),
            )
            .child(
                div()
                    .h_full()
                    .flex()
                    .items_center()
                    .child(caption_button(
                        "minimize-window",
                        Icon::Minimize,
                        "最小化",
                        |_, window, _| window.minimize_window(),
                    ))
                    .child(caption_button(
                        "maximize-window",
                        if window.is_maximized() {
                            Icon::Restore
                        } else {
                            Icon::Maximize
                        },
                        if window.is_maximized() {
                            "还原窗口"
                        } else {
                            "最大化"
                        },
                        |_, window, _| toggle_maximize(window),
                    ))
                    .child(caption_button(
                        "close-window",
                        Icon::Close,
                        "关闭窗口",
                        cx.listener(|view, _, window, cx| view.close_desktop_window(window, cx)),
                    )),
            )
    }

    fn activity(&self, cx: &Context<Self>) -> Div {
        div()
            .w(RAIL_WIDTH)
            .flex_none()
            .flex()
            .flex_col()
            .items_center()
            .gap(space::SM)
            .pt(space::SM)
            .child(
                icon_control(
                    "show-connections",
                    Icon::Connections,
                    "连接列表",
                    self.show_connections,
                )
                .on_click(cx.listener(|view, _, _, cx| {
                    view.show_connections = !view.show_connections;
                    cx.notify();
                })),
            )
            .child(
                icon_control("show-files", Icon::Files, "远程文件", self.show_files)
                    .on_click(cx.listener(|view, _, window, cx| view.toggle_files(window, cx))),
            )
            .child(
                icon_control("show-tools", Icon::Tools, "底部工具", self.tool.is_some()).on_click(
                    cx.listener(|view, _, _, cx| {
                        view.tool = if view.tool.is_some() {
                            None
                        } else {
                            Some(Tool::Logs)
                        };
                        cx.notify();
                    }),
                ),
            )
            .child(
                icon_control(
                    "show-history",
                    Icon::History,
                    "执行历史",
                    self.page == Page::History,
                )
                .on_click(cx.listener(|view, _, window, cx| view.show_history(window, cx))),
            )
            .child(div().flex_1())
            .child(
                icon_control(
                    "open-settings",
                    Icon::Settings,
                    "设置",
                    matches!(self.dialog, Some(Dialog::Settings(_))),
                )
                .on_click(cx.listener(|view, _, window, cx| {
                    view.open_dialog(Dialog::Settings(SettingsTab::Appearance), window, cx)
                })),
            )
    }

    fn connections(&self, width: Pixels, cx: &Context<Self>) -> Div {
        let query = self
            .connection_filter
            .read(cx)
            .value()
            .trim()
            .to_lowercase();
        let matches = |profile: &Profile| {
            !profile.temporary
                && (query.is_empty()
                    || profile.title.to_lowercase().contains(&query)
                    || profile.address.to_lowercase().contains(&query))
        };
        let mut groups = div()
            .id("connection-groups")
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll()
            .px(space::SM)
            .py(space::MD);
        for (group, title) in self.groups.iter().enumerate() {
            if !self
                .profiles
                .iter()
                .any(|profile| profile.group == group && matches(profile))
            {
                continue;
            }
            let expanded = self.group_open[group] || !query.is_empty();
            groups = groups.child(
                div()
                    .id(ElementId::named_usize("connection-group", group))
                    .h(space::ROW_HEIGHT)
                    .px(space::SM)
                    .flex()
                    .items_center()
                    .gap(space::SM)
                    .rounded(space::CONTROL_RADIUS)
                    .text_color(rgb(color::MUTED))
                    .text_size(text::DETAIL)
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(color::HOVER)))
                    .child(div().w(space::MD).child(if expanded { "▾" } else { "▸" }))
                    .child(title.clone())
                    .when(title != "未分组", |row| {
                        row.on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |view, event: &MouseDownEvent, _, cx| {
                                view.open_group_menu(GroupMenuTarget::Group(group), event.position, cx);
                                cx.stop_propagation();
                            }),
                        )
                    })
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.group_open[group] = !view.group_open[group];
                        cx.notify();
                    })),
            );
            if expanded {
                groups = groups.children(
                    self.profiles
                        .iter()
                        .enumerate()
                        .filter(|(_, item)| item.group == group && matches(item))
                        .map(|(index, profile)| {
                            let active = self.active == Some(index);
                            let is_open = self.sessions.iter().any(|session| {
                                session.profile == index
                                    && session.terminal.read(cx).input_enabled()
                            });
                            div()
                                .id(ElementId::named_usize("connection", index))
                                .h(space::ROW_HEIGHT)
                                .pl(space::XL)
                                .pr(space::SM)
                                .flex()
                                .items_center()
                                .gap(space::SM)
                                .rounded(space::CONTROL_RADIUS)
                                .cursor_pointer()
                                .bg(rgb(if active {
                                    color::SELECTED
                                } else {
                                    color::SIDEBAR
                                }))
                                .text_color(rgb(if active {
                                    color::ACCENT_TEXT
                                } else {
                                    color::TEXT
                                }))
                                .hover(move |style| {
                                    style.bg(rgb(if active {
                                        color::SELECTED_HOVER
                                    } else {
                                        color::HOVER
                                    }))
                                })
                                .child(dot(if is_open {
                                    color::ACCENT
                                } else {
                                    color::SESSION_IDLE
                                }))
                                .child(div().min_w(px(0.)).truncate().child(profile.title.clone()))
                                .when(profile.remote.is_some(), |row| {
                                    row.on_mouse_down(
                                        MouseButton::Right,
                                        cx.listener(move |view, event: &MouseDownEvent, _, cx| {
                                            view.open_group_menu(
                                                GroupMenuTarget::Host(index),
                                                event.position,
                                                cx,
                                            );
                                            cx.stop_propagation();
                                        }),
                                    )
                                })
                                .on_click(cx.listener(move |view, _, window, cx| {
                                    view.open_session(index, cx, window)
                                }))
                        }),
                );
            }
        }

        if !self.profiles.iter().any(matches) {
            groups = groups.child(
                div()
                    .p(space::SM)
                    .text_color(rgb(color::MUTED))
                    .child(if query.is_empty() {
                        "暂无 SSH 连接"
                    } else {
                        "没有匹配的连接"
                    }),
            );
        }

        panel(color::SIDEBAR)
            .w(width)
            .flex_none()
            .child(panel_heading("连接").child(
                control("sidebar-new-connection", "新建", false).on_click(cx.listener(
                    |view, _, window, cx| view.open_dialog(Dialog::Profile(None), window, cx),
                )),
            ))
            .child(
                div()
                    .px(space::MD)
                    .pt(space::SM)
                    .child(self.connection_filter.clone()),
            )
            .child(groups)
            .child(
                div()
                    .h(space::TOOLBAR_HEIGHT)
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px(space::SM)
                    .border_t_1()
                    .border_color(rgb(color::LINE))
                    .text_size(text::DETAIL)
                    .child(
                        control("sidebar-manage-connections", "管理连接", false).on_click(
                            cx.listener(|view, _, window, cx| {
                                view.open_dialog(Dialog::Connections, window, cx)
                            }),
                        ),
                    )
                    .child(
                        div()
                            .text_color(rgb(color::MUTED))
                            .child(format!("{} 个会话", self.sessions.len())),
                    ),
            )
    }

    fn center(&self, viewport_height: Pixels, cx: &Context<Self>) -> Div {
        let tabs = div()
            .id("session-tabs")
            .h(space::TOOLBAR_HEIGHT)
            .flex_none()
            .flex()
            .items_center()
            .gap(space::XS)
            .px(space::LG)
            .overflow_x_scroll()
            .bg(rgb(color::TOOLBAR))
            .border_b_1()
            .border_color(rgb(color::LINE))
            .children(self.visible_sessions().map(|session| {
                let id = session.profile;
                let active = self.page == Page::Terminal && self.active == Some(id);
                div()
                    .h(space::CONTROL_HEIGHT)
                    .flex_none()
                    .flex()
                    .items_center()
                    .rounded(space::CONTROL_RADIUS)
                    .when(active, |tab| tab.bg(rgb(color::SELECTED)))
                    .on_mouse_down(
                        MouseButton::Middle,
                        cx.listener(move |view, _, window, cx| {
                            view.close_session(id, window, cx);
                            cx.stop_propagation();
                        }),
                    )
                    .child(
                        div()
                            .id(ElementId::named_usize("activate-session", id))
                            .h_full()
                            .pl(space::SM)
                            .pr(space::XS)
                            .flex()
                            .items_center()
                            .gap(space::SM)
                            .rounded(space::CONTROL_RADIUS)
                            .cursor_pointer()
                            .text_color(rgb(if active {
                                color::ACCENT_TEXT
                            } else {
                                color::MUTED
                            }))
                            .hover(move |style| {
                                style.bg(rgb(if active {
                                    color::SELECTED_HOVER
                                } else {
                                    color::HOVER
                                }))
                            })
                            .child(dot(if active {
                                color::ACCENT
                            } else {
                                color::SESSION_IDLE
                            }))
                            .child(self.profiles[id].title.clone())
                            .when(session.terminal.read(cx).mcp_binding().is_some(), |label| {
                                label.child(div().text_size(text::DETAIL).child("MCP"))
                            })
                            .on_click(cx.listener(move |view, _, window, cx| {
                                view.open_session(id, cx, window)
                            })),
                    )
                    .child(
                        icon_control_with_size(
                            ElementId::named_usize("close-session", id),
                            Icon::Close,
                            "关闭会话",
                            active,
                            space::COMPACT_ICON_SIZE,
                        )
                        .size(space::CONTROL_HEIGHT)
                        .on_click(cx.listener(
                            move |view, _, window, cx| view.close_session(id, window, cx),
                        )),
                    )
            }))
            .children(self.ai_tabs(cx))
            .children(self.editor_tabs(cx))
            .when(self.history_open, |tabs| {
                tabs.child(self.utility_tab("执行历史", Page::History, cx))
            })
            .when_some(self.preview.as_ref(), |tabs, preview| {
                tabs.child(self.utility_tab(preview.name, Page::Preview, cx))
            });

        let session = self.active_session();
        let ssh_session = session.filter(|_| self.page == Page::Terminal);
        let server_metrics = ssh_session
            .and_then(|session| session.remote.as_ref())
            .filter(|remote| remote.supports_metrics());
        let header = div()
            .h(space::HEADER_HEIGHT)
            .flex_none()
            .flex()
            .items_center()
            .gap(space::SM)
            .px(space::LG)
            .border_b_1()
            .border_color(rgb(color::LINE))
            .child(icon(
                if self.page == Page::Editor {
                    Icon::Pencil
                } else {
                    Icon::Terminal
                },
                color::ACCENT,
            ))
            .child(
                div()
                    .flex_none()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(text::HEADING)
                    .child(match self.page {
                        Page::History => "执行历史".to_owned(),
                        Page::Ai => "独立 AI 终端".to_owned(),
                        Page::Editor => "远程文件编辑".to_owned(),
                        Page::Preview => self
                            .preview
                            .as_ref()
                            .map_or("文件预览", |preview| preview.name)
                            .to_owned(),
                        Page::Terminal => session
                            .map_or("终端", |session| {
                                self.profiles[session.profile].title.as_str()
                            })
                            .to_owned(),
                    }),
            )
            .child(
                div()
                    .min_w(px(0.))
                    .flex_1()
                    .flex()
                    .items_center()
                    .gap(space::SM)
                    .when_some(ssh_session, |row, session| {
                        row.child(
                            div()
                                .min_w(px(0.))
                                .when(server_metrics.is_some(), |address| address.max_w(relative(0.42)))
                                .truncate()
                                .text_size(text::DETAIL)
                                .text_color(rgb(color::MUTED))
                                .child(self.profiles[session.profile].address.clone()),
                        )
                    })
                    .when_some(server_metrics, |row, remote| row.child(remote.metrics.strip())),
            )
            .when(self.page == Page::Terminal && session.is_some(), |row| {
                row.child(
                    icon_control(
                        "find-in-terminal",
                        Icon::Search,
                        "查找终端内容 · Ctrl+F",
                        false,
                    )
                    .size(space::ROW_HEIGHT)
                    .on_click(cx.listener(|view, _, window, cx| {
                        view.open_dialog(Dialog::Search, window, cx)
                    })),
                )
            })
            .when_some(
                session
                    .filter(|_| self.page == Page::Terminal)
                    .and_then(|session| {
                        session
                            .terminal
                            .read(cx)
                            .mcp_binding()
                            .map(|binding| (session.profile, binding))
                    }),
                |row, (profile, binding)| {
                    row.child(
                        div()
                            .truncate()
                            .max_w(px(170.))
                            .text_size(text::DETAIL)
                            .text_color(rgb(color::ACCENT_TEXT))
                            .child(format!("{} 接管中", binding.name)),
                    )
                    .child(
                        control("release-mcp-terminal", "交回终端", true).on_click(cx.listener(
                            move |view, _, window, cx| {
                                if let Some(session) = view
                                    .sessions
                                    .iter()
                                    .find(|session| session.profile == profile)
                                {
                                    session.terminal.read(cx).release_mcp(&binding.id);
                                }
                                view.focus_active(window, cx);
                                cx.notify();
                            },
                        )),
                    )
                },
            )
            .when_some(
                session
                    .filter(|_| self.page == Page::Terminal)
                    .and_then(|session| {
                        session
                            .remote
                            .as_ref()
                            .filter(|remote| {
                                remote
                                    .live
                                    .connected
                                    .load(std::sync::atomic::Ordering::Acquire)
                            })
                            .filter(|_| session.terminal.read(cx).mcp_binding().is_none())
                            .map(|_| session.profile)
                    }),
                |row, profile| {
                    row.when(
                        self.mcp_server
                            .as_ref()
                            .is_some_and(crate::mcp::RunningMcp::is_running),
                        |row| {
                            row.child(control("hand-off-terminal", "交给 AI", false).on_click(
                                cx.listener(move |view, _, window, cx| {
                                    view.open_dialog(Dialog::AiHandoff(profile), window, cx);
                                }),
                            ))
                        },
                    )
                },
            )
            .when_some(
                session
                    .filter(|_| self.page == Page::Terminal)
                    .and_then(|s| s.remote.as_ref().map(|remote| (s.profile, remote))),
                |row, (profile, remote)| {
                    if remote.awaiting_host() {
                        row.child(control("confirm-remote-key", "确认指纹", true).on_click(
                            cx.listener(move |view, _, window, cx| {
                                view.open_dialog(Dialog::HostKey(profile), window, cx)
                            }),
                        ))
                    } else if remote.is_closed() {
                        row.child(control("reconnect-ssh", "重新连接", false).on_click(
                            cx.listener(move |view, _, window, cx| {
                                view.connect_or_prompt(profile, window, cx)
                            }),
                        ))
                    } else {
                        row
                    }
                },
            )
            .child(
                icon_control(
                    "toggle-inspector",
                    Icon::Files,
                    "切换远程文件面板",
                    self.show_files,
                )
                .size(space::ROW_HEIGHT)
                .on_click(cx.listener(|view, _, window, cx| view.toggle_files(window, cx))),
            );
        let mut center = panel(color::SURFACE)
            .relative()
            .flex_1()
            .min_w(px(0.))
            .when(
                self.visible_sessions().next().is_some()
                    || !self.ai_views.is_empty()
                    || self.editors.has_tabs()
                    || self.history_open
                    || self.preview.is_some(),
                |panel| panel.child(tabs),
            )
            .when(self.page != Page::Terminal || session.is_some(), |panel| {
                panel.child(header)
            });
        if let Some(notice) = self.notice.filter(|_| self.dialog.is_none()) {
            center = center.child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(space::SM)
                    .px(space::LG)
                    .text_size(text::DETAIL)
                    .bg(rgb(color::SELECTED))
                    .text_color(rgb(color::ACCENT_TEXT))
                    .child(div().flex_1().min_w(px(0.)).truncate().child(notice))
                    .child(
                        icon_control("dismiss-workbench-notice", Icon::Close, "关闭提示", false)
                            .size(space::CONTROL_HEIGHT)
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.notice = None;
                                cx.notify();
                            })),
                    ),
            );
        }
        center = match self.page {
            Page::History => center.child(self.history_page(cx)),
            Page::Ai => center.child(self.ai_terminal_panel(cx)),
            Page::Editor => center.child(self.editor_panel(cx)),
            Page::Preview => center.child(self.preview_page(cx)),
            Page::Terminal => match session {
                Some(session) => center.child(session.terminal.clone()),
                None => center.child(self.start_page(cx)),
            },
        };
        if let Some(tool) = self.tool {
            center = center.child(self.tools(tool, viewport_height, cx));
        }
        center.when(self.dialog == Some(Dialog::Search), |panel| {
            panel.child(gpui::deferred(self.search_layer(cx)))
        })
    }

    fn transfer_queue_control(&self, cx: &Context<Self>) -> Stateful<Div> {
        icon_control_with_size(
            "open-transfer-queue",
            Icon::Tools,
            "传输队列",
            self.tool == Some(Tool::Transfers),
            space::COMPACT_ICON_SIZE,
        )
        .size(space::CONTROL_HEIGHT)
        .on_click(cx.listener(|view, _, _, cx| {
            view.tool = if view.tool == Some(Tool::Transfers) {
                None
            } else {
                Some(Tool::Transfers)
            };
            cx.notify();
        }))
    }

    fn files(&self, width: Pixels, cx: &Context<Self>) -> Div {
        let panel = panel(color::SURFACE)
            .w(width)
            .flex_none()
            .track_focus(&self.file_focus);
        let session = self.active_session().filter(|session| {
            self.page != Page::Ai
                || self.current_ai_view().is_some_and(|tab| {
                    session
                        .remote
                        .as_ref()
                        .is_some_and(|remote| remote.id == tab.terminal.session_id)
                })
        });
        let Some(session) = session else {
            return panel.child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .px(space::MD)
                    .text_center()
                    .text_color(rgb(color::MUTED))
                    .child("打开会话后显示文件"),
            );
        };
        if let Some(remote) = &session.remote {
            let profile = session.profile;
            return panel
                .on_key_down(cx.listener(move |view, event: &KeyDownEvent, window, cx| {
                    view.handle_remote_file_key(profile, event, window, cx);
                }))
                .child(self.remote_files(session, remote, width, cx));
        }
        let directory = session.directory;
        let path = directory.path(self.profiles[session.profile].home);
        let entries = directory.entries();
        let visible = |entry: &demo::Entry| self.show_hidden_files || !entry.name.starts_with('.');

        panel
            .on_key_down(cx.listener(|view, event: &KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "enter" => view.open_selected_entry(window, cx),
                    "backspace" => view.go_to_home(cx),
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .child(
                div()
                    .h(space::TOOLBAR_HEIGHT)
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(space::XS)
                    .px(space::SM)
                    .when(directory != Directory::Home, |row| {
                        row.child(
                            control("parent-folder", "上一级", false)
                                .text_size(text::DETAIL)
                                .on_click(cx.listener(|view, _, _, cx| view.go_to_home(cx))),
                        )
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .truncate()
                            .px(space::SM)
                            .py(space::XS)
                            .rounded(space::CONTROL_RADIUS)
                            .bg(rgb(color::TOOLBAR))
                            .text_color(rgb(color::MUTED))
                            .font_family(text::MONO_FONT)
                            .text_size(text::DETAIL)
                            .child(path),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(space::XS)
                    .px(space::SM)
                    .text_size(text::DETAIL)
                    .child(
                        control("file-hidden", "隐藏文件", self.show_hidden_files)
                            .on_click(cx.listener(|view, _, _, cx| view.toggle_hidden_files(cx))),
                    )
                    .child(
                        control("file-location", "定位", false)
                            .tooltip(|_, cx| cx.new(|_| Tooltip("回到示例会话目录")).into())
                            .on_click(cx.listener(|view, _, _, cx| view.go_to_home(cx))),
                    )
                    .child(self.transfer_queue_control(cx))
                    .child(div().flex_1())
                    .child(unavailable_file_action("upload-file", "上传"))
                    .child(unavailable_file_action("download-file", "下载")),
            )
            .child(
                div()
                    .h(space::ROW_HEIGHT)
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px(space::MD)
                    .border_b_1()
                    .border_color(rgb(color::LINE))
                    .text_size(text::DETAIL)
                    .text_color(rgb(color::MUTED))
                    .child("名称")
                    .child("大小"),
            )
            .child(
                div()
                    .id("file-list")
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .p(space::XS)
                    .children(
                        entries
                            .iter()
                            .enumerate()
                            .filter(|(_, entry)| visible(entry))
                            .map(|(index, entry)| {
                                let active = session.selected_file == Some(index);
                                div()
                                    .id(ElementId::named_usize("file", index))
                                    .h(if self.compact_files {
                                        space::ROW_HEIGHT
                                    } else {
                                        space::TOOLBAR_HEIGHT
                                    })
                                    .px(space::SM)
                                    .flex()
                                    .items_center()
                                    .gap(space::SM)
                                    .rounded(space::CONTROL_RADIUS)
                                    .cursor_pointer()
                                    .bg(rgb(if active {
                                        color::SELECTED
                                    } else {
                                        color::SURFACE
                                    }))
                                    .hover(move |style| {
                                        style.bg(rgb(if active {
                                            color::SELECTED_HOVER
                                        } else {
                                            color::HOVER
                                        }))
                                    })
                                    .child(file_icon(entry.name, entry.opens.is_some()))
                                    .child(
                                        div().flex_1().min_w(px(0.)).truncate().child(entry.name),
                                    )
                                    .child(
                                        div()
                                            .flex_none()
                                            .text_color(rgb(color::MUTED))
                                            .text_size(text::DETAIL)
                                            .child(entry.size),
                                    )
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(
                                            move |view,
                                                  event: &gpui::MouseDownEvent,
                                                  window,
                                                  cx| {
                                                view.file_focus.focus(window);
                                                view.select_file(
                                                    index,
                                                    event.click_count == 2,
                                                    window,
                                                    cx,
                                                );
                                            },
                                        ),
                                    )
                            }),
                    ),
            )
            .child(panel_footer(format!(
                "{} 项",
                entries.iter().filter(|entry| visible(entry)).count()
            )))
    }

    fn tools(&self, tool: Tool, viewport_height: Pixels, cx: &Context<Self>) -> Div {
        let header = div()
            .h(space::ROW_HEIGHT)
            .flex_none()
            .flex()
            .items_center()
            .gap(space::XS)
            .px(space::SM)
            .border_b_1()
            .border_color(rgb(color::LINE))
            .children(
                [
                    (Tool::Logs, "tool-logs", "连接日志"),
                    (Tool::Transfers, "tool-transfers", "传输队列"),
                    (Tool::Commands, "tool-commands", "快捷指令"),
                    (Tool::Ai, "tool-ai", "AI 工具"),
                ]
                .map(|(tab, id, label)| {
                    control(id, label, tab == tool).on_click(cx.listener(move |view, _, _, cx| {
                        view.tool = Some(tab);
                        cx.notify();
                    }))
                }),
            )
            .child(div().flex_1())
            .when(tool == Tool::Logs && self.page == Page::Terminal, |header| {
                let capture = self
                    .active_session()
                    .and_then(|session| session.remote.as_ref())
                    .and_then(|remote| {
                        let diagnostics = remote.live.diagnostics.get()?.clone();
                        Some((remote.id.clone(), diagnostics))
                    });
                header.when_some(capture, |header, (id, diagnostics)| {
                    header.child(
                        control("save-ssh-diagnostics", "保存本次 SSH 诊断", false)
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.save_ssh_diagnostics(id.clone(), diagnostics.clone(), cx);
                            })),
                    )
                })
            })
            .when(tool == Tool::Transfers, |header| {
                header.child(
                    control("transfer-history", "传输记录", false).on_click(cx.listener(
                        |view, _, window, cx| view.open_dialog(Dialog::Transfers, window, cx),
                    )),
                )
            })
            .when(tool == Tool::Commands, |header| {
                header.child(
                    icon_control_with_size(
                        "manage-quick-commands",
                        Icon::Settings,
                        "管理快捷指令",
                        false,
                        space::COMPACT_ICON_SIZE,
                    )
                    .size(space::CONTROL_HEIGHT)
                    .on_click(cx.listener(|view, _, window, cx| {
                        view.open_quick_command_manager(window, cx);
                    })),
                )
            })
            .child(control("close-tools", "收起", false).on_click(cx.listener(
                |view, _, _, cx| {
                    view.tool = None;
                    cx.notify();
                },
            )));
        let body = div()
            .id(if tool == Tool::Commands {
                "tool-commands-body"
            } else {
                "tool-body"
            })
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll()
            .when(tool == Tool::Commands, |body| {
                body.track_scroll(self.command_picker.scroll())
            })
            .px(space::MD)
            .py(space::SM)
            .text_color(rgb(color::MUTED))
            .text_size(text::DETAIL);
        let body = match tool {
            Tool::Logs => body.children(
                self.logs
                    .iter()
                    .rev()
                    .cloned()
                    .map(|event| div().py(space::XS).child(event)),
            ),
            Tool::Transfers => body.child(self.transfer_rows(cx)),
            Tool::Commands => body.child(self.quick_commands(cx)),
            Tool::Ai => body.child(self.agent_panel(cx)),
        };
        div()
            .h(self.tools_height.min(tools_max_height(viewport_height)))
            .flex_none()
            .flex()
            .flex_col()
            .child(
                div()
                    .id("resize-tools")
                    .h(space::SPLITTER_WIDTH)
                    .flex_none()
                    .flex()
                    .items_center()
                    .cursor(CursorStyle::ResizeUpDown)
                    .hover(|grip| grip.bg(rgb(color::SELECTED_HOVER)))
                    .child(div().flex_1().h(px(1.)).bg(rgb(color::LINE)))
                    .child(
                        div()
                            .w(px(26.))
                            .h(px(2.))
                            .rounded_full()
                            .bg(rgb(color::SESSION_IDLE)),
                    )
                    .child(div().flex_1().h(px(1.)).bg(rgb(color::LINE)))
                    .on_drag(ToolsSplitter, |_, _, _, cx| cx.new(|_| ResizePreview)),
            )
            .child(header)
            .when(tool == Tool::Commands, |panel| {
                panel.child(self.quick_command_filters(cx))
            })
            .child(body)
    }
}

impl Render for Workbench {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(windows)]
        self.show_external_link(window, cx);
        self.show_agent_host_prompt(window, cx);
        self.show_mcp_request(window, cx);
        self.show_pending_upload(window, cx);
        if self.tool != Some(Tool::Commands) || self.dialog.is_some() {
            self.command_picker.dismiss_groups();
        }
        if self.dialog.is_some() || !self.show_connections {
            self.group_menu = None;
        }
        self.command_picker.sync_groups(&self.quick_commands.groups);
        if self.dialog.is_some()
            || self
                .file_menu
                .as_ref()
                .is_some_and(|menu| !self.file_target_is_current(&menu.target))
        {
            self.file_menu = None;
        }
        self.mcp_workspace.set_active(
            (self.page == Page::Terminal)
                .then(|| self.active_session())
                .flatten()
                .and_then(|session| session.remote.as_ref())
                .map(|remote| remote.id.as_str()),
        );
        let viewport_size = window.viewport_size();
        let (connections_width, files_width) = self.panel_widths(viewport_size.width);
        div()
            .id("orbitshell-workbench")
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(color::FRAME))
            .text_color(rgb(color::TEXT))
            .font_family(text::UI_FONT)
            .text_size(text::BODY)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key_down))
            .on_drag_move(cx.listener(Self::resize_panel))
            .on_drag_move(cx.listener(Self::resize_tools))
            .capture_any_mouse_up(cx.listener(Self::finish_panel_resize))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::finish_panel_resize))
            .child(self.titlebar(window, cx))
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .flex()
                    .pr(space::MD)
                    .pb(space::MD)
                    .child(self.activity(cx))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .flex()
                            .overflow_hidden()
                            .rounded(space::WORKSPACE_RADIUS)
                            .border_1()
                            .border_color(rgb(color::LINE))
                            .bg(rgb(color::SURFACE))
                            .when(self.show_connections, |row| {
                                row.child(self.connections(connections_width, cx))
                                    .child(splitter("resize-connections", Splitter::Connections))
                            })
                            .child(self.center(viewport_size.height, cx))
                            .when(self.show_files, |row| {
                                row.child(splitter("resize-files", Splitter::Files))
                                    .child(self.files(files_width, cx))
                            }),
                    ),
            )
            .when_some(self.file_menu.as_ref(), |root, menu| {
                root.child(gpui::deferred(self.remote_file_context_menu(menu, cx)))
            })
            .when_some(self.group_menu.as_ref(), |root, menu| {
                root.child(gpui::deferred(self.group_context_menu(menu, cx)))
            })
            .when_some(
                self.dialog.filter(|dialog| *dialog != Dialog::Search),
                |root, dialog| root.child(gpui::deferred(self.dialog_layer(dialog, window, cx))),
            )
    }
}

fn panel(background: ThemeColor) -> Div {
    div()
        .flex()
        .flex_col()
        .overflow_hidden()
        .bg(rgb(background))
}

fn unavailable_file_action(id: &'static str, label: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .h(space::CONTROL_HEIGHT)
        .px(space::XS)
        .flex_none()
        .flex()
        .items_center()
        .text_color(rgb(color::MUTED))
        .opacity(0.65)
        .child(label)
        .tooltip(|_, cx| cx.new(|_| Tooltip("接入 SFTP 后可用")).into())
}

fn panel_heading(title: &'static str) -> Div {
    div()
        .h(space::HEADER_HEIGHT)
        .flex_none()
        .flex()
        .items_center()
        .justify_between()
        .px(space::MD)
        .border_b_1()
        .border_color(rgb(color::LINE))
        .child(
            div()
                .text_size(text::HEADING)
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
}

fn panel_footer(label: String) -> Div {
    div()
        .h(space::FOOTER_HEIGHT)
        .flex_none()
        .flex()
        .items_center()
        .px(space::MD)
        .border_t_1()
        .border_color(rgb(color::LINE))
        .text_size(text::DETAIL)
        .text_color(rgb(color::MUTED))
        .child(label)
}

fn dot(color: ThemeColor) -> Div {
    div().size(px(6.)).flex_none().rounded_full().bg(rgb(color))
}

fn icon_control(
    id: impl Into<ElementId>,
    glyph: Icon,
    label: &'static str,
    selected: bool,
) -> Stateful<Div> {
    icon_control_with_size(id, glyph, label, selected, space::ICON_SIZE)
}

fn icon_control_with_size(
    id: impl Into<ElementId>,
    glyph: Icon,
    label: &'static str,
    selected: bool,
    glyph_size: Pixels,
) -> Stateful<Div> {
    div()
        .id(id)
        .size(px(35.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(space::CONTROL_RADIUS)
        .cursor_pointer()
        .when(selected, |button| button.bg(rgb(color::SELECTED)))
        .hover(move |style| {
            style.bg(rgb(if selected {
                color::SELECTED_HOVER
            } else {
                color::HOVER
            }))
        })
        .child(
            icon(
                glyph,
                if selected {
                    color::ACCENT_TEXT
                } else {
                    color::MUTED
                },
            )
            .size(glyph_size),
        )
        .tooltip(move |_, cx| cx.new(|_| Tooltip(label)).into())
}

fn caption_button(
    id: &'static str,
    glyph: Icon,
    label: &'static str,
    action: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> Stateful<Div> {
    div()
        .id(id)
        .w(px(36.))
        .h_full()
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .hover(move |style| {
            style.bg(rgb(if matches!(glyph, Icon::Close) {
                color::WINDOW_CLOSE_HOVER
            } else {
                color::SELECTED_HOVER
            }))
        })
        .child(icon(glyph, color::TEXT).size(space::COMPACT_ICON_SIZE))
        .tooltip(move |_, cx| cx.new(|_| Tooltip(label)).into())
        .on_click(action)
}

fn tools_max_height(viewport_height: Pixels) -> Pixels {
    (viewport_height - space::TITLEBAR_HEIGHT - space::MD - px(2.) - CENTER_MIN_HEIGHT_WITH_TOOLS)
        .max(TOOLS_MIN_HEIGHT)
}

fn splitter(id: &'static str, side: Splitter) -> Stateful<Div> {
    div()
        .id(id)
        .w(space::SPLITTER_WIDTH)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .cursor(CursorStyle::ResizeLeftRight)
        .hover(|style| style.bg(rgb(color::SELECTED_HOVER)))
        .child(div().w(px(1.)).h_full().bg(rgb(color::LINE)))
        .on_drag(side, |_, _, _, cx| cx.new(|_| ResizePreview))
}
