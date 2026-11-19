use std::collections::HashSet;

use gpui::{
    Anchored, Context, Div, ElementId, Entity, Pixels, Point, Window, anchored, div, prelude::*, px,
};

use super::{Dialog, Workbench, actions::dialog_tab};
use crate::{
    icons::{Icon, icon},
    ssh_backend::ConnectionGroupChange,
    text_field::TextField,
    theme::{color, control, primary_control, rgb, space, text},
};

#[derive(Clone, Copy)]
pub(super) enum GroupMenuTarget {
    Host(usize),
    Group(usize),
}

pub(super) struct GroupMenu {
    target: GroupMenuTarget,
    position: Point<Pixels>,
}

pub(super) struct GroupDraft {
    change: ConnectionGroupChange,
    name: Entity<TextField>,
    from_management: bool,
    error: Option<String>,
}

impl Workbench {
    pub(super) fn open_group_menu(
        &mut self,
        target: GroupMenuTarget,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.menu = None;
        self.file_menu = None;
        self.command_picker.dismiss_groups();
        self.group_menu = Some(GroupMenu { target, position });
        cx.notify();
    }

    pub(super) fn group_context_menu(&self, menu: &GroupMenu, cx: &Context<Self>) -> Anchored {
        let target = menu.target;
        let (label, glyph) = match target {
            GroupMenuTarget::Host(_) => ("移到分组…", Icon::Files),
            GroupMenuTarget::Group(_) => ("重命名分组…", Icon::Pencil),
        };
        anchored()
            .position(menu.position)
            .snap_to_window_with_margin(space::SM)
            .child(
                div()
                    .id("connection-group-menu")
                    .w(px(160.))
                    .p(space::XS)
                    .rounded(space::CONTROL_RADIUS)
                    .border_1()
                    .border_color(rgb(color::LINE))
                    .bg(rgb(color::SURFACE))
                    .shadow_md()
                    .occlude()
                    .on_mouse_down_out(cx.listener(|view, _, _, cx| {
                        if view.group_menu.take().is_some() {
                            cx.notify();
                        }
                    }))
                    .child(
                        control("connection-group-action", label, false)
                            .w_full()
                            .justify_start()
                            .gap(space::XS)
                            .child(icon(glyph, color::MUTED).size(space::COMPACT_ICON_SIZE))
                            .on_click(cx.listener(move |view, _, window, cx| {
                                view.group_menu = None;
                                match target {
                                    GroupMenuTarget::Host(index) => {
                                        view.begin_move_connections([index], false, window, cx);
                                    }
                                    GroupMenuTarget::Group(index) => {
                                        if let Some(group) = view.groups.get(index).cloned() {
                                            view.open_group_editor(
                                                ConnectionGroupChange::Rename(group.clone()),
                                                &group,
                                                false,
                                                window,
                                                cx,
                                            );
                                        }
                                    }
                                }
                            })),
                    ),
            )
    }

    pub(super) fn batch_group_controls(&self, cx: &Context<Self>) -> Div {
        let row = div().flex_none().flex().items_center().gap(space::XS);
        let Some(selected) = &self.batch_group_selection else {
            return row.child(
                dialog_tab(control("start-batch-group-move", "批量移组", false), 3).on_click(
                    cx.listener(|view, _, window, cx| {
                        view.batch_group_selection = Some(HashSet::new());
                        view.management_filter.read(cx).focus(window);
                        cx.notify();
                    }),
                ),
            );
        };
        row.child(
            dialog_tab(control("cancel-batch-group-move", "取消", false), 3).on_click(cx.listener(
                |view, _, window, cx| {
                    view.batch_group_selection = None;
                    view.management_filter.read(cx).focus(window);
                    cx.notify();
                },
            )),
        )
        .when(selected.is_empty(), |row| {
            row.child(
                div()
                    .px(space::XS)
                    .text_color(rgb(color::MUTED))
                    .child("选择主机"),
            )
        })
        .when(!selected.is_empty(), |row| {
            row.child(
                dialog_tab(
                    primary_control("move-selected-hosts", format!("移动 {} 台", selected.len())),
                    4,
                )
                .on_click(cx.listener(|view, _, window, cx| {
                    if let Some(selected) = view.batch_group_selection.clone() {
                        view.begin_move_connections(selected, true, window, cx);
                    }
                })),
            )
        })
    }

    fn begin_move_connections(
        &mut self,
        indexes: impl IntoIterator<Item = usize>,
        from_management: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ids: HashSet<_> = indexes
            .into_iter()
            .filter_map(|index| self.profiles.get(index))
            .filter(|profile| !profile.temporary)
            .filter_map(|profile| profile.remote.as_ref().map(|remote| remote.id.clone()))
            .collect();
        if !ids.is_empty() {
            self.open_group_editor(
                ConnectionGroupChange::Move(ids),
                "",
                from_management,
                window,
                cx,
            );
        }
    }

    fn open_group_editor(
        &mut self,
        change: ConnectionGroupChange,
        initial: &str,
        from_management: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let placeholder = match &change {
            ConnectionGroupChange::Rename(_) => "输入新的分组名",
            ConnectionGroupChange::Move(_) => "选择分组，或输入新分组名",
        };
        let name = cx.new(|cx| {
            TextField::new(cx, placeholder)
                .with_limit(120)
                .with_tab_index(1)
        });
        cx.observe(&name, |_, _, cx| cx.notify()).detach();
        name.update(cx, |field, cx| {
            field.set_value(initial, cx);
            field.select_all();
        });
        self.open_dialog(Dialog::ConnectionGroup, window, cx);
        name.read(cx).focus(window);
        self.group_draft = Some(GroupDraft {
            change,
            name,
            from_management,
            error: None,
        });
        cx.notify();
    }

    pub(super) fn group_editor_focused(&self, window: &Window, cx: &Context<Self>) -> bool {
        self.group_draft.as_ref().is_some_and(|draft| {
            let name = draft.name.read(cx);
            name.is_focused(window) && !name.is_composing()
        })
    }

    pub(super) fn close_group_editor(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self
            .group_draft
            .take()
            .is_some_and(|draft| draft.from_management)
        {
            self.dialog = Some(Dialog::Connections);
            self.management_filter.read(cx).focus(window);
            cx.notify();
            return true;
        }
        false
    }

    pub(super) fn group_editor_title(&self) -> &'static str {
        if self
            .group_draft
            .as_ref()
            .is_some_and(|draft| matches!(&draft.change, ConnectionGroupChange::Rename(_)))
        {
            "重命名分组"
        } else {
            "移到分组"
        }
    }

    pub(super) fn group_editor(&self, cx: &Context<Self>) -> Div {
        let Some(draft) = &self.group_draft else {
            return div();
        };
        let selected = draft.name.read(cx).value().trim();
        div()
            .p(space::LG)
            .flex()
            .flex_col()
            .gap(space::MD)
            .child(draft.name.clone())
            .when_some(
                match &draft.change {
                    ConnectionGroupChange::Move(ids) => Some(ids.len()),
                    ConnectionGroupChange::Rename(_) => None,
                },
                |body, count| {
                    let groups = self.groups.iter().enumerate().filter(|(index, group)| {
                        group.as_str() != "未分组"
                            && self.profiles.iter().any(|profile| {
                                profile.group == *index
                                    && !profile.temporary
                                    && profile.remote.is_some()
                            })
                    });
                    body.child(
                        div()
                            .text_size(text::DETAIL)
                            .text_color(rgb(color::MUTED))
                            .child(format!("移动 {count} 台主机")),
                    )
                    .child(
                        div().flex().flex_wrap().gap(space::XS).children(
                            std::iter::once((self.groups.len(), "未分组"))
                                .chain(groups.map(|(index, name)| (index, name.as_str())))
                                .map(|(index, name)| {
                                    let name = name.to_owned();
                                    dialog_tab(
                                        control(
                                            ElementId::named_usize("move-hosts-target", index),
                                            name.clone(),
                                            name == selected,
                                        ),
                                        2,
                                    )
                                    .on_click(cx.listener(
                                        move |view, _, window, cx| {
                                            if let Some(draft) = &view.group_draft {
                                                draft.name.update(cx, |field, cx| {
                                                    field.set_value(&name, cx)
                                                });
                                                draft.name.read(cx).focus(window);
                                            }
                                            cx.notify();
                                        },
                                    ))
                                }),
                        ),
                    )
                },
            )
            .when_some(draft.error.clone(), |body, error| {
                body.child(div().text_color(rgb(color::DANGER)).child(error))
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(space::SM)
                    .child(
                        dialog_tab(control("cancel-group-edit", "取消", false), 3).on_click(
                            cx.listener(|view, _, window, cx| view.close_dialog(window, cx)),
                        ),
                    )
                    .child(
                        dialog_tab(primary_control("save-group-edit", "保存"), 4).on_click(
                            cx.listener(|view, _, window, cx| view.save_group_change(window, cx)),
                        ),
                    ),
            )
    }

    pub(super) fn save_group_change(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = &mut self.group_draft else {
            return;
        };
        let name = draft.name.read(cx).value().trim();
        if name.is_empty() {
            draft.error = Some(
                match &draft.change {
                    ConnectionGroupChange::Rename(_) => "请输入新的分组名",
                    ConnectionGroupChange::Move(_) => "请选择目标分组，或输入新分组名",
                }
                .to_owned(),
            );
            cx.notify();
            return;
        }
        let result = self
            .backend
            .as_ref()
            .ok_or_else(|| "连接配置不可用".to_owned())
            .and_then(|backend| {
                backend
                    .change_connection_group(&draft.change, name)
                    .map_err(|error| format!("{error:#}"))
            });
        match result {
            Ok((group, ids)) => {
                let new_index = self.add_group(&group, true);
                for profile in &mut self.profiles {
                    if let Some(remote) = &mut profile.remote
                        && !profile.temporary
                        && ids.contains(&remote.id)
                    {
                        remote.group = group.clone();
                        profile.group = new_index;
                    }
                }
                self.batch_group_selection = None;
                self.close_dialog(window, cx);
            }
            Err(error) => draft.error = Some(error),
        }
        cx.notify();
    }
}
