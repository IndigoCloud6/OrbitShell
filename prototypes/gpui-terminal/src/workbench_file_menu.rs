use std::sync::Arc;

use gpui::{
    Anchored, ClipboardItem, Context, Div, ElementId, KeyDownEvent, Pixels, Point, Window,
    anchored, div, prelude::*, px,
};
use orbitshell_ssh::sftp::{self, RemoteEntry};

use super::{
    Dialog, Page, Workbench, control,
    editor::can_edit_remote,
    transfers::{DownloadTarget, UploadTarget},
};
use crate::{
    icons::{Icon, icon},
    theme::{color, danger_control, rgb, space, text},
};

pub(super) struct FileMenu {
    pub target: UploadTarget,
    pub entry: Option<RemoteEntry>,
    pub position: Point<Pixels>,
    pub cursor: Option<usize>,
}

pub(super) struct RemoteDelete {
    pub target: UploadTarget,
    pub entry: RemoteEntry,
}

#[derive(Clone, Copy)]
enum FileAction {
    Open,
    Upload,
    Download,
    Edit,
    CopyName,
    CopyPath,
    Delete,
    Refresh,
    Parent,
    Locate,
    Hidden,
}

struct Item {
    action: FileAction,
    title: &'static str,
    icon: Icon,
    separator: bool,
}

impl Item {
    fn new(action: FileAction, title: &'static str, icon: Icon) -> Self {
        Self {
            action,
            title,
            icon,
            separator: false,
        }
    }

    fn separated(mut self) -> Self {
        self.separator = true;
        self
    }
}

impl FileMenu {
    fn items(&self, show_hidden: bool) -> Vec<Item> {
        use FileAction::*;
        if let Some(entry) = &self.entry {
            let mut items = if entry.is_directory {
                vec![
                    Item::new(Open, "打开目录", Icon::Files),
                    Item::new(Upload, "上传到此目录", Icon::Upload),
                ]
            } else {
                vec![Item::new(Download, "下载文件", Icon::Download)]
            };
            if can_edit_remote(entry) {
                items.insert(0, Item::new(Edit, "编辑文件", Icon::Pencil));
            }
            items.extend([
                Item::new(CopyName, "复制名称", Icon::Copy).separated(),
                Item::new(
                    CopyPath,
                    if entry.is_directory {
                        "复制目录路径"
                    } else {
                        "复制完整路径"
                    },
                    Icon::Copy,
                ),
                Item::new(Delete, "删除", Icon::Trash).separated(),
            ]);
            items
        } else {
            let mut items = vec![
                Item::new(Upload, "上传文件", Icon::Upload),
                Item::new(Refresh, "刷新目录", Icon::Refresh),
            ];
            if self.target.directory != "/" {
                items.push(Item::new(Parent, "返回上级目录", Icon::ArrowUp));
            }
            items.push(Item::new(Locate, "定位到终端当前目录", Icon::FolderSearch));
            let (title, icon) = if show_hidden {
                ("隐藏点号文件", Icon::Eye)
            } else {
                ("显示隐藏文件", Icon::EyeOff)
            };
            items.extend([
                Item::new(Hidden, title, icon).separated(),
                Item::new(CopyPath, "复制目录路径", Icon::Copy),
            ]);
            items
        }
    }
}

impl Workbench {
    pub(super) fn file_target_is_current(&self, target: &UploadTarget) -> bool {
        self.show_files
            && matches!(self.page, Page::Terminal | Page::Editor)
            && self
                .active_session()
                .filter(|session| session.profile == target.profile)
                .and_then(|session| session.remote.as_ref())
                .is_some_and(|remote| {
                    Arc::ptr_eq(&remote.live, &target.live)
                        && remote
                            .ready_directory()
                            .is_some_and(|directory| directory.path == target.directory)
                })
    }

    pub(super) fn remote_file_context_menu(&self, menu: &FileMenu, cx: &Context<Self>) -> Anchored {
        anchored()
            .position(menu.position)
            .snap_to_window_with_margin(space::SM)
            .child(
                div()
                    .id("remote-file-context-menu")
                    .w(px(192.))
                    .p(space::XS)
                    .bg(rgb(color::SURFACE))
                    .border_1()
                    .border_color(rgb(color::LINE))
                    .rounded(space::CONTROL_RADIUS)
                    .shadow_md()
                    .occlude()
                    .on_mouse_down_out(cx.listener(|view, _, _, cx| {
                        if view.file_menu.take().is_some() {
                            cx.notify();
                        }
                    }))
                    .children(
                        menu.items(self.show_hidden_files)
                            .into_iter()
                            .enumerate()
                            .map(|(index, item)| {
                                let selected = menu.cursor == Some(index);
                                let destructive = matches!(item.action, FileAction::Delete);
                                div()
                                    .when(item.separator, |group| {
                                        group.child(
                                            div()
                                                .h(px(1.))
                                                .mx(space::XS)
                                                .my(space::XS)
                                                .bg(rgb(color::LINE)),
                                        )
                                    })
                                    .child(
                                        div()
                                            .id(ElementId::named_usize("remote-file-action", index))
                                            .h(space::CONTROL_HEIGHT)
                                            .flex()
                                            .items_center()
                                            .gap(space::SM)
                                            .px(space::SM)
                                            .rounded(space::CONTROL_RADIUS)
                                            .cursor_pointer()
                                            .text_size(text::DETAIL)
                                            .text_color(rgb(if destructive {
                                                color::DANGER
                                            } else {
                                                color::TEXT
                                            }))
                                            .when(selected, |row| row.bg(rgb(color::HOVER)))
                                            .hover(move |row| {
                                                row.bg(rgb(if destructive {
                                                    color::WINDOW_CLOSE_HOVER
                                                } else {
                                                    color::HOVER
                                                }))
                                            })
                                            .child(
                                                icon(
                                                    item.icon,
                                                    if destructive {
                                                        color::DANGER
                                                    } else {
                                                        color::MUTED
                                                    },
                                                )
                                                .size(space::COMPACT_ICON_SIZE),
                                            )
                                            .child(item.title)
                                            .on_click(cx.listener(move |view, _, window, cx| {
                                                view.perform_remote_file_action(
                                                    item.action,
                                                    window,
                                                    cx,
                                                )
                                            })),
                                    )
                            }),
                    ),
            )
    }

    pub(super) fn handle_file_menu_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(menu) = self.file_menu.as_mut() else {
            return false;
        };
        let items = menu.items(self.show_hidden_files);
        match event.keystroke.key.as_str() {
            "down" => menu.cursor = Some(menu.cursor.map_or(0, |index| (index + 1) % items.len())),
            "up" => {
                menu.cursor = Some(menu.cursor.map_or(items.len() - 1, |index| {
                    (index + items.len() - 1) % items.len()
                }))
            }
            "enter" => {
                if let Some(item) = menu.cursor.and_then(|index| items.get(index)) {
                    self.perform_remote_file_action(item.action, window, cx);
                }
            }
            "escape" => self.file_menu = None,
            _ => return false,
        }
        cx.notify();
        true
    }

    fn perform_remote_file_action(
        &mut self,
        action: FileAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(menu) = self.file_menu.take() else {
            return;
        };
        cx.notify();
        if !self.file_target_is_current(&menu.target) {
            return;
        }
        let target = menu.target;
        match action {
            FileAction::Open => {
                if let Some(entry) = menu.entry {
                    self.load_remote_dir(target.profile, entry.path, cx);
                }
            }
            FileAction::Upload => {
                let directory = menu.entry.map_or(target.directory, |entry| entry.path);
                self.choose_upload(
                    UploadTarget {
                        directory,
                        ..target
                    },
                    window,
                    cx,
                );
            }
            FileAction::Download => {
                if let Some(entry) = menu.entry {
                    self.choose_download(
                        DownloadTarget {
                            live: target.live,
                            path: entry.path,
                            name: entry.name,
                        },
                        window,
                        cx,
                    );
                }
            }
            FileAction::Edit => {
                if let Some(entry) = menu.entry {
                    self.open_remote_editor(target, entry, window, cx);
                }
            }
            FileAction::CopyName => {
                if let Some(entry) = menu.entry {
                    cx.write_to_clipboard(ClipboardItem::new_string(entry.name));
                }
            }
            FileAction::CopyPath => {
                let path = menu.entry.map_or(target.directory, |entry| entry.path);
                cx.write_to_clipboard(ClipboardItem::new_string(path));
            }
            FileAction::Refresh => self.load_remote_dir(target.profile, target.directory, cx),
            FileAction::Parent => self.go_remote_parent(target.profile, cx),
            FileAction::Locate => self.locate_terminal_directory(target.profile, cx),
            FileAction::Hidden => self.toggle_hidden_files(cx),
            FileAction::Delete => {
                if let Some(entry) = menu.entry {
                    self.file_delete = Some(RemoteDelete { target, entry });
                    self.open_dialog(Dialog::DeleteRemoteFile, window, cx);
                }
            }
        }
    }

    pub(super) fn remote_delete_dialog(&self, cx: &Context<Self>) -> Div {
        let Some(delete) = &self.file_delete else {
            return div();
        };
        let entry = &delete.entry;
        let title = if entry.is_symlink {
            "删除符号链接？"
        } else if entry.is_directory {
            "删除远程空目录？"
        } else {
            "删除远程文件？"
        };
        div()
            .p(space::LG)
            .flex()
            .flex_col()
            .gap(space::SM)
            .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child(title))
            .child(
                div()
                    .text_color(rgb(color::MUTED))
                    .text_size(text::DETAIL)
                    .child(self.profiles[delete.target.profile].title.clone()),
            )
            .child(
                div()
                    .rounded(space::CONTROL_RADIUS)
                    .bg(rgb(color::TOOLBAR))
                    .px(space::SM)
                    .py(space::XS)
                    .font_family(text::MONO_FONT)
                    .whitespace_normal()
                    .child(entry.path.clone()),
            )
            .child(
                div()
                    .text_size(text::DETAIL)
                    .text_color(rgb(color::MUTED))
                    .child("此操作无法撤销。"),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(space::SM)
                    .child(
                        control("cancel-remote-delete", "取消", false).on_click(
                            cx.listener(|view, _, window, cx| view.close_dialog(window, cx)),
                        ),
                    )
                    .child(danger_control("confirm-remote-delete", "删除").on_click(
                        cx.listener(|view, _, window, cx| view.confirm_remote_delete(window, cx)),
                    )),
            )
    }

    fn confirm_remote_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(delete) = self.file_delete.take() else {
            return;
        };
        self.close_dialog(window, cx);
        if !self.file_target_is_current(&delete.target) {
            return;
        }
        let Some(backend) = self.backend.as_ref() else {
            return;
        };
        let runtime = backend.runtime_handle();
        let target = delete.target.clone();
        let entry = delete.entry.clone();
        cx.spawn(async move |view, cx| {
            let result = runtime
                .spawn(async move {
                    sftp::delete_entry(
                        &target.live,
                        &target.directory,
                        &entry.path,
                        entry.is_directory,
                        entry.is_symlink,
                    )
                    .await
                })
                .await
                .unwrap_or_else(|_| Err("无法确认删除结果，请刷新目录".into()));
            let _ = view.update(cx, |view, cx| {
                view.finish_remote_delete(&delete, result, cx)
            });
        })
        .detach();
    }
}
