use std::time::Duration;

#[cfg(windows)]
use gpui::FontWeight;
use gpui::{Context, Div, Window, div, prelude::*, px};
use orbitshell_ssh::launch::{PendingLaunch, parse_link};

#[cfg(windows)]
use super::actions::setting_action;
use super::{
    Dialog, LoginCredential, LoginOrigin, Page, Profile, SettingsTab, Workbench,
    actions::{setting_choices, setting_group, setting_row},
    control,
};
use crate::theme::{color, primary_control, rgb, space, text};

impl Workbench {
    #[cfg(windows)]
    pub(super) fn listen_external_links(&mut self, window: &Window, cx: &mut Context<Self>) {
        use orbitshell_ssh::launch::first_argument_link;
        let (sender, mut launches) = tokio::sync::mpsc::channel(16);
        if !crate::windows_launch::register_window(window, sender) {
            self.bastion_registration = Some("当前窗口无法接收外部链接转发".into());
            return;
        }
        cx.spawn_in(window, async move |view, cx| {
            while let Some(arguments) = launches.recv().await {
                let Some(launch) = first_argument_link(&arguments) else {
                    continue;
                };
                if view
                    .update_in(cx, |view, window, cx| {
                        if view.external_launches.len() < 16 {
                            view.external_launches.push_back(launch);
                            view.show_desktop_window(window);
                            cx.notify();
                            cx.spawn(async |view, cx| {
                                cx.background_executor()
                                    .timer(Duration::from_secs(120))
                                    .await;
                                let _ = view.update(cx, |_, cx| cx.notify());
                            })
                            .detach();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    #[cfg(windows)]
    pub(super) fn show_external_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.external_launches.retain(|request| {
            request.as_ref().map_or(true, |launch| {
                launch.created.elapsed() < Duration::from_secs(120)
            })
        });
        if self.dialog.is_none()
            && let Some(launch) = self.external_launches.pop_front()
        {
            self.receive_bastion_launch(launch, window, cx);
        }
    }

    pub(super) fn clear_bastion_link(&mut self, cx: &mut Context<Self>) {
        self.bastion_link
            .update(cx, |field, cx| field.set_value("", cx));
        self.bastion_error = None;
        self.bastion_decode = false;
    }

    pub(super) fn bastion_settings(&self, cx: &Context<Self>) -> Div {
        let username_choices = setting_choices().children(
            [
                ("preserve-bastion-user", "原样发送", false),
                ("decode-bastion-user", "解码 b64>>", true),
            ]
            .map(|(id, label, decode)| {
                control(id, label, self.bastion_decode == decode).on_click(cx.listener(
                    move |view, _, _, cx| {
                        view.bastion_decode = decode;
                        cx.notify();
                    },
                ))
            }),
        );
        let panel = setting_group("堡垒机会话链接")
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(space::SM)
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .child(self.bastion_link.clone()),
                    )
                    .child(
                        primary_control("prepare-bastion-link", "打开确认窗口")
                            .h(space::ROW_HEIGHT)
                            .px(space::MD)
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.submit_bastion_link(window, cx)
                            })),
                    ),
            )
            .child(
                setting_row("用户名处理")
                    .h(space::TOOLBAR_HEIGHT)
                    .border_b_0()
                    .child(username_choices),
            )
            .when_some(self.bastion_error.clone(), |form, error| {
                form.child(
                    div()
                        .text_size(text::DETAIL)
                        .text_color(rgb(color::DANGER))
                        .child(error),
                )
            });

        #[cfg(windows)]
        let panel = panel.child(
            div().mt(space::SM).pt(space::MD).border_t_1().border_color(rgb(color::LINE))
                .flex().flex_col().gap(space::SM)
                .child(
                    div().flex().items_center().gap(space::SM)
                        .child(div().flex_1().min_w(px(0.)).text_size(text::HEADING)
                            .font_weight(FontWeight::SEMIBOLD).child("Windows 链接唤起"))
                        .child(setting_action("register-bastion-protocol", "注册链接入口").on_click(
                            cx.listener(|view, _, _, cx| {
                                view.bastion_registration = Some(match crate::windows_launch::register_protocols() {
                                    Ok(()) => "入口已注册，可在 Windows 默认应用中选择 OrbitShell GPUI。".into(),
                                    Err(error) => error,
                                });
                                cx.notify();
                            }),
                        ))
                        .child(setting_action("bastion-default-apps", "系统默认应用")
                            .on_click(|_, _, cx| cx.open_url("ms-settings:defaultapps"))),
                )
                .when_some(self.bastion_registration.clone(), |section, status| {
                    section.child(div().text_size(text::DETAIL).text_color(rgb(color::MUTED)).child(status))
                }),
        );
        panel
    }

    pub(super) fn submit_bastion_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let link = self
            .bastion_link
            .update(cx, |field, cx| field.take_secret(cx));
        let mode = if self.bastion_decode {
            "decodeBase64"
        } else {
            "preserve"
        };
        self.receive_bastion_launch(parse_link(&link, mode, None), window, cx);
    }

    pub(crate) fn receive_bastion_launch(
        &mut self,
        result: Result<PendingLaunch, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut launch = match result {
            Ok(launch) => launch,
            Err(error) => {
                self.open_dialog(Dialog::Settings(SettingsTab::Bastion), window, cx);
                self.bastion_error = Some(error);
                cx.notify();
                return;
            }
        };
        let original_user = std::mem::take(&mut launch.profile.username);
        launch.profile.username = "temporary".into();
        let validated = launch.profile.validate();
        launch.profile.username = original_user;
        if let Err(error) = validated {
            self.open_dialog(Dialog::Settings(SettingsTab::Bastion), window, cx);
            self.bastion_error = Some(error);
            cx.notify();
            return;
        }
        self.clear_bastion_link(cx);
        let group = self.add_group("临时会话", false);
        let mut profile = Profile::from_ssh(launch.profile.clone(), group);
        profile.temporary = true;
        profile.address = if launch.profile.hostname.contains(':') {
            format!("[{}]:{}", launch.profile.hostname, launch.profile.port)
        } else {
            format!("{}:{}", launch.profile.hostname, launch.profile.port)
        };
        let index = self
            .profiles
            .iter()
            .enumerate()
            .find(|(index, profile)| {
                profile.temporary
                    && !self
                        .sessions
                        .iter()
                        .any(|session| session.profile == *index)
            })
            .map(|(index, _)| index);
        let index = if let Some(index) = index {
            self.profiles[index] = profile;
            index
        } else {
            let index = self.profiles.len();
            self.profiles.push(profile);
            index
        };
        let dialog = Dialog::Login(index, LoginOrigin::User);
        if self.dialog == Some(dialog) {
            self.close_dialog(window, cx);
        }
        self.page = Page::Terminal;
        self.open_dialog(dialog, window, cx);
        if launch.profile.username.is_empty() {
            self.bastion_username
                .update(cx, |field, cx| field.set_value("", cx));
            self.bastion_username.read(cx).focus(window);
        }
        if launch
            .credential
            .as_ref()
            .is_some_and(|credential| !credential.is_empty())
        {
            self.login_credential = LoginCredential::Saved;
            if !launch.profile.username.is_empty() {
                self.dialog_focus.focus(window);
            }
        }
        let id = launch.profile.id.clone();
        let remaining = Duration::from_secs(120).saturating_sub(launch.created.elapsed());
        self.pending_bastion = Some(launch);
        cx.spawn_in(window, async move |view, cx| {
            cx.background_executor().timer(remaining).await;
            let _ = view.update_in(cx, |view, window, cx| {
                if view
                    .pending_bastion
                    .as_ref()
                    .is_some_and(|launch| launch.profile.id == id)
                {
                    view.pending_bastion = None;
                    if view.dialog == Some(dialog) {
                        view.open_dialog(Dialog::Settings(SettingsTab::Bastion), window, cx);
                        view.bastion_error = Some("堡垒机链接确认已超时，请重新打开链接".into());
                        cx.notify();
                    }
                }
            });
        })
        .detach();
        cx.notify();
    }
}
