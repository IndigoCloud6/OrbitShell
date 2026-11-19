use gpui::{
    Context, Div, ElementId, Entity, FocusHandle, FontWeight, MouseButton, ScrollHandle, Window,
    deferred, div, point, prelude::*, px,
};
use orbitshell_ssh::quick_command::QuickCommand;

use super::{Dialog, Workbench, icon_control_with_size};
use crate::{
    icons::{Icon, icon},
    ssh_backend::UNGROUPED_COMMANDS,
    text_field::TextField,
    theme::{color, control, danger_control, primary_control, rgb, space, text},
};

pub(super) struct CommandPicker {
    search: Entity<TextField>,
    group: Option<String>,
    groups_open: bool,
    group_focus: FocusHandle,
    scroll: ScrollHandle,
}

impl CommandPicker {
    pub(super) fn new(cx: &mut Context<Workbench>) -> Self {
        let search = cx.new(|cx| TextField::new(cx, "搜索指令名称或内容…"));
        cx.observe(&search, |view, _, cx| {
            view.command_picker.reset_scroll();
            cx.notify();
        })
        .detach();
        Self {
            search,
            group: None,
            groups_open: false,
            group_focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
        }
    }

    pub(super) fn scroll(&self) -> &ScrollHandle {
        &self.scroll
    }

    pub(super) fn dismiss_groups(&mut self) -> bool {
        std::mem::take(&mut self.groups_open)
    }

    pub(super) fn sync_groups(&mut self, groups: &[String]) {
        if self
            .group
            .as_ref()
            .is_some_and(|group| group != UNGROUPED_COMMANDS && !groups.contains(group))
        {
            self.group = None;
            self.reset_scroll();
        }
    }

    fn reset_scroll(&self) {
        self.scroll.set_offset(point(px(0.), px(0.)));
    }
}

enum ManagementAction {
    EditGroup {
        previous: Option<String>,
        field: Entity<TextField>,
    },
    RemoveCommand {
        id: String,
        name: String,
    },
    RemoveGroup(String),
}

pub(super) struct CommandManager {
    search: Entity<TextField>,
    group: Option<String>,
    action: Option<ManagementAction>,
    error: Option<String>,
}

impl CommandManager {
    pub(super) fn new(cx: &mut Context<Workbench>) -> Self {
        let search = cx.new(|cx| TextField::new(cx, "搜索名称、分组或指令内容…"));
        cx.observe(&search, |_, _, cx| cx.notify()).detach();
        Self {
            search,
            group: None,
            action: None,
            error: None,
        }
    }

    pub(super) fn focus(&self, window: &mut Window, cx: &Context<Workbench>) {
        self.search.read(cx).focus(window);
    }
}

impl Workbench {
    pub(super) fn quick_command_filters(&self, cx: &Context<Self>) -> Div {
        let picker = &self.command_picker;
        let open = picker.groups_open;
        let selected = picker.group.as_deref();
        let label = format!("{} ▾", selected.unwrap_or("全部分组"));

        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(space::SM)
            .px(space::MD)
            .py(space::XS)
            .border_b_1()
            .border_color(rgb(color::LINE))
            .bg(rgb(color::TOOLBAR))
            .child(
                div()
                    .relative()
                    .flex_none()
                    .track_focus(&picker.group_focus)
                    .child(
                        control("pick-command-group", label, open || selected.is_some())
                            .w(px(144.))
                            .justify_start()
                            .truncate()
                            // Use the rendered state because the outside handler runs first.
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |view, _, window, cx| {
                                    view.command_picker.groups_open = !open;
                                    view.command_picker.group_focus.focus(window);
                                    cx.notify();
                                }),
                            ),
                    )
                    .when(open, |anchor| {
                        anchor.child(deferred(self.command_group_options(cx)))
                    }),
            )
            .child(icon(Icon::Search, color::MUTED).size(space::COMPACT_ICON_SIZE))
            .child(div().flex_1().min_w(px(0.)).child(picker.search.clone()))
            .when(picker.search.read(cx).has_value(), |row| {
                row.child(
                    management_icon("clear-command-search", Icon::Close, "清空搜索").on_click(
                        cx.listener(|view, _, window, cx| {
                            view.command_picker
                                .search
                                .update(cx, |field, cx| field.set_value("", cx));
                            view.command_picker.search.read(cx).focus(window);
                        }),
                    ),
                )
            })
    }

    fn command_group_options(&self, cx: &Context<Self>) -> gpui::Stateful<Div> {
        let selected = self.command_picker.group.as_deref();
        div()
            .id("command-group-options")
            .absolute()
            .left(px(0.))
            .bottom(space::CONTROL_HEIGHT + space::XS)
            .w(px(204.))
            .max_h(px(228.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .p(space::XS)
            .occlude()
            .shadow_md()
            .bg(rgb(color::SURFACE))
            .border_1()
            .border_color(rgb(color::LINE))
            .rounded(space::CONTROL_RADIUS)
            .on_mouse_down_out(cx.listener(|view, _, _, cx| {
                if view.command_picker.dismiss_groups() {
                    cx.notify();
                }
            }))
            .children(command_groups(&self.quick_commands.groups).enumerate().map(
                |(index, group)| {
                    let value = group.map(str::to_owned);
                    control(
                        ElementId::named_usize("command-group-option", index),
                        group.unwrap_or("全部分组").to_owned(),
                        selected == group,
                    )
                    .justify_start()
                    .truncate()
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.command_picker.group = value.clone();
                        view.command_picker.dismiss_groups();
                        view.command_picker.reset_scroll();
                        cx.notify();
                    }))
                },
            ))
    }

    pub(super) fn quick_commands(&self, cx: &Context<Self>) -> Div {
        let picker = &self.command_picker;
        let group = picker.group.as_deref();
        let query = picker.search.read(cx).value().trim().to_lowercase();
        let entries = &self.quick_commands.commands;
        let items: Vec<_> = entries
            .iter()
            .map(|entry| {
                (
                    entry.name.as_str(),
                    entry.group.as_str(),
                    entry.command.as_str(),
                )
            })
            .enumerate()
            .filter(|&(_, (name, entry_group, command))| {
                group.is_none_or(|selected| selected == entry_group)
                    && command_matches(&query, [name, entry_group, command])
            })
            .collect();
        div()
            .when(items.is_empty(), |list| {
                list.child(div().py(space::SM).child("没有匹配的快捷指令"))
            })
            .children(items.into_iter().map(|(index, (name, _, command))| {
                let command = command.to_owned();
                div()
                    .h(space::ROW_HEIGHT)
                    .flex()
                    .items_center()
                    .gap(space::SM)
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .truncate()
                            .child(name.to_owned()),
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
                            ElementId::named_usize("draft-command", index),
                            "填入终端",
                            false,
                        )
                        .on_click(cx.listener(
                            move |view, _, window, cx| view.draft_command(&command, window, cx),
                        )),
                    )
            }))
    }

    pub(super) fn open_quick_command_manager(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.command_manager.action = None;
        self.command_manager.error = None;
        match self
            .backend
            .as_ref()
            .ok_or_else(|| "工作区配置不可用。".to_owned())
            .and_then(|backend| backend.quick_commands())
        {
            Ok(collection) => {
                if self.command_manager.group.as_ref().is_some_and(|name| {
                    name != UNGROUPED_COMMANDS && !collection.groups.contains(name)
                }) {
                    self.command_manager.group = None;
                }
                self.quick_commands = collection;
            }
            Err(error) => self.command_manager.error = Some(error),
        }
        self.open_dialog(Dialog::QuickCommands, window, cx);
        self.command_manager.focus(window, cx);
    }

    fn edit_command_group(
        &mut self,
        previous: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let field = cx.new(|cx| TextField::new(cx, "分组名称").with_limit(120));
        if let Some(name) = &previous {
            field.update(cx, |field, cx| {
                field.set_value(name, cx);
                field.select_all();
            });
        }
        field.read(cx).focus(window);
        self.show_command_management_action(ManagementAction::EditGroup { previous, field }, cx);
    }

    fn show_command_management_action(&mut self, action: ManagementAction, cx: &mut Context<Self>) {
        self.command_manager.action = Some(action);
        self.command_manager.error = None;
        cx.notify();
    }

    fn submit_command_management_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(action) = &self.command_manager.action else {
            return;
        };
        let Some(backend) = &self.backend else {
            self.command_manager.error = Some("工作区配置不可用。".into());
            cx.notify();
            return;
        };
        let mut group = self.command_manager.group.clone();
        let result = match action {
            ManagementAction::EditGroup { previous, field } => {
                let name = field.read(cx).value();
                group = Some(name.trim().to_owned());
                backend.save_quick_command_group(name, previous.as_deref())
            }
            ManagementAction::RemoveCommand { id, .. } => backend.delete_quick_command(id),
            ManagementAction::RemoveGroup(name) => {
                group = Some(UNGROUPED_COMMANDS.into());
                backend.delete_quick_command_group(name)
            }
        };
        match result {
            Ok(collection) => {
                self.quick_commands = collection;
                self.command_manager.group = group;
                self.command_manager.action = None;
                self.command_manager.error = None;
                self.command_manager.focus(window, cx);
            }
            Err(error) => self.command_manager.error = Some(error),
        }
        cx.notify();
    }

    pub(super) fn quick_command_manager(&self, cx: &Context<Self>) -> Div {
        let manager = &self.command_manager;
        let query = manager.search.read(cx).value().trim().to_lowercase();
        let items: Vec<_> = self
            .quick_commands
            .commands
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                manager
                    .group
                    .as_ref()
                    .is_none_or(|group| group == &item.group)
                    && command_matches(&query, [&item.name, &item.group, &item.command])
            })
            .collect();
        let viewport = div()
            .id("managed-quick-commands")
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll()
            .px(space::MD)
            .when(items.is_empty(), |list| {
                list.child(div().py(space::LG).text_color(rgb(color::MUTED)).child(
                    if self.quick_commands.commands.is_empty() {
                        "暂无已保存的快捷指令"
                    } else {
                        "没有匹配的快捷指令"
                    },
                ))
            })
            .children(
                items
                    .into_iter()
                    .map(|(index, item)| self.managed_command_row(index, item, cx)),
            );
        div()
            .flex_1()
            .min_w(px(0.))
            .min_h(px(0.))
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(space::SM)
                    .p(space::MD)
                    .border_b_1()
                    .border_color(rgb(color::LINE))
                    .bg(rgb(color::TOOLBAR))
                    .child(icon(Icon::Search, color::MUTED).size(space::COMPACT_ICON_SIZE))
                    .child(div().flex_1().min_w(px(0.)).child(manager.search.clone()))
                    .child(
                        primary_control("new-quick-command", "新建指令").on_click(cx.listener(
                            |view, _, window, cx| {
                                let command = QuickCommand {
                                    id: uuid::Uuid::new_v4().to_string(),
                                    name: String::new(),
                                    group: view.command_manager.group.clone().unwrap_or_default(),
                                    command: String::new(),
                                    append_newline: false,
                                };
                                view.open_command_form(command, false, window, cx);
                            },
                        )),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h(px(0.))
                    .min_w(px(0.))
                    .child(self.quick_command_groups(cx))
                    .child(viewport),
            )
            .when_some(manager.action.as_ref(), |panel, action| {
                panel.child(self.command_management_action(action, cx))
            })
            .when_some(manager.error.clone(), |panel, error| {
                panel.child(
                    div()
                        .flex_none()
                        .px(space::MD)
                        .py(space::XS)
                        .text_size(text::DETAIL)
                        .text_color(rgb(color::DANGER))
                        .child(error),
                )
            })
    }

    fn quick_command_groups(&self, cx: &Context<Self>) -> Div {
        let selected = self.command_manager.group.as_deref();
        let groups = command_groups(&self.quick_commands.groups);
        div()
            .w(px(162.))
            .flex_none()
            .min_h(px(0.))
            .flex()
            .flex_col()
            .border_r_1()
            .border_color(rgb(color::LINE))
            .bg(rgb(color::SIDEBAR))
            .child(
                div()
                    .id("quick-command-groups")
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .p(space::SM)
                    .children(groups.enumerate().map(|(index, group)| {
                        let name = group.map(str::to_owned);
                        control(
                            ElementId::named_usize("command-group", index),
                            group.unwrap_or("全部指令").to_owned(),
                            selected == group,
                        )
                        .w_full()
                        .justify_start()
                        .overflow_hidden()
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.command_manager.group = name.clone();
                            view.command_manager.action = None;
                            view.command_manager.error = None;
                            cx.notify();
                        }))
                    })),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(space::XS)
                    .p(space::SM)
                    .border_t_1()
                    .border_color(rgb(color::LINE))
                    .child(control("new-command-group", "新建分组", false).on_click(
                        cx.listener(|view, _, window, cx| {
                            view.edit_command_group(None, window, cx)
                        }),
                    ))
                    .when_some(
                        selected.filter(|name| *name != UNGROUPED_COMMANDS),
                        |row, group| {
                            let group = group.to_owned();
                            let renamed = group.clone();
                            row.child(
                                management_icon("rename-command-group", Icon::Pencil, "重命名分组")
                                    .on_click(cx.listener(move |view, _, window, cx| {
                                        view.edit_command_group(Some(renamed.clone()), window, cx);
                                    })),
                            )
                            .child(
                                management_icon("remove-command-group", Icon::Trash, "删除分组")
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        view.show_command_management_action(
                                            ManagementAction::RemoveGroup(group.clone()),
                                            cx,
                                        );
                                    })),
                            )
                        },
                    ),
            )
    }

    fn managed_command_row(&self, index: usize, item: &QuickCommand, cx: &Context<Self>) -> Div {
        let id = item.id.clone();
        let edit_id = id.clone();
        let name = item.name.clone();
        let preview = item.command.lines().next().unwrap_or_default().to_owned();
        div()
            .flex()
            .items_center()
            .gap(space::SM)
            .py(space::SM)
            .border_b_1()
            .border_color(rgb(color::LINE))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .truncate()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(item.name.clone()),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(text::DETAIL)
                            .text_color(rgb(color::MUTED))
                            .font_family(text::MONO_FONT)
                            .child(preview),
                    ),
            )
            .child(
                div()
                    .w(px(85.))
                    .truncate()
                    .text_size(text::DETAIL)
                    .text_color(rgb(color::MUTED))
                    .child(item.group.clone()),
            )
            .child(
                management_icon(
                    ElementId::named_usize("edit-managed-command", index),
                    Icon::Pencil,
                    "编辑指令",
                )
                .on_click(cx.listener(move |view, _, window, cx| {
                    if let Some(command) = view
                        .quick_commands
                        .commands
                        .iter()
                        .find(|command| command.id == edit_id)
                        .cloned()
                    {
                        view.open_command_form(command, true, window, cx);
                    }
                })),
            )
            .child(
                management_icon(
                    ElementId::named_usize("remove-managed-command", index),
                    Icon::Trash,
                    "删除指令",
                )
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.show_command_management_action(
                        ManagementAction::RemoveCommand {
                            id: id.clone(),
                            name: name.clone(),
                        },
                        cx,
                    );
                })),
            )
    }

    fn command_management_action(&self, action: &ManagementAction, cx: &Context<Self>) -> Div {
        let submit =
            cx.listener(|view, _, window, cx| view.submit_command_management_action(window, cx));
        let section = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(space::SM)
            .p(space::MD)
            .border_t_1()
            .border_color(rgb(color::LINE))
            .bg(rgb(color::TOOLBAR));
        let cancel = control("cancel-command-action", "取消", false).on_click(cx.listener(
            |view, _, window, cx| {
                view.command_manager.action = None;
                view.command_manager.error = None;
                view.command_manager.focus(window, cx);
                cx.notify();
            },
        ));
        if let ManagementAction::EditGroup { previous, field } = action {
            return section
                .child(if previous.is_some() {
                    "重命名分组"
                } else {
                    "新建分组"
                })
                .child(div().flex_1().min_w(px(0.)).child(field.clone()))
                .child(cancel)
                .child(primary_control("save-command-group", "保存分组").on_click(submit));
        }
        let confirmation = match action {
            ManagementAction::RemoveCommand { name, .. } => format!("删除快捷指令「{name}」？"),
            ManagementAction::RemoveGroup(name) => {
                format!("删除分组「{name}」？其中的指令会移到「未分组」。")
            }
            ManagementAction::EditGroup { .. } => unreachable!(),
        };
        section
            .child(div().flex_1().min_w(px(0.)).child(confirmation))
            .child(cancel)
            .child(danger_control("confirm-command-removal", "删除").on_click(submit))
    }
}

fn command_groups(groups: &[String]) -> impl Iterator<Item = Option<&str>> {
    [None, Some(UNGROUPED_COMMANDS)]
        .into_iter()
        .chain(groups.iter().map(|name| Some(name.as_str())))
}

fn command_matches(query: &str, fields: [&str; 3]) -> bool {
    query.is_empty()
        || fields
            .iter()
            .any(|value| value.to_lowercase().contains(query))
}

fn management_icon(
    id: impl Into<ElementId>,
    glyph: Icon,
    label: &'static str,
) -> gpui::Stateful<Div> {
    icon_control_with_size(id, glyph, label, false, space::COMPACT_ICON_SIZE)
        .size(space::CONTROL_HEIGHT)
}
