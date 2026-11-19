use std::{cmp::Ordering, sync::Arc, time::Duration};

use chrono::{DateTime, Local, Utc};
use gpui::{
    Context, Div, ElementId, Entity, ExternalPaths, FontWeight, KeyDownEvent, MouseButton, Pixels,
    Point, Stateful, Window, div, point, prelude::*, px,
};
use orbitshell_ssh::{
    ConnectionProfile, credentials,
    channel_diagnostics::ChannelDiagnostics,
    session::{LiveSession, SessionCommand, SessionEvent},
    sftp::{RemoteDirectory, RemoteEntry},
};
use tokio::task::AbortHandle;

use super::file_menu::{FileMenu, RemoteDelete};
use super::transfers::{DownloadTarget, UploadTarget};
use super::{
    Dialog, LoginOrigin, Page, Profile, Session, Workbench, actions::dialog_tab, control,
    icon_control_with_size, panel_footer,
};
use crate::{
    demo::Directory,
    file_icons::file_icon,
    icons::{Icon, icon},
    text_field::TextField,
    theme::{color, primary_control, rgb, space, text},
};

struct Fingerprint {
    hostname: String,
    port: u16,
    fingerprint: String,
    algorithm: String,
}

enum Status {
    Connecting,
    Verify(Fingerprint),
    Connected,
    Closed,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum LoginCredential {
    Manual,
    Saved,
    ReplaceSaved,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FileColumn {
    Name,
    Size,
    Modified,
}

#[derive(Clone, Copy)]
pub(super) struct FileSort {
    column: FileColumn,
    descending: bool,
}

impl Default for FileSort {
    fn default() -> Self {
        Self {
            column: FileColumn::Name,
            descending: false,
        }
    }
}

impl FileSort {
    fn select(&mut self, column: FileColumn) {
        self.descending = if self.column == column {
            !self.descending
        } else {
            column != FileColumn::Name
        };
        self.column = column;
    }

    fn apply(self, entries: &mut [RemoteEntry]) {
        let direction = |order: Ordering| {
            if self.descending {
                order.reverse()
            } else {
                order
            }
        };
        let name = |left: &RemoteEntry, right: &RemoteEntry| {
            left.name
                .chars()
                .flat_map(char::to_lowercase)
                .cmp(right.name.chars().flat_map(char::to_lowercase))
                .then_with(|| left.name.cmp(&right.name))
        };
        entries.sort_by(|left, right| {
            right.is_directory.cmp(&left.is_directory).then_with(|| {
                match self.column {
                    FileColumn::Name => direction(name(left, right)),
                    FileColumn::Size if !left.is_directory => self.compare_values(
                        left.has_size.then_some(left.size),
                        right.has_size.then_some(right.size),
                    ),
                    FileColumn::Size => Ordering::Equal,
                    FileColumn::Modified => {
                        self.compare_values(left.modified_at, right.modified_at)
                    }
                }
                .then_with(|| name(left, right))
            })
        });
    }

    fn compare_values<T: Ord>(self, left: Option<T>, right: Option<T>) -> Ordering {
        match (left, right) {
            (Some(left), Some(right)) if self.descending => right.cmp(&left),
            (Some(left), Some(right)) => left.cmp(&right),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        }
    }
}

pub(super) struct RemoteSession {
    pub(super) id: String,
    pub(super) live: Arc<LiveSession>,
    pub(super) metrics: super::metrics::RemoteMetrics,
    status: Status,
    message: String,
    needs_credentials: bool,
    credential_warning: Option<String>,
    directory: Option<RemoteDirectory>,
    selection: Option<usize>,
    path_field: Entity<TextField>,
    requested_path: String,
    directory_request: Option<AbortHandle>,
    directory_error: Option<String>,
    pub(super) file_action_error: Option<String>,
    locating_directory: bool,
}

impl RemoteSession {
    fn sort_files(&mut self, sort: FileSort) {
        let Some(directory) = &mut self.directory else {
            return;
        };
        let selected_path = self
            .selection
            .and_then(|index| directory.entries.get(index))
            .map(|entry| entry.path.clone());
        sort.apply(&mut directory.entries);
        self.selection = selected_path.and_then(|path| {
            directory
                .entries
                .iter()
                .position(|entry| entry.path == path)
        });
    }

    pub(super) fn ready_directory(&self) -> Option<&RemoteDirectory> {
        self.directory.as_ref().filter(|_| {
            matches!(self.status, Status::Connected)
                && !self.live.cancel.is_cancelled()
                && self.directory_request.is_none()
                && self.directory_error.is_none()
        })
    }

    pub(super) fn awaiting_host(&self) -> bool {
        matches!(self.status, Status::Verify(_))
    }

    pub(super) fn is_closed(&self) -> bool {
        matches!(self.status, Status::Closed)
    }

    pub(super) fn hide_dot_selection(&mut self) {
        if self
            .directory
            .as_ref()
            .and_then(|dir| dir.entries.get(self.selection?))
            .is_some_and(|entry| entry.name.starts_with('.'))
        {
            self.selection = None;
        }
    }
}

impl Workbench {
    fn select_file_sort(
        &mut self,
        column: FileColumn,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.file_sort.select(column);
        for remote in self
            .sessions
            .iter_mut()
            .filter_map(|session| session.remote.as_mut())
        {
            remote.sort_files(self.file_sort);
        }
        self.file_menu = None;
        self.file_focus.focus(window);
        cx.notify();
    }

    fn file_sort_header(
        &self,
        id: &'static str,
        title: &'static str,
        column: FileColumn,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let active = self.file_sort.column == column;
        let label = if active {
            let arrow = if self.file_sort.descending {
                "↓"
            } else {
                "↑"
            };
            format!("{title} {arrow}")
        } else {
            title.to_owned()
        };
        control(id, label, active)
            .px(space::XS)
            .on_click(cx.listener(move |view, _, window, cx| {
                view.select_file_sort(column, window, cx);
            }))
    }

    pub(super) fn add_group(&mut self, name: &str, open: bool) -> usize {
        if let Some(index) = self.groups.iter().position(|group| group == name) {
            self.group_open[index] |= open;
            return index;
        }
        self.groups.push(name.to_owned());
        self.group_open.push(open);
        self.groups.len() - 1
    }

    pub(super) fn populate_profile(&mut self, index: Option<usize>, cx: &mut Context<Self>) {
        let profile = index.and_then(|i| self.profiles[i].remote.as_ref());
        let values = [
            profile.map_or("", |p| p.name.as_str()).to_owned(),
            profile.map_or("", |p| p.hostname.as_str()).to_owned(),
            profile.map_or(22, |p| p.port).to_string(),
            profile.map_or("", |p| p.username.as_str()).to_owned(),
            profile.map_or("", |p| p.group.as_str()).to_owned(),
            profile
                .and_then(|p| p.private_key_path.clone())
                .unwrap_or_default(),
        ];
        self.profile_uses_key = profile.is_some_and(|p| p.auth_method == "privateKey");
        for (field, value) in [
            &self.profile_title,
            &self.profile_address,
            &self.profile_port,
            &self.profile_username,
            &self.profile_group,
            &self.profile_key_path,
        ]
        .into_iter()
        .zip(values)
        {
            field.update(cx, |input, cx| input.set_value(&value, cx));
        }
    }

    pub(super) fn can_edit_profile(&self, index: usize) -> bool {
        !self.profiles[index].temporary
            && !self.sessions.iter().any(|session| {
                session.profile == index
                    && session
                        .remote
                        .as_ref()
                        .is_some_and(|remote| !remote.is_closed())
            })
    }

    pub(super) fn save_ssh_profile(
        &mut self,
        index: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let previous = if let Some(index) = index {
            let Some(remote) = self.profiles[index].remote.as_ref() else {
                return;
            };
            if !self.can_edit_profile(index) {
                self.ssh_error = Some("请先断开此 SSH 会话再保存连接配置".into());
                cx.notify();
                return;
            }
            Some(remote)
        } else {
            None
        };
        let field = |input: &gpui::Entity<crate::text_field::TextField>| {
            input.read(cx).value().trim().to_owned()
        };
        let Ok(port) = field(&self.profile_port).parse::<u16>() else {
            self.ssh_error = Some("请填写 1–65535 之间的端口".into());
            cx.notify();
            return;
        };
        let mut profile = ConnectionProfile {
            id: previous
                .map(|profile| profile.id.clone())
                .unwrap_or_default(),
            name: field(&self.profile_title),
            group: field(&self.profile_group),
            hostname: field(&self.profile_address),
            port,
            username: field(&self.profile_username),
            auth_method: if self.profile_uses_key {
                "privateKey"
            } else {
                "password"
            }
            .into(),
            private_key_path: self.profile_uses_key.then(|| field(&self.profile_key_path)),
        };
        let result = profile.validate().and_then(|()| {
            self.backend
                .as_ref()
                .ok_or_else(|| "SSH 配置不可用".to_string())?
                .save_profile(&profile, previous)
                .map_err(|error| format!("{error:#}"))
        });
        let warning = match result {
            Ok(warning) => warning,
            Err(error) => {
                self.ssh_error = Some(error);
                cx.notify();
                return;
            }
        };
        let identity_changed =
            previous.is_some_and(|previous| !credentials::same_identity(previous, &profile));
        let group = self.add_group(&profile.group, true);
        let profile = Profile::from_ssh(profile, group);
        if let Some(index) = index {
            self.profiles[index] = profile;
            if identity_changed {
                self.recent.retain(|&profile| profile != index);
                if let Some(remote) = self
                    .sessions
                    .iter_mut()
                    .find(|session| session.profile == index)
                    .and_then(|session| session.remote.as_mut())
                {
                    remote.needs_credentials = false;
                }
            }
        } else {
            self.profiles.push(profile);
        }
        self.open_dialog(Dialog::Connections, window, cx);
        self.ssh_error = warning;
        cx.notify();
    }

    pub(super) fn connect_or_prompt(
        &mut self,
        profile: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.connect_or_prompt_from(profile, LoginOrigin::User, window, cx);
    }

    pub(super) fn connect_or_prompt_from(
        &mut self,
        profile: usize,
        origin: LoginOrigin,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.profiles[profile].temporary {
            self.open_dialog(Dialog::Settings(super::SettingsTab::Bastion), window, cx);
            self.bastion_error = Some("临时会话需要重新打开堡垒机链接".into());
            cx.notify();
            return;
        }
        let dialog = Dialog::Login(profile, origin);
        if self.dialog == Some(dialog) {
            return;
        }
        self.open_dialog(dialog, window, cx);
        if let Some(remote) = self
            .sessions
            .iter()
            .find(|session| session.profile == profile)
            .and_then(|session| session.remote.as_ref())
            .filter(|remote| remote.needs_credentials)
        {
            self.ssh_error = Some(remote.message.clone());
            cx.notify();
            return;
        }
        let Some((backend, remote)) = self
            .backend
            .as_ref()
            .zip(self.profiles[profile].remote.as_ref())
        else {
            return;
        };
        let check = backend.has_saved_credential(remote.clone());
        self.dialog_focus.focus(window);
        self.login_pending = Some(cx.spawn_in(window, async move |view, cx| {
            let result = check
                .await
                .unwrap_or_else(|_| Err("无法检查已保存凭据".into()));
            let _ = view.update_in(cx, |view, window, cx| {
                if view.dialog != Some(dialog) {
                    return;
                }
                view.login_pending = None;
                if matches!(result, Ok(true)) {
                    view.login_credential = LoginCredential::Saved;
                    view.dialog_focus.focus(window);
                } else {
                    view.ssh_error = result.err();
                    view.password.read(cx).focus(window);
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn start_remote(
        &mut self,
        profile: usize,
        origin: LoginOrigin,
        saved: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.dialog != Some(Dialog::Login(profile, origin)) {
            return;
        }
        let Some(remote) = self.profiles[profile].remote.as_ref() else {
            return;
        };
        let temporary = self.profiles[profile].temporary;
        if temporary
            && !self.pending_bastion.as_ref().is_some_and(|launch| {
                launch.profile.id == remote.id
                    && launch.created.elapsed() < Duration::from_secs(120)
            })
        {
            self.connect_or_prompt(profile, window, cx);
            return;
        }
        let Some(backend) = &self.backend else {
            self.ssh_error = Some("SSH 运行时不可用".into());
            cx.notify();
            return;
        };
        let existing = self
            .sessions
            .iter()
            .position(|session| session.profile == profile);
        if existing.is_some_and(|i| {
            self.sessions[i]
                .remote
                .as_ref()
                .is_some_and(|remote| !remote.is_closed())
        }) {
            if origin == LoginOrigin::User {
                self.open_session(profile, cx, window);
            } else {
                self.close_dialog(window, cx);
            }
            return;
        }
        if !saved && remote.auth_method == "password" && !self.password.read(cx).has_value() {
            self.ssh_error = Some("请输入 SSH 密码".into());
            cx.notify();
            return;
        }
        let mut config = remote.clone();
        if temporary && config.username.is_empty() {
            config.username = self.bastion_username.read(cx).value().trim().to_owned();
        }
        let temporary_username = temporary.then(|| config.username.clone());
        if let Err(error) = config.validate() {
            self.ssh_error = Some(error);
            cx.notify();
            return;
        }
        if let Some(username) = temporary_username {
            config.username = username;
        }

        let terminal = existing.map_or_else(
            || self.new_terminal("", window, cx),
            |index| self.sessions[index].terminal.clone(),
        );
        let show_tab = origin == LoginOrigin::User
            || existing.is_some_and(|index| self.sessions[index].show_tab);
        let mut secret = self.password.update(cx, |input, cx| input.take_secret(cx));
        if temporary {
            if let Some(mut launch) = self.pending_bastion.take()
                && saved
            {
                secret = zeroize::Zeroizing::new(launch.credential.take().unwrap_or_default());
            }
        } else if saved {
            secret = Default::default();
        }
        let remember = !temporary && !saved && self.remember_password && !secret.is_empty();
        let (transport, mut events) = backend.connect(
            config,
            temporary,
            secret,
            saved && !temporary,
            remember,
            terminal.read(cx).pty_size(),
        );
        let live = transport.live.clone();
        let id = transport.id.clone();
        terminal.update(cx, |view, cx| view.attach(transport, cx));
        let session = Session {
            profile,
            show_tab,
            terminal,
            directory: Directory::Home,
            selected_file: None,
            remote: Some(RemoteSession {
                id,
                live: live.clone(),
                metrics: Default::default(),
                status: Status::Connecting,
                message: "正在连接 SSH…".into(),
                needs_credentials: false,
                credential_warning: None,
                directory: None,
                selection: None,
                path_field: cx.new(|cx| TextField::new(cx, "输入远程路径").with_limit(4096)),
                requested_path: String::new(),
                directory_request: None,
                directory_error: None,
                file_action_error: None,
                locating_directory: false,
            }),
        };
        if let Some(index) = existing {
            self.sessions[index] = session;
        } else {
            self.sessions.push(session);
        }
        if show_tab {
            self.active = Some(profile);
            self.page = Page::Terminal;
        }
        self.dialog = None;
        self.ssh_error = None;
        self.focus_active(window, cx);
        cx.notify();

        cx.spawn_in(window, async move |workbench, cx| {
            let mut next = None;
            loop {
                let Some(event) = (match next.take() {
                    Some(event) => Some(event),
                    None => events.recv().await,
                }) else {
                    break;
                };
                let event = if let SessionEvent::Data { mut data } = event {
                    while data.len() < 64 * 1024 {
                        match events.try_recv() {
                            Ok(SessionEvent::Data { data: part }) => data.extend(part),
                            Ok(event) => {
                                next = Some(event);
                                break;
                            }
                            Err(_) => break,
                        }
                    }
                    SessionEvent::Data { data }
                } else {
                    event
                };
                let bytes = match &event {
                    SessionEvent::Data { data } => data.len(),
                    _ => 0,
                };
                let handled = workbench.update_in(cx, |view, window, cx| {
                    view.ssh_event(profile, &live, event, window, cx)
                });
                if !matches!(handled, Ok(true)) {
                    break;
                }
                if bytes != 0 {
                    let _ = live.commands.send(SessionCommand::Acknowledge(bytes)).await;
                    if bytes >= 64 * 1024 {
                        cx.background_executor()
                            .timer(Duration::from_millis(1))
                            .await;
                    }
                }
            }
        })
        .detach();
    }

    fn ssh_event(
        &mut self,
        profile: usize,
        live: &Arc<LiveSession>,
        event: SessionEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(session) = self
            .sessions
            .iter_mut()
            .find(|session| session.profile == profile)
        else {
            return false;
        };
        let Some(remote) = session
            .remote
            .as_mut()
            .filter(|remote| Arc::ptr_eq(&remote.live, live))
        else {
            return false;
        };
        let show_tab = session.show_tab;
        let mut open_home = false;
        let mut open_verification = false;
        let mut failed = false;
        let mut log = None;
        match event {
            SessionEvent::Data { data } => {
                session
                    .terminal
                    .update(cx, |view, cx| view.receive_remote(&data, cx));
                return true;
            }
            SessionEvent::HostKey {
                hostname,
                port,
                fingerprint,
                algorithm,
            } => {
                remote.status = Status::Verify(Fingerprint {
                    hostname,
                    port,
                    fingerprint,
                    algorithm,
                });
                remote.message = "请确认服务器指纹".into();
                open_verification = show_tab
                    && self.active == Some(profile)
                    && self.page == Page::Terminal
                    && self.dialog.is_none();
            }
            SessionEvent::Status { status, message } => {
                open_home = status == "connected";
                remote.status = if open_home {
                    Status::Connected
                } else {
                    Status::Connecting
                };
                remote.message = message;
                if open_home {
                    log = Some("SSH 已连接".to_owned());
                }
            }
            SessionEvent::CredentialWarning { message } => {
                remote.credential_warning = Some(message.clone());
                log = Some(message);
            }
            SessionEvent::Error {
                message,
                authentication,
            } => {
                failed = true;
                remote.status = Status::Closed;
                remote.needs_credentials = authentication;
                remote.message = message.clone();
                log = Some(message);
            }
            SessionEvent::Exit { message, .. } => {
                remote.status = Status::Closed;
                remote.message = message.clone();
                log = Some(message);
            }
        }
        let is_closed = remote.is_closed();
        if is_closed && let Some(request) = remote.directory_request.take() {
            request.abort();
        }
        let login_error = (remote.needs_credentials && !self.profiles[profile].temporary)
            .then(|| remote.message.clone());
        let connected = matches!(remote.status, Status::Connected);
        let terminal_message = if connected {
            remote.credential_warning.as_deref().unwrap_or_default()
        } else {
            remote.message.as_str()
        };
        session.terminal.update(cx, |view, cx| {
            view.set_remote_status(terminal_message.to_owned(), connected, cx)
        });
        if let Some(message) = log {
            self.log(format!("{} · {message}", self.profiles[profile].title));
        }
        if is_closed && self.dialog == Some(Dialog::HostKey(profile)) {
            self.dialog = None;
            self.focus_active(window, cx);
        }
        if let Some(error) = login_error
            && (!show_tab || (self.active == Some(profile) && self.page == Page::Terminal))
            && self.dialog.is_none()
        {
            let origin = if show_tab {
                LoginOrigin::User
            } else {
                LoginOrigin::Agent
            };
            self.open_dialog(Dialog::Login(profile, origin), window, cx);
            self.ssh_error = Some(error);
        } else if failed && !show_tab {
            self.notice = Some("Agent 发起的 SSH 连接失败，可在连接日志查看原因。");
        }
        if open_verification {
            self.open_dialog(Dialog::HostKey(profile), window, cx);
        }
        if open_home {
            if !self.profiles[profile].temporary {
                self.recent.retain(|id| *id != profile);
                self.recent.insert(0, profile);
                self.recent.truncate(6);
            }
            self.load_remote_dir(profile, String::new(), cx);
        }
        cx.notify();
        true
    }

    pub(super) fn save_ssh_diagnostics(
        &self,
        session_id: String,
        diagnostics: Arc<ChannelDiagnostics>,
        cx: &mut Context<Self>,
    ) {
        let Some(backend) = &self.backend else { return };
        let task = backend.save_ssh_diagnostics(session_id, diagnostics);
        cx.spawn(async move |view, cx| {
            let result = match task.await {
                Ok(Ok(path)) => format!("SSH 诊断已保存到 {}", path.display()),
                Ok(Err(error)) => format!("保存 SSH 诊断失败：{error:#}"),
                Err(_) => "保存 SSH 诊断的任务已中断".into(),
            };
            let _ = view.update(cx, |view, cx| {
                view.log(result);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn show_agent_host_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dialog.is_some() {
            return;
        }
        if let Some(profile) = self.sessions.iter().find_map(|session| {
            let remote = session.remote.as_ref()?;
            (!session.show_tab && remote.awaiting_host() && !remote.live.cancel.is_cancelled())
                .then_some(session.profile)
        }) {
            self.open_dialog(Dialog::HostKey(profile), window, cx);
        }
    }

    pub(super) fn answer_host(
        &mut self,
        profile: usize,
        accept: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(session) = self
            .sessions
            .iter_mut()
            .find(|session| session.profile == profile)
            && let Some(remote) = session
                .remote
                .as_mut()
                .filter(|remote| remote.awaiting_host())
        {
            remote.message = match remote.live.trust_host(accept) {
                Ok(()) if accept => "正在验证登录信息…".into(),
                Ok(()) => "已拒绝服务器指纹".into(),
                Err(error) => error,
            };
            remote.status = Status::Connecting;
            session.terminal.update(cx, |view, cx| {
                view.set_remote_status(remote.message.clone(), false, cx)
            });
        }
        self.dialog = None;
        self.focus_active(window, cx);
        cx.notify();
    }

    pub(super) fn load_remote_dir(&mut self, profile: usize, path: String, cx: &mut Context<Self>) {
        let Some(backend) = &self.backend else { return };
        let Some(remote) = self
            .sessions
            .iter_mut()
            .find(|session| session.profile == profile)
            .and_then(|session| session.remote.as_mut())
            .filter(|remote| matches!(remote.status, Status::Connected))
        else {
            return;
        };
        if remote.directory_request.is_some() && remote.requested_path == path {
            return;
        }
        remote.directory_error = None;
        remote.file_action_error = None;
        remote.requested_path = path.clone();
        let path_field = remote.path_field.clone();
        path_field.update(cx, |field, cx| field.set_value(&path, cx));
        let submitted_text = path_field.read(cx).value().to_owned();
        let live = remote.live.clone();
        let task = backend.list(live.clone(), path);
        let request = task.abort_handle();
        let request_id = request.id();
        if let Some(previous) = remote.directory_request.replace(request) {
            previous.abort();
        }
        cx.spawn(async move |view, cx| {
            let result = task
                .await
                .unwrap_or_else(|_| Err("目录读取任务已中断，请重试".into()));
            let _ = view.update(cx, |view, cx| {
                let file_sort = view.file_sort;
                let Some(remote) = view
                    .sessions
                    .iter_mut()
                    .find(|session| session.profile == profile)
                    .and_then(|session| session.remote.as_mut())
                    .filter(|remote| {
                        Arc::ptr_eq(&remote.live, &live)
                            && matches!(remote.status, Status::Connected)
                            && remote
                                .directory_request
                                .as_ref()
                                .is_some_and(|request| request.id() == request_id)
                    })
                else {
                    return;
                };
                remote.directory_request = None;
                match result {
                    Ok(mut directory) => {
                        file_sort.apply(&mut directory.entries);
                        remote.path_field.update(cx, |field, cx| {
                            if !field.is_composing() && field.value() == submitted_text {
                                field.set_value(&directory.path, cx);
                            }
                        });
                        remote.requested_path = directory.path.clone();
                        remote.directory = Some(directory);
                        remote.selection = None;
                    }
                    Err(error) => remote.directory_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn refresh_uploaded_directory(
        &mut self,
        target: &UploadTarget,
        cx: &mut Context<Self>,
    ) {
        if self
            .sessions
            .iter()
            .find(|session| session.profile == target.profile)
            .and_then(|session| session.remote.as_ref())
            .is_some_and(|remote| {
                Arc::ptr_eq(&remote.live, &target.live)
                    && remote.directory_request.is_none()
                    && remote.path_field.read(cx).value() == target.directory
                    && remote
                        .directory
                        .as_ref()
                        .is_some_and(|directory| directory.path == target.directory)
            })
        {
            self.load_remote_dir(target.profile, target.directory.clone(), cx);
        }
    }

    fn open_remote_entry(&mut self, profile: usize, cx: &mut Context<Self>) {
        if let Some(entry) = self
            .active_session()
            .filter(|session| session.profile == profile)
            .and_then(|session| session.remote.as_ref())
            .and_then(|remote| remote.directory.as_ref()?.entries.get(remote.selection?))
            .filter(|entry| entry.is_directory || entry.is_symlink)
        {
            self.load_remote_dir(profile, entry.path.clone(), cx);
        }
    }

    pub(super) fn go_remote_parent(&mut self, profile: usize, cx: &mut Context<Self>) {
        if let Some(path) = self
            .active_session()
            .filter(|session| session.profile == profile)
            .and_then(|session| session.remote.as_ref()?.directory.as_ref())
            .map(|directory| directory.path.trim_end_matches('/'))
            .filter(|path| !path.is_empty())
        {
            self.load_remote_dir(profile, format!("{path}/.."), cx);
        }
    }

    pub(super) fn locate_terminal_directory(&mut self, profile: usize, cx: &mut Context<Self>) {
        if !matches!(self.page, Page::Terminal | Page::Editor) {
            return;
        }
        let Some(session) = self
            .active_session_mut()
            .filter(|session| session.profile == profile)
        else {
            return;
        };
        let Some(remote) = session.remote.as_mut().filter(|remote| {
            matches!(remote.status, Status::Connected) && !remote.locating_directory
        }) else {
            return;
        };
        let request = match session
            .terminal
            .update(cx, |view, cx| view.request_working_directory(cx))
        {
            Ok(request) => request,
            Err(error) => {
                remote.file_action_error = Some(error.into());
                cx.notify();
                return;
            }
        };
        let live = remote.live.clone();
        let requested_path = remote.requested_path.clone();
        let editor_value = remote.path_field.read(cx).value().to_owned();
        remote.locating_directory = true;
        remote.file_action_error = None;
        cx.spawn(async move |view, cx| {
            let result = request.await.unwrap_or(Err("终端已关闭，目录查询已结束"));
            let _ = view.update(cx, |view, cx| {
                let Some(remote) = view
                    .sessions
                    .iter_mut()
                    .find(|session| session.profile == profile)
                    .and_then(|session| session.remote.as_mut())
                    .filter(|remote| Arc::ptr_eq(&remote.live, &live))
                else {
                    return;
                };
                remote.locating_directory = false;
                match result {
                    Ok(path)
                        if remote.requested_path == requested_path
                            && remote.directory_request.is_none()
                            && !remote.path_field.read(cx).is_composing()
                            && remote.path_field.read(cx).value() == editor_value =>
                    {
                        view.load_remote_dir(profile, path, cx);
                    }
                    Ok(_) => {}
                    Err(error) => remote.file_action_error = Some(error.into()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn open_remote_file_menu(
        &mut self,
        profile: usize,
        index: Option<usize>,
        position: Point<Pixels>,
        keyboard: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.dialog.is_some() || !matches!(self.page, Page::Terminal | Page::Editor) {
            return;
        }
        let Some(remote) = self
            .active_session_mut()
            .filter(|session| session.profile == profile)
            .and_then(|session| session.remote.as_mut())
        else {
            return;
        };
        let Some(directory) = remote.ready_directory() else {
            return;
        };
        let entry = index
            .and_then(|index| directory.entries.get(index))
            .cloned();
        let target = UploadTarget {
            profile,
            live: remote.live.clone(),
            directory: directory.path.clone(),
        };
        if entry.is_some() {
            remote.selection = index;
        }
        self.file_focus.focus(window);
        self.menu = None;
        self.file_menu = Some(FileMenu {
            target,
            entry,
            position,
            cursor: keyboard.then_some(0),
        });
        cx.notify();
    }

    pub(super) fn finish_remote_delete(
        &mut self,
        delete: &RemoteDelete,
        result: Result<(), String>,
        cx: &mut Context<Self>,
    ) {
        let Some(remote) = self
            .sessions
            .iter_mut()
            .find(|session| session.profile == delete.target.profile)
            .and_then(|session| session.remote.as_mut())
            .filter(|remote| Arc::ptr_eq(&remote.live, &delete.target.live))
        else {
            return;
        };
        let Some(directory) = remote
            .directory
            .as_mut()
            .filter(|dir| dir.path == delete.target.directory)
        else {
            return;
        };
        let mut reload = false;
        match result {
            Ok(()) => {
                remote.file_action_error = None;
                if let Some(index) = directory
                    .entries
                    .iter()
                    .position(|entry| entry.path == delete.entry.path)
                {
                    directory.entries.remove(index);
                    remote.selection =
                        remote
                            .selection
                            .and_then(|selected| match selected.cmp(&index) {
                                std::cmp::Ordering::Less => Some(selected),
                                std::cmp::Ordering::Equal => None,
                                std::cmp::Ordering::Greater => Some(selected - 1),
                            });
                }
                if remote.requested_path == delete.target.directory
                    && remote.path_field.read(cx).value() == delete.target.directory
                    && let Some(request) = remote.directory_request.take()
                {
                    request.abort();
                    reload = true;
                }
            }
            Err(error) => remote.file_action_error = Some(format!("删除未完成 · {error}")),
        }
        if reload {
            self.load_remote_dir(delete.target.profile, delete.target.directory.clone(), cx);
        }
        cx.notify();
    }

    pub(super) fn handle_remote_file_key(
        &mut self,
        profile: usize,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.handle_file_menu_key(event, window, cx) {
            cx.stop_propagation();
            return;
        }
        let Some(remote) = self
            .active_session()
            .filter(|session| session.profile == profile)
            .and_then(|session| session.remote.as_ref())
        else {
            return;
        };
        let field = remote.path_field.read(cx);
        let editing = field.is_focused(window);
        if editing && field.is_composing() {
            return;
        }
        let key = event.keystroke.key.as_str();
        if !editing && (key == "menu" || key == "f10" && event.keystroke.modifiers.shift) {
            let selected = remote.selection;
            let width = self.panel_widths(window.viewport_size().width).1;
            let anchor = point(
                window.viewport_size().width - width,
                space::TITLEBAR_HEIGHT + space::HEADER_HEIGHT,
            );
            self.open_remote_file_menu(profile, selected, anchor, true, window, cx);
            cx.stop_propagation();
            return;
        }
        match (key, editing) {
            ("enter", true) => self.load_remote_dir(profile, field.value().to_owned(), cx),
            ("escape", true) => {
                let path = if remote.directory_request.is_some() {
                    &remote.requested_path
                } else {
                    remote
                        .directory
                        .as_ref()
                        .map_or("", |directory| directory.path.as_str())
                };
                remote
                    .path_field
                    .update(cx, |field, cx| field.set_value(path, cx));
                self.file_focus.focus(window);
            }
            ("enter", false) => self.open_remote_entry(profile, cx),
            ("backspace", false) => self.go_remote_parent(profile, cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    pub(super) fn login_dialog(&self, profile: usize, cx: &Context<Self>) -> Div {
        let profile_display = &self.profiles[profile];
        let temporary = profile_display.temporary;
        let remote = profile_display.remote.as_ref().unwrap();
        let password_auth = remote.auth_method == "password";
        let saved = self.login_credential == LoginCredential::Saved;
        let checking = self.login_pending.is_some();
        let form = div()
            .px(space::LG)
            .py(space::MD)
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
                            .size(space::ROW_HEIGHT)
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(space::CONTROL_RADIUS)
                            .bg(rgb(color::SELECTED))
                            .child(
                                icon(Icon::Connections, color::ACCENT)
                                    .size(space::COMPACT_ICON_SIZE),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .child(
                                div()
                                    .truncate()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(profile_display.title.clone()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(text::DETAIL)
                                    .text_color(rgb(color::MUTED))
                                    .child(profile_display.address.clone()),
                            ),
                    ),
            )
            .when(temporary && remote.username.is_empty(), |form| {
                form.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(space::XS)
                        .child(
                            div()
                                .text_size(text::DETAIL)
                                .text_color(rgb(color::MUTED))
                                .child("SSH 用户名"),
                        )
                        .child(self.bastion_username.clone()),
                )
            });
        let form = if saved || checking {
            form.child(
                div()
                    .h(space::TOOLBAR_HEIGHT)
                    .px(space::SM)
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(space::SM)
                    .border_1()
                    .border_color(rgb(color::LINE))
                    .rounded(space::CONTROL_RADIUS)
                    .child(if checking {
                        "正在读取凭据…"
                    } else if temporary {
                        "使用链接附带的凭据"
                    } else if password_auth {
                        "已保存密码"
                    } else {
                        "已保存私钥口令"
                    })
                    .when(saved && !checking, |row| {
                        row.child(self.login_credential_switch(temporary, cx))
                    }),
            )
        } else {
            form.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(space::XS)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(space::SM)
                            .text_size(text::DETAIL)
                            .text_color(rgb(color::MUTED))
                            .child(if password_auth {
                                "SSH 密码"
                            } else {
                                "私钥口令（未加密可留空）"
                            })
                            .when(
                                self.login_credential == LoginCredential::ReplaceSaved,
                                |row| row.child(self.login_credential_switch(temporary, cx)),
                            ),
                    )
                    .child(self.password.clone())
                    .when(credentials::AVAILABLE && !temporary, |field| {
                        field.child(
                            div().child(
                                dialog_tab(
                                    control(
                                        "remember-ssh-password",
                                        match (password_auth, self.remember_password) {
                                            (true, false) => "记住密码",
                                            (true, true) => "✓ 记住密码",
                                            (false, false) => "记住私钥口令",
                                            (false, true) => "✓ 记住私钥口令",
                                        },
                                        self.remember_password,
                                    )
                                    .text_size(text::DETAIL),
                                    3,
                                )
                                .on_click(cx.listener(
                                    |view, _, _, cx| {
                                        view.remember_password = !view.remember_password;
                                        cx.notify();
                                    },
                                )),
                            ),
                        )
                    }),
            )
        };
        form.when_some(self.ssh_error.clone(), |form, error| {
            form.child(
                div()
                    .text_size(text::DETAIL)
                    .text_color(rgb(color::ACCENT_TEXT))
                    .child(error),
            )
        })
    }

    fn login_credential_switch(&self, temporary: bool, cx: &Context<Self>) -> Stateful<Div> {
        let saved = self.login_credential == LoginCredential::Saved;
        dialog_tab(
            control(
                "toggle-saved-credential",
                if saved {
                    "手动输入"
                } else if temporary {
                    "使用链接附带凭据"
                } else {
                    "使用已保存凭据"
                },
                false,
            )
            .text_size(text::DETAIL)
            .text_color(rgb(color::ACCENT_TEXT)),
            1,
        )
        .on_click(cx.listener(|view, _, window, cx| {
            view.ssh_error = None;
            if view.login_credential == LoginCredential::Saved {
                view.login_credential = LoginCredential::ReplaceSaved;
                view.password.read(cx).focus(window);
            } else {
                view.login_credential = LoginCredential::Saved;
                view.password
                    .update(cx, |field, cx| field.set_value("", cx));
                view.dialog_focus.focus(window);
            }
            cx.notify();
        }))
    }

    pub(super) fn login_footer(
        &self,
        profile: usize,
        origin: LoginOrigin,
        cx: &Context<Self>,
    ) -> Div {
        let saved = self.login_credential == LoginCredential::Saved;
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap(space::SM)
            .px(space::LG)
            .py(space::SM)
            .border_t_1()
            .border_color(rgb(color::LINE))
            .bg(rgb(color::TOOLBAR))
            .when(self.can_edit_profile(profile), |row| {
                row.child(
                    dialog_tab(control("edit-login-profile", "编辑连接", false), 4).on_click(
                        cx.listener(move |view, _, window, cx| {
                            view.open_dialog(Dialog::Profile(Some(profile)), window, cx)
                        }),
                    ),
                )
            })
            .child(div().flex_1())
            .child(
                dialog_tab(control("cancel-ssh-login", "取消", false), 5)
                    .h(space::ROW_HEIGHT)
                    .px(space::MD)
                    .on_click(cx.listener(|view, _, window, cx| view.close_dialog(window, cx))),
            )
            .when(self.login_pending.is_none(), |row| {
                row.child(
                    primary_control("connect-ssh", "连接")
                        .tab_index(6)
                        .h(space::ROW_HEIGHT)
                        .px(space::LG)
                        .font_weight(FontWeight::SEMIBOLD)
                        .focus(|style| style.border_1().border_color(rgb(color::TEXT)))
                        .on_click(cx.listener(move |view, _, window, cx| {
                            view.start_remote(profile, origin, saved, window, cx)
                        })),
                )
            })
    }

    pub(super) fn host_dialog(&self, profile: usize, cx: &Context<Self>) -> Div {
        let content = div().p(space::LG).flex().flex_col().gap(space::SM);
        let Some(Status::Verify(key)) = self
            .sessions
            .iter()
            .find(|session| session.profile == profile)
            .and_then(|session| session.remote.as_ref().map(|remote| &remote.status))
        else {
            return content;
        };
        content
            .child("首次连接此服务器。请与管理员核对指纹。")
            .child(format!("{}:{} · {}", key.hostname, key.port, key.algorithm))
            .child(
                div()
                    .font_family("Cascadia Mono")
                    .text_size(text::DETAIL)
                    .child(key.fingerprint.clone()),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(space::SM)
                    .child(
                        control("reject-ssh-key", "拒绝", false).on_click(cx.listener(
                            move |view, _, window, cx| view.answer_host(profile, false, window, cx),
                        )),
                    )
                    .child(
                        control("trust-ssh-key", "信任并连接", true).on_click(cx.listener(
                            move |view, _, window, cx| view.answer_host(profile, true, window, cx),
                        )),
                    ),
            )
    }

    pub(super) fn remote_files(
        &self,
        session: &Session,
        remote: &RemoteSession,
        width: Pixels,
        cx: &Context<Self>,
    ) -> Div {
        let profile = session.profile;
        let narrow = width < px(290.);
        let size_width = if narrow { px(54.) } else { px(68.) };
        let modified_width = if narrow { px(76.) } else { px(114.) };
        let date_format = if narrow {
            "%m-%d %H:%M"
        } else {
            "%Y-%m-%d %H:%M"
        };
        let current = remote
            .directory
            .as_ref()
            .map(|directory| directory.path.as_str());
        let connected = matches!(remote.status, Status::Connected);
        let loading = remote.directory_request.is_some();
        let ready = connected && !loading && remote.directory_error.is_none();
        let upload = current.filter(|_| ready).map(|directory| UploadTarget {
            profile,
            live: remote.live.clone(),
            directory: directory.to_owned(),
        });
        let selected_download = remote
            .directory
            .as_ref()
            .and_then(|directory| directory.entries.get(remote.selection?))
            .filter(|entry| ready && !entry.is_directory)
            .map(|entry| DownloadTarget {
                live: remote.live.clone(),
                path: entry.path.clone(),
                name: entry.name.clone(),
            });
        let retry = remote.requested_path.clone();
        let path_field = remote.path_field.clone();
        let (hidden_icon, hidden_label) = if self.show_hidden_files {
            (Icon::Eye, "隐藏点号文件")
        } else {
            (Icon::EyeOff, "显示隐藏文件")
        };
        let toolbar_button = |id: &'static str, glyph, label, selected| {
            icon_control_with_size(id, glyph, label, selected, space::COMPACT_ICON_SIZE)
                .size(space::CONTROL_HEIGHT)
        };
        let list = div().flex().flex_col().flex_1().min_h(px(0.)).when_some(
            upload.clone(),
            |list, target| {
                list.drag_over::<ExternalPaths>(|style, _, _, _| style.bg(rgb(color::TOOLBAR)))
                    .on_drop(cx.listener(move |view, paths: &ExternalPaths, _, cx| {
                        view.drop_upload(target.clone(), paths, cx);
                    }))
            },
        );
        let message = if loading {
            Some("正在读取远程目录…".into())
        } else if !connected {
            Some(remote.message.clone())
        } else {
            remote
                .directory_error
                .as_ref()
                .map(|error| format!("无法读取远程目录 · {error}"))
        };
        let entries = remote
            .directory
            .iter()
            .flat_map(|dir| dir.entries.iter().enumerate())
            .filter(|(_, entry)| self.show_hidden_files || !entry.name.starts_with('.'));
        let count = entries.clone().count();
        list.child(
            div()
                .flex_none()
                .px(space::MD)
                .pt(space::SM)
                .pb(space::XS)
                .child(path_field.clone()),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap(space::XS)
                .px(space::MD)
                .pb(space::SM)
                .when(connected, |row| {
                    row.when(current.is_some_and(|path| path != "/"), |row| {
                        row.child(
                            toolbar_button("sftp-parent", Icon::ArrowUp, "上一级", false).on_click(
                                cx.listener(move |view, _, _, cx| {
                                    view.go_remote_parent(profile, cx)
                                }),
                            ),
                        )
                    })
                    .child(
                        toolbar_button("sftp-home", Icon::House, "主目录", false).on_click(
                            cx.listener(move |view, _, _, cx| {
                                view.load_remote_dir(profile, String::new(), cx)
                            }),
                        ),
                    )
                    .child(
                        toolbar_button(
                            "sftp-go-to",
                            Icon::FolderSearch,
                            "编辑路径 · Enter 跳转",
                            false,
                        )
                        .on_click(move |_, window, cx| {
                            path_field.update(cx, |field, cx| {
                                field.focus(window);
                                field.select_all();
                                cx.notify();
                            });
                        }),
                    )
                    .child(
                        toolbar_button("sftp-refresh", Icon::Refresh, "刷新目录", false).on_click(
                            cx.listener(move |view, _, _, cx| {
                                view.load_remote_dir(profile, retry.clone(), cx)
                            }),
                        ),
                    )
                    .child(
                        toolbar_button(
                            "sftp-terminal-directory",
                            Icon::Terminal,
                            if remote.locating_directory {
                                "正在定位终端目录…"
                            } else {
                                "定位到终端当前目录"
                            },
                            remote.locating_directory,
                        )
                        .when(!remote.locating_directory, |button| {
                            button.on_click(cx.listener(move |view, _, _, cx| {
                                view.locate_terminal_directory(profile, cx)
                            }))
                        }),
                    )
                })
                .child(
                    toolbar_button(
                        "sftp-hidden",
                        hidden_icon,
                        hidden_label,
                        self.show_hidden_files,
                    )
                    .on_click(cx.listener(|view, _, _, cx| view.toggle_hidden_files(cx))),
                )
                .child(
                    toolbar_button("sftp-upload", Icon::Upload, "上传文件", false)
                        .when(upload.is_none(), |button| button.opacity(0.45))
                        .when_some(upload.clone(), |button, target| {
                            button.on_click(cx.listener(move |view, _, window, cx| {
                                view.choose_upload(target.clone(), window, cx)
                            }))
                        }),
                )
                .child(
                    toolbar_button("sftp-download", Icon::Download, "下载选中的文件", false)
                        .when(selected_download.is_none(), |button| button.opacity(0.45))
                        .when_some(selected_download, |button, target| {
                            button.on_click(cx.listener(move |view, _, window, cx| {
                                view.choose_download(target.clone(), window, cx)
                            }))
                        }),
                )
                .child(self.transfer_queue_control(cx)),
        )
        .when_some(message, |list, message| {
            list.child(
                div()
                    .px(space::MD)
                    .pb(space::SM)
                    .text_size(text::DETAIL)
                    .text_color(rgb(color::MUTED))
                    .child(message),
            )
        })
        .when_some(remote.file_action_error.clone(), |list, error| {
            list.child(
                div()
                    .px(space::MD)
                    .pb(space::SM)
                    .text_size(text::DETAIL)
                    .text_color(rgb(color::DANGER))
                    .child(error),
            )
        })
        .child(
            div()
                .px(space::LG)
                .py(space::XS)
                .flex()
                .items_center()
                .gap(space::SM)
                .text_size(text::DETAIL)
                .child(
                    self.file_sort_header("sftp-sort-name", "名称", FileColumn::Name, cx)
                        .flex_1()
                        .min_w(px(0.))
                        .justify_start(),
                )
                .child(
                    self.file_sort_header("sftp-sort-size", "大小", FileColumn::Size, cx)
                        .w(size_width)
                        .justify_end(),
                )
                .child(
                    self.file_sort_header(
                        "sftp-sort-modified",
                        "更新时间",
                        FileColumn::Modified,
                        cx,
                    )
                    .w(modified_width)
                    .justify_end(),
                ),
        )
        .child(
            div()
                .id("sftp-entries")
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scroll()
                .px(space::SM)
                .when(ready, |list| {
                    list.on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |view, event: &gpui::MouseDownEvent, window, cx| {
                            view.open_remote_file_menu(
                                profile,
                                None,
                                event.position,
                                false,
                                window,
                                cx,
                            );
                            cx.stop_propagation();
                        }),
                    )
                })
                .when(count == 0 && remote.directory.is_some(), |list| {
                    list.child(
                        div()
                            .p(space::SM)
                            .text_color(rgb(color::MUTED))
                            .child("没有可显示的文件"),
                    )
                })
                .children(entries.map(|(index, entry)| {
                    let size = if entry.is_directory || !entry.has_size {
                        "—".into()
                    } else {
                        file_size(entry.size)
                    };
                    let modified = entry
                        .modified_at
                        .and_then(|seconds| DateTime::<Utc>::from_timestamp(i64::from(seconds), 0))
                        .map_or_else(
                            || "—".into(),
                            |time| time.with_timezone(&Local).format(date_format).to_string(),
                        );
                    let selected = remote.selection == Some(index);
                    div()
                        .id(ElementId::named_usize("sftp-entry", index))
                        .h(if self.compact_files {
                            space::ROW_HEIGHT
                        } else {
                            space::HEADER_HEIGHT
                        })
                        .px(space::SM)
                        .flex()
                        .items_center()
                        .gap(space::SM)
                        .rounded(space::CONTROL_RADIUS)
                        .cursor_pointer()
                        .bg(rgb(if selected {
                            color::SELECTED
                        } else {
                            color::SURFACE
                        }))
                        .hover(move |style| {
                            style.bg(rgb(if selected {
                                color::SELECTED_HOVER
                            } else {
                                color::HOVER
                            }))
                        })
                        .child(file_icon(&entry.name, entry.is_directory))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .truncate()
                                .child(entry.name.clone()),
                        )
                        .when(entry.is_symlink && !narrow, |row| {
                            row.child(
                                div()
                                    .text_size(text::DETAIL)
                                    .text_color(rgb(color::MUTED))
                                    .child("链接"),
                            )
                        })
                        .child(
                            div()
                                .flex_none()
                                .w(size_width)
                                .text_right()
                                .text_size(text::DETAIL)
                                .text_color(rgb(color::MUTED))
                                .child(size),
                        )
                        .child(
                            div()
                                .flex_none()
                                .w(modified_width)
                                .text_right()
                                .text_size(text::DETAIL)
                                .text_color(rgb(color::MUTED))
                                .child(modified),
                        )
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |view, event: &gpui::MouseDownEvent, window, cx| {
                                view.file_focus.focus(window);
                                if let Some(remote) = view
                                    .active_session_mut()
                                    .filter(|session| session.profile == profile)
                                    .and_then(|session| session.remote.as_mut())
                                {
                                    remote.selection = Some(index);
                                }
                                if event.click_count == 2 {
                                    view.open_remote_entry(profile, cx);
                                }
                                cx.notify();
                            }),
                        )
                        .when(ready, |row| {
                            row.on_mouse_down(
                                MouseButton::Right,
                                cx.listener(
                                    move |view, event: &gpui::MouseDownEvent, window, cx| {
                                        view.open_remote_file_menu(
                                            profile,
                                            Some(index),
                                            event.position,
                                            false,
                                            window,
                                            cx,
                                        );
                                        cx.stop_propagation();
                                    },
                                ),
                            )
                        })
                        .when(entry.is_directory && ready, |row| {
                            let target = UploadTarget {
                                profile,
                                live: remote.live.clone(),
                                directory: entry.path.clone(),
                            };
                            row.drag_over::<ExternalPaths>(|style, _, _, _| {
                                style.bg(rgb(color::SELECTED))
                            })
                            .on_drop(cx.listener(
                                move |view, paths: &ExternalPaths, _, cx| {
                                    view.drop_upload(target.clone(), paths, cx);
                                },
                            ))
                        })
                        .when(!entry.is_directory && ready, |row| {
                            self.drag_download_row(
                                row,
                                DownloadTarget {
                                    live: remote.live.clone(),
                                    path: entry.path.clone(),
                                    name: entry.name.clone(),
                                },
                                cx,
                            )
                        })
                })),
        )
        .when(remote.directory.is_some(), |list| {
            list.child(panel_footer(format!("{count} 项")))
        })
    }
}

pub(super) fn file_size(bytes: u64) -> String {
    match bytes {
        0..=1023 => format!("{bytes} B"),
        1024..=1_048_575 => format!("{:.1} KB", bytes as f64 / 1024.),
        1_048_576..=1_073_741_823 => format!("{:.1} MB", bytes as f64 / 1_048_576.),
        _ => format!("{:.1} GB", bytes as f64 / 1_073_741_824.),
    }
}
