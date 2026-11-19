use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use gpui::{Context, Div, ElementId, Entity, MouseButton, Window, div, prelude::*, px};
use orbitshell_ssh::{
    remote_editor::{MAX_BYTES, OpenedFile, RemoteText, SaveResult, SnapshotInfo},
    session::LiveSession,
    sftp::RemoteEntry,
};

use super::{
    Dialog, Page, Workbench, control, icon_control, ssh::file_size, transfers::UploadTarget,
};
use crate::{
    editor_input::{EditorInput, EditorToolRequest},
    icons::Icon,
    theme::{color, danger_control, primary_control, rgb, space, text},
};

#[path = "workbench_editor_tools.rs"]
mod tools;
use tools::EditorSearch;

#[derive(Default)]
pub(super) struct RemoteEditors {
    tabs: Vec<EditorTab>,
    active: Option<String>,
    opening: Vec<(UploadTarget, String)>,
    confirmation: Option<(String, Confirm)>,
}

pub(super) struct EditorTab {
    id: String,
    name: String,
    path: String,
    requested_path: String,
    live: Arc<LiveSession>,
    base: RemoteText,
    input: Entity<EditorInput>,
    status: Option<String>,
    busy: bool,
    restoring: bool,
    snapshots: Option<Vec<SnapshotInfo>>,
    comparison: Option<Comparison>,
    search: Option<EditorSearch>,
    format_menu: Option<usize>,
    format_focus: gpui::FocusHandle,
}

pub(super) struct ExitDraft {
    pub(super) id: String,
    pub(super) path: String,
    pub(super) can_save: bool,
}

struct Comparison {
    document: RemoteText,
    input: Entity<EditorInput>,
    kind: ComparisonKind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ComparisonKind {
    Remote,
    Snapshot,
}

#[derive(Clone, Copy)]
enum Confirm {
    Close,
    Reload,
    UseRemote,
    Overwrite,
    RestoreSnapshot,
}

enum Operation {
    Save { close: bool, compared_version: bool },
    Reload,
    Snapshots,
    Snapshot(String),
}

enum Outcome {
    Save {
        result: SaveResult,
        close: bool,
    },
    Reload {
        document: RemoteText,
        previous_content: String,
    },
    Snapshots(Vec<SnapshotInfo>),
    Snapshot(RemoteText),
}

impl RemoteEditors {
    pub(super) fn active(&self) -> Option<&EditorTab> {
        self.tabs
            .iter()
            .find(|tab| Some(&tab.id) == self.active.as_ref())
    }

    pub(super) fn has_tabs(&self) -> bool {
        !self.tabs.is_empty()
    }
    pub(super) fn cancel_confirmation(&mut self) {
        self.confirmation = None;
    }

    pub(super) fn confirmation_title(&self) -> &'static str {
        match self.confirmation.as_ref().map(|(_, action)| action) {
            Some(Confirm::Overwrite) => "确认保存到远程",
            Some(Confirm::RestoreSnapshot) => "恢复快照",
            _ => "未保存的修改",
        }
    }
}

impl EditorTab {
    pub(super) fn focus(&self, window: &mut Window, cx: &Context<Workbench>) {
        self.input.read(cx).focus(window);
    }

    fn dirty(&self, cx: &Context<Workbench>) -> bool {
        self.input.read(cx).text() != self.base.content
    }

    fn connected(&self) -> bool {
        !self.live.cancel.is_cancelled()
    }

    fn has_conflict(&self) -> bool {
        self.comparison
            .as_ref()
            .is_some_and(|item| item.kind == ComparisonKind::Remote)
    }

    fn apply_base(&mut self, document: RemoteText, cx: &mut Context<Workbench>) {
        self.input
            .update(cx, |input, cx| input.reset(document.content.clone(), cx));
        self.base = document;
        self.restoring = false;
        self.comparison = None;
    }

    fn compare(&mut self, document: RemoteText, kind: ComparisonKind, cx: &mut Context<Workbench>) {
        let input = cx.new(|cx| {
            EditorInput::for_file(document.content.clone(), true, &self.name, &self.path, cx)
        });
        self.comparison = Some(Comparison {
            document,
            input,
            kind,
        });
    }
}

pub(super) fn can_edit_remote(entry: &RemoteEntry) -> bool {
    if entry.is_directory || entry.size > MAX_BYTES as u64 {
        return false;
    }
    let extension = entry
        .name
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase());
    !matches!(
        extension.as_deref(),
        Some(
            "7z" | "zip"
                | "gz"
                | "tgz"
                | "bz2"
                | "xz"
                | "tar"
                | "rar"
                | "jar"
                | "war"
                | "class"
                | "exe"
                | "dll"
                | "so"
                | "dylib"
                | "bin"
                | "dat"
                | "db"
                | "sqlite"
                | "sqlite3"
                | "png"
                | "jpg"
                | "jpeg"
                | "gif"
                | "webp"
                | "ico"
                | "bmp"
                | "avif"
                | "heic"
                | "pdf"
                | "doc"
                | "docx"
                | "xls"
                | "xlsx"
                | "ppt"
                | "pptx"
                | "mp3"
                | "mp4"
                | "mkv"
                | "mov"
                | "avi"
                | "wav"
                | "ogg"
                | "flac"
                | "woff"
                | "woff2"
                | "ttf"
                | "otf"
                | "p12"
                | "pfx"
        )
    )
}

impl Workbench {
    pub(super) fn editors_busy(&self) -> bool {
        self.editors.tabs.iter().any(|tab| tab.busy)
    }

    pub(super) fn exit_drafts(&self, cx: &Context<Self>) -> Vec<ExitDraft> {
        self.editors
            .tabs
            .iter()
            .filter_map(|tab| {
                let composing = tab.input.read(cx).is_composing();
                (composing || tab.dirty(cx)).then(|| ExitDraft {
                    id: tab.id.clone(),
                    path: tab.path.clone(),
                    can_save: !composing && tab.connected(),
                })
            })
            .collect()
    }

    pub(super) fn save_exit_editor(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.run_editor(
            id,
            Operation::Save {
                close: false,
                compared_version: false,
            },
            window,
            cx,
        );
        self.editors.tabs.iter().any(|tab| tab.id == id && tab.busy)
    }

    pub(super) fn open_remote_editor(
        &mut self,
        target: UploadTarget,
        entry: RemoteEntry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.file_target_is_current(&target) || !can_edit_remote(&entry) {
            return;
        }
        if let Some(tab) = self.editors.tabs.iter().find(|tab| {
            Arc::ptr_eq(&tab.live, &target.live)
                && (tab.requested_path == entry.path || tab.path == entry.path)
        }) {
            let id = tab.id.clone();
            self.show_editor(&id, window, cx);
            return;
        }
        if self
            .editors
            .opening
            .iter()
            .any(|(pending, path)| Arc::ptr_eq(&pending.live, &target.live) && path == &entry.path)
        {
            return;
        }
        let Some(backend) = &self.backend else {
            return;
        };
        let store = backend.editor();
        let job = backend.open_remote_text(target.live.clone(), entry.path.clone());
        self.editors
            .opening
            .push((target.clone(), entry.path.clone()));
        self.set_editor_open_status(&target, Some(format!("正在打开 {}…", entry.name)));
        cx.spawn_in(window, async move |view, cx| {
            let result = job
                .await
                .unwrap_or_else(|_| Err("文件读取任务未完成".into()));
            let mut opened_id = result.as_ref().ok().map(|opened| opened.id.clone());
            let _ = view.update_in(cx, |view, window, cx| {
                view.editors.opening.retain(|(pending, path)| {
                    !Arc::ptr_eq(&pending.live, &target.live) || path != &entry.path
                });
                match result {
                    Ok(opened) => {
                        view.set_editor_open_status(&target, None);
                        if !target.live.cancel.is_cancelled() {
                            let id = view.add_remote_editor(&target, entry, opened, window, cx);
                            if opened_id.as_ref() == Some(&id) {
                                opened_id = None;
                            }
                            if view.dialog.is_none() && view.file_target_is_current(&target) {
                                view.show_editor(&id, window, cx);
                            }
                        }
                    }
                    Err(error) => view.set_editor_open_status(&target, Some(error)),
                }
                cx.notify();
            });
            if let Some(id) = opened_id {
                let _ = store.close(&id);
            }
        })
        .detach();
        cx.notify();
    }

    fn set_editor_open_status(&mut self, target: &UploadTarget, status: Option<String>) {
        if let Some(remote) = self
            .sessions
            .iter_mut()
            .find(|session| session.profile == target.profile)
            .and_then(|session| session.remote.as_mut())
            .filter(|remote| Arc::ptr_eq(&remote.live, &target.live))
        {
            remote.file_action_error = status;
        }
    }

    fn add_remote_editor(
        &mut self,
        target: &UploadTarget,
        entry: RemoteEntry,
        opened: OpenedFile,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> String {
        if let Some(existing) = self
            .editors
            .tabs
            .iter()
            .find(|tab| Arc::ptr_eq(&tab.live, &target.live) && tab.path == opened.path)
        {
            return existing.id.clone();
        }
        let input = cx.new(|cx| {
            EditorInput::for_file(
                opened.document.content.clone(),
                false,
                &entry.name,
                &opened.path,
                cx,
            )
        });
        cx.observe(&input, |_, _, cx| cx.notify()).detach();
        let id = opened.id.clone();
        let editor_id = id.clone();
        cx.subscribe_in(
            &input,
            window,
            move |view, _, request: &EditorToolRequest, window, cx| {
                if view.editors.active.as_deref() == Some(editor_id.as_str())
                    && view.page == Page::Editor
                {
                    match request {
                        EditorToolRequest::Search => view.open_editor_search(window, cx),
                        EditorToolRequest::Format(mode) => {
                            view.format_active_editor(*mode, window, cx)
                        }
                    }
                }
            },
        )
        .detach();
        self.editors.tabs.push(EditorTab {
            id: opened.id,
            name: entry.name,
            path: opened.path,
            requested_path: entry.path,
            live: target.live.clone(),
            base: opened.document,
            input,
            status: opened.warning,
            busy: false,
            restoring: false,
            snapshots: None,
            comparison: None,
            search: None,
            format_menu: None,
            format_focus: cx.focus_handle(),
        });
        id
    }

    pub(super) fn show_editor(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.editors.tabs.iter().find(|tab| tab.id == id) else {
            return;
        };
        if let Some(session) = self.sessions.iter().find(|session| {
            session.show_tab
                && session
                    .remote
                    .as_ref()
                    .is_some_and(|remote| Arc::ptr_eq(&remote.live, &tab.live))
        }) {
            self.active = Some(session.profile);
        }
        self.page = Page::Editor;
        self.editors.active = Some(id.to_owned());
        self.focus_active(window, cx);
        cx.notify();
    }

    pub(super) fn save_active_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.editors.active.clone() {
            if self.editors.active().is_some_and(|tab| tab.has_conflict()) {
                self.request_editor_action(&id, Confirm::Overwrite, window, cx);
            } else {
                self.run_editor(
                    &id,
                    Operation::Save {
                        close: false,
                        compared_version: false,
                    },
                    window,
                    cx,
                );
            }
        }
    }

    pub(super) fn close_active_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.editors.active.clone() {
            self.request_editor_action(&id, Confirm::Close, window, cx);
        }
    }

    fn remove_editor(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.editors.tabs.iter().position(|tab| tab.id == id) else {
            return;
        };
        self.editors.tabs.remove(index);
        if let Some(backend) = &self.backend {
            let _ = backend.editor().close(id);
        }
        if self.editors.active.as_deref() == Some(id) {
            self.editors.active = self
                .editors
                .tabs
                .get(index.min(self.editors.tabs.len().saturating_sub(1)))
                .map(|tab| tab.id.clone());
            if self.page == Page::Editor {
                if let Some(id) = self.editors.active.clone() {
                    self.show_editor(&id, window, cx);
                } else {
                    self.page = Page::Terminal;
                    self.focus_active(window, cx);
                }
            }
        }
        cx.notify();
    }

    fn request_editor_action(
        &mut self,
        id: &str,
        action: Confirm,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self
            .editors
            .tabs
            .iter()
            .find(|tab| tab.id == id && !tab.busy)
        else {
            return;
        };
        if tab.dirty(cx) || matches!(action, Confirm::RestoreSnapshot | Confirm::Overwrite) {
            self.editors.confirmation = Some((id.into(), action));
            self.open_dialog(Dialog::EditorConfirm, window, cx);
        } else {
            self.apply_editor_action(id, action, window, cx);
        }
    }

    fn apply_editor_action(
        &mut self,
        id: &str,
        action: Confirm,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            Confirm::Close => self.remove_editor(id, window, cx),
            Confirm::Reload => self.run_editor(id, Operation::Reload, window, cx),
            Confirm::Overwrite => self.run_editor(
                id,
                Operation::Save {
                    close: false,
                    compared_version: true,
                },
                window,
                cx,
            ),
            Confirm::UseRemote | Confirm::RestoreSnapshot => {
                let Some(tab) = self
                    .editors
                    .tabs
                    .iter_mut()
                    .find(|tab| tab.id == id && !tab.busy)
                else {
                    return;
                };
                let Some(comparison) = tab.comparison.take() else {
                    return;
                };
                if matches!(action, Confirm::UseRemote) {
                    tab.apply_base(comparison.document, cx);
                    tab.status = Some("已载入远程版本".into());
                } else {
                    tab.input.update(cx, |input, cx| {
                        input.replace_all(&comparison.document.content, cx)
                    });
                    tab.restoring = true;
                    tab.status = Some("已将快照载入编辑器，保存后写入远程文件".into());
                }
                tab.focus(window, cx);
                cx.notify();
            }
        }
    }

    fn run_editor(
        &mut self,
        id: &str,
        operation: Operation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(backend) = &self.backend else {
            return;
        };
        let Some(tab) = self
            .editors
            .tabs
            .iter_mut()
            .find(|tab| tab.id == id && !tab.busy)
        else {
            return;
        };
        let remote = matches!(operation, Operation::Save { .. } | Operation::Reload);
        if remote && !tab.connected() {
            return;
        }
        if matches!(operation, Operation::Save { .. })
            && (!tab.dirty(cx) || tab.input.read(cx).is_composing())
        {
            return;
        }
        let revision = match operation {
            Operation::Save {
                compared_version: true,
                ..
            } => {
                let Some(compared) = tab
                    .comparison
                    .as_ref()
                    .filter(|item| item.kind == ComparisonKind::Remote)
                else {
                    return;
                };
                compared.document.revision.clone()
            }
            _ => tab.base.revision.clone(),
        };
        tab.busy = true;
        tab.status = Some(
            match operation {
                Operation::Save { .. } => "正在保存远程文件…",
                Operation::Reload => "正在读取远程版本…",
                _ => "正在读取本地快照…",
            }
            .into(),
        );
        let store = backend.editor();
        let id = id.to_owned();
        let remote_id = id.clone();
        let content = remote
            .then(|| tab.input.read(cx).text().to_owned())
            .unwrap_or_default();
        let restoring = tab.restoring;
        let job = backend.runtime_handle().spawn(async move {
            match operation {
                Operation::Save { close, .. } => store
                    .save(&remote_id, &content, &revision, restoring)
                    .await
                    .map(|result| Outcome::Save { result, close }),
                Operation::Reload => store
                    .read(&remote_id)
                    .await
                    .map(|document| Outcome::Reload {
                        document,
                        previous_content: content,
                    }),
                Operation::Snapshots => store.snapshots(&remote_id).await.map(Outcome::Snapshots),
                Operation::Snapshot(snapshot) => store
                    .read_snapshot(&remote_id, &snapshot)
                    .await
                    .map(Outcome::Snapshot),
            }
        });
        cx.spawn_in(window, async move |view, cx| {
            let result = job
                .await
                .unwrap_or_else(|_| Err("文件操作任务未完成".into()));
            let _ = view.update_in(cx, |view, window, cx| {
                view.finish_editor(&id, result, window, cx)
            });
        })
        .detach();
        cx.notify();
    }

    fn finish_editor(
        &mut self,
        id: &str,
        result: Result<Outcome, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.editors.tabs.iter_mut().find(|tab| tab.id == id) else {
            return;
        };
        tab.busy = false;
        tab.status = match result {
            Ok(Outcome::Save {
                result: SaveResult::Saved { document, snapshot },
                close,
            }) => {
                tab.base = document;
                tab.restoring = false;
                tab.comparison = None;
                if let Some(snapshots) = &mut tab.snapshots {
                    snapshots.insert(0, snapshot);
                    snapshots.truncate(20);
                }
                if close && !tab.dirty(cx) {
                    self.remove_editor(id, window, cx);
                    return;
                }
                Some(
                    if tab.dirty(cx) {
                        "早先的修改已保存，编辑器里还有新修改"
                    } else {
                        "已保存到远程，保存前的版本已保留为快照"
                    }
                    .into(),
                )
            }
            Ok(Outcome::Save {
                result: SaveResult::Conflict { current },
                ..
            }) => {
                tab.compare(current, ComparisonKind::Remote, cx);
                Some("远程文件已有修改。本地内容已保留，请对照右侧版本。".into())
            }
            Ok(Outcome::Reload {
                document,
                previous_content,
            }) => {
                if tab.input.read(cx).text() != previous_content {
                    tab.compare(document, ComparisonKind::Remote, cx);
                    Some("已读取远程版本，当前编辑内容仍保留".into())
                } else {
                    tab.apply_base(document, cx);
                    Some("已读取远程版本".into())
                }
            }
            Ok(Outcome::Snapshots(snapshots)) => {
                tab.snapshots = Some(snapshots);
                None
            }
            Ok(Outcome::Snapshot(document)) => {
                tab.compare(document, ComparisonKind::Snapshot, cx);
                None
            }
            Err(error) => Some(error),
        };
        self.finish_exit_save(id, window, cx);
        cx.notify();
    }

    pub(super) fn editor_tabs(&self, cx: &Context<Self>) -> Vec<Div> {
        self.editors
            .tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| {
                let id = tab.id.clone();
                let close = id.clone();
                let middle = id.clone();
                let active = self.page == Page::Editor && Some(&id) == self.editors.active.as_ref();
                let label = if tab.dirty(cx) {
                    format!("{}  •", tab.name)
                } else {
                    tab.name.clone()
                };
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .rounded(space::CONTROL_RADIUS)
                    .when(active, |row| row.bg(rgb(color::SELECTED)))
                    .on_mouse_down(
                        MouseButton::Middle,
                        cx.listener(move |view, _, window, cx| {
                            view.request_editor_action(&middle, Confirm::Close, window, cx);
                            cx.stop_propagation();
                        }),
                    )
                    .child(
                        control(
                            ElementId::named_usize("select-editor", index),
                            label,
                            active,
                        )
                        .max_w(px(192.))
                        .overflow_hidden()
                        .on_click(
                            cx.listener(move |view, _, window, cx| {
                                view.show_editor(&id, window, cx)
                            }),
                        ),
                    )
                    .child(
                        icon_control(
                            ElementId::named_usize("close-editor", index),
                            Icon::Close,
                            "关闭文件",
                            active,
                        )
                        .size(space::CONTROL_HEIGHT)
                        .on_click(cx.listener(
                            move |view, _, window, cx| {
                                view.request_editor_action(&close, Confirm::Close, window, cx)
                            },
                        )),
                    )
            })
            .collect()
    }

    pub(super) fn editor_panel(&self, cx: &Context<Self>) -> Div {
        let Some(tab) = self.editors.active() else {
            return div();
        };
        let reload = tab.id.clone();
        let history = tab.id.clone();
        let input = tab.input.read(cx);
        let (line, column) = input.cursor_position();
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(space::SM)
                    .px(space::LG)
                    .h(space::TOOLBAR_HEIGHT)
                    .border_b_1()
                    .border_color(rgb(color::LINE))
                    .child(
                        div()
                            .flex_1()
                            .truncate()
                            .text_size(text::DETAIL)
                            .text_color(rgb(color::MUTED))
                            .child(tab.path.clone()),
                    )
                    .child(self.editor_tool_controls(tab, cx))
                    .when(tab.connected() && !tab.busy, |row| {
                        row.child(control("reload-editor", "重新读取", false).on_click(
                            cx.listener(move |view, _, window, cx| {
                                view.request_editor_action(&reload, Confirm::Reload, window, cx)
                            }),
                        ))
                    })
                    .when(!tab.busy, |row| {
                        row.child(
                            control("editor-snapshots", "快照", tab.snapshots.is_some()).on_click(
                                cx.listener(move |view, _, window, cx| {
                                    if let Some(tab) =
                                        view.editors.tabs.iter_mut().find(|tab| tab.id == history)
                                        && tab.snapshots.take().is_some()
                                    {
                                        cx.notify();
                                    } else {
                                        view.run_editor(&history, Operation::Snapshots, window, cx);
                                    }
                                }),
                            ),
                        )
                    })
                    .child(if tab.connected() && !tab.busy && tab.dirty(cx) {
                        primary_control("save-editor", "保存").on_click(
                            cx.listener(|view, _, window, cx| view.save_active_editor(window, cx)),
                        )
                    } else {
                        div()
                            .id("save-editor")
                            .flex_none()
                            .px(space::SM)
                            .text_color(rgb(color::MUTED))
                            .child("保存")
                    }),
            )
            .when_some(tab.snapshots.as_ref(), |pane, snapshots| {
                pane.child(self.editor_snapshots(tab, snapshots, cx))
            })
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h(px(0.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w(px(0.))
                            .min_h(px(0.))
                            .when(tab.comparison.is_some(), |pane| {
                                pane.child(self.editor_comparison_header("当前编辑", None, cx))
                            })
                            .child(tab.input.clone()),
                    )
                    .when_some(tab.comparison.as_ref(), |pane, comparison| {
                        let (title, action) = match comparison.kind {
                            ComparisonKind::Remote => ("远程版本", Confirm::UseRemote),
                            ComparisonKind::Snapshot => ("快照内容", Confirm::RestoreSnapshot),
                        };
                        pane.child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_w(px(0.))
                                .min_h(px(0.))
                                .border_l_1()
                                .border_color(rgb(color::LINE))
                                .child(self.editor_comparison_header(
                                    title,
                                    Some((tab, action)),
                                    cx,
                                ))
                                .child(comparison.input.clone()),
                        )
                    }),
            )
            .when(
                tab.search.as_ref().is_some_and(EditorSearch::visible),
                |pane| pane.child(self.editor_search_panel(tab, cx)),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(space::SM)
                    .px(space::LG)
                    .h(space::FOOTER_HEIGHT)
                    .border_t_1()
                    .border_color(rgb(color::LINE))
                    .text_size(text::DETAIL)
                    .text_color(rgb(color::MUTED))
                    .children((!tab.connected()).then(|| {
                        div()
                            .flex_none()
                            .text_color(rgb(color::DANGER))
                            .child("SSH 已断开")
                    }))
                    .child(
                        div()
                            .flex_1()
                            .truncate()
                            .when(
                                input.format_notice().is_some_and(|(_, failed)| failed),
                                |label| label.text_color(rgb(color::DANGER)),
                            )
                            .child(if tab.input.read(cx).limit_reached() {
                                "编辑内容最多 2 MiB".into()
                            } else if input.format_pending() {
                                "正在格式化草稿…".into()
                            } else if let Some((notice, _)) = input.format_notice() {
                                notice.to_owned()
                            } else {
                                tab.status.clone().unwrap_or_else(|| {
                                    if tab.dirty(cx) {
                                        "有未保存的修改".into()
                                    } else {
                                        String::new()
                                    }
                                })
                            }),
                    )
                    .child(format!(
                        "UTF-8{} · {} · {} · {line}:{column}",
                        if tab.base.bom { " BOM" } else { "" },
                        tab.base.line_ending,
                        input.language().name()
                    )),
            )
    }

    fn editor_comparison_header(
        &self,
        title: &'static str,
        target: Option<(&EditorTab, Confirm)>,
        cx: &Context<Self>,
    ) -> Div {
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(space::XS)
            .px(space::SM)
            .h(space::TOOLBAR_HEIGHT)
            .bg(rgb(color::TOOLBAR))
            .border_b_1()
            .border_color(rgb(color::LINE))
            .text_size(text::DETAIL)
            .child(div().flex_1().truncate().child(title))
            .when_some(target, |row, (tab, action)| {
                let id = tab.id.clone();
                let close = id.clone();
                row.children((!tab.busy).then(|| {
                    control(
                        "apply-editor-comparison",
                        if matches!(action, Confirm::UseRemote) {
                            "采用远程版本"
                        } else {
                            "载入快照"
                        },
                        false,
                    )
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.request_editor_action(&id, action, window, cx)
                    }))
                }))
                .child(
                    icon_control("close-editor-comparison", Icon::Close, "关闭对照", false)
                        .size(space::CONTROL_HEIGHT)
                        .on_click(cx.listener(move |view, _, window, cx| {
                            if let Some(tab) =
                                view.editors.tabs.iter_mut().find(|tab| tab.id == close)
                            {
                                tab.comparison = None;
                                tab.focus(window, cx);
                                cx.notify();
                            }
                        })),
                )
            })
    }

    fn editor_snapshots(
        &self,
        tab: &EditorTab,
        snapshots: &[SnapshotInfo],
        cx: &Context<Self>,
    ) -> gpui::Stateful<Div> {
        div()
            .id("editor-snapshot-list")
            .flex_none()
            .max_h(px(130.))
            .overflow_y_scroll()
            .px(space::LG)
            .py(space::XS)
            .bg(rgb(color::TOOLBAR))
            .border_b_1()
            .border_color(rgb(color::LINE))
            .when(snapshots.is_empty(), |list| {
                list.child(
                    div()
                        .py(space::SM)
                        .text_color(rgb(color::MUTED))
                        .child("暂无该文件的本地快照"),
                )
            })
            .children(snapshots.iter().enumerate().map(|(index, snapshot)| {
                let id = tab.id.clone();
                let snapshot_id = snapshot.id.clone();
                let reason = match snapshot.reason.as_str() {
                    "open" => "首次打开",
                    "restore" => "恢复前",
                    _ => "保存前",
                };
                let label = format!(
                    "{reason} · {} · {}",
                    snapshot_age(snapshot.created_at),
                    file_size(snapshot.bytes as u64)
                );
                control(
                    ElementId::named_usize("editor-snapshot", index),
                    label,
                    false,
                )
                .justify_start()
                .on_click(cx.listener(move |view, _, window, cx| {
                    view.run_editor(&id, Operation::Snapshot(snapshot_id.clone()), window, cx)
                }))
            }))
    }

    pub(super) fn editor_confirmation(&self, cx: &Context<Self>) -> Div {
        let Some((id, action)) = &self.editors.confirmation else {
            return div();
        };
        let action = *action;
        let Some(tab) = self.editors.tabs.iter().find(|tab| &tab.id == id) else {
            return div();
        };
        let message = match action {
            Confirm::Close => "关闭标签会丢弃当前编辑内容。",
            Confirm::Reload => "重新读取会替换当前编辑内容。",
            Confirm::UseRemote => "采用远程版本会替换当前编辑内容。",
            Confirm::Overwrite => {
                "将更新刚才对照的远程版本。保存前会保留快照，远程文件若再次变更会重新提示。"
            }
            Confirm::RestoreSnapshot => "将快照载入编辑器。点击保存后才会更新远程文件。",
        };
        div()
            .p(space::LG)
            .flex()
            .flex_col()
            .gap(space::MD)
            .child(div().truncate().child(tab.path.clone()))
            .child(div().text_color(rgb(color::MUTED)).child(message))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(space::SM)
                    .child(
                        control("cancel-editor-action", "取消", false).on_click(
                            cx.listener(|view, _, window, cx| view.close_dialog(window, cx)),
                        ),
                    )
                    .child(
                        (match action {
                            Confirm::Overwrite => {
                                primary_control("confirm-editor-action", "记录快照并保存")
                            }
                            Confirm::RestoreSnapshot => {
                                primary_control("confirm-editor-action", "载入快照")
                            }
                            _ => danger_control("confirm-editor-action", "放弃修改"),
                        })
                        .on_click(cx.listener(|view, _, window, cx| {
                            if let Some((id, action)) = view.editors.confirmation.take() {
                                view.close_dialog(window, cx);
                                view.apply_editor_action(&id, action, window, cx);
                            }
                        })),
                    )
                    .when(matches!(action, Confirm::Close) && tab.connected(), |row| {
                        row.child(primary_control("save-close-editor", "保存并关闭").on_click(
                            cx.listener(|view, _, window, cx| {
                                if let Some((id, _)) = view.editors.confirmation.take() {
                                    view.close_dialog(window, cx);
                                    view.run_editor(
                                        &id,
                                        Operation::Save {
                                            close: true,
                                            compared_version: false,
                                        },
                                        window,
                                        cx,
                                    );
                                }
                            }),
                        ))
                    }),
            )
    }
}

fn snapshot_age(created_at: u64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let minutes = now.saturating_sub(created_at) / 60_000;
    match minutes {
        0 => "刚刚".into(),
        1..60 => format!("{minutes} 分钟前"),
        60..1440 => format!("{} 小时前", minutes / 60),
        _ => format!("{} 天前", minutes / 1440),
    }
}
