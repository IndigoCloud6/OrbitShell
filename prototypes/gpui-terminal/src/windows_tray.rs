use gpui::Window;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use tokio::sync::mpsc::UnboundedSender;
use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
};
use windows_sys::Win32::{
    Foundation::HWND,
    UI::WindowsAndMessaging::{
        IsIconic, IsWindowVisible, SW_HIDE, SW_RESTORE, SW_SHOW, ShowWindowAsync,
    },
};

use crate::workbench::DesktopAction;

const TRAY_ID: &str = "orbitshell-main-tray";
const SHOW: &str = "orbitshell-tray-show";
const HIDE: &str = "orbitshell-tray-hide";
const EXIT: &str = "orbitshell-tray-exit";
const TITLE: &str = "OrbitShell";
const BACKGROUND: &str = "OrbitShell · 正在后台运行";

pub(crate) struct Tray(TrayIcon);

impl Tray {
    pub(crate) fn new(actions: UnboundedSender<DesktopAction>) -> anyhow::Result<Self> {
        let asset = include_bytes!("../../../src-tauri/icons/32x32.png");
        let image =
            image::load_from_memory_with_format(asset, image::ImageFormat::Png)?.into_rgba8();
        let (width, height) = image.dimensions();
        let icon = Icon::from_rgba(image.into_raw(), width, height)?;
        let show = MenuItem::with_id(SHOW, "显示窗口", true, None);
        let hide = MenuItem::with_id(HIDE, "后台运行", true, None);
        let exit = MenuItem::with_id(EXIT, "退出 OrbitShell", true, None);
        let menu = Menu::with_items(&[&show, &hide, &PredefinedMenuItem::separator(), &exit])?;
        let icon = TrayIconBuilder::new()
            .with_id(TRAY_ID)
            .with_icon(icon)
            .with_menu(Box::new(menu))
            .with_tooltip(TITLE)
            .with_menu_on_left_click(false)
            .build()?;
        let tray_actions = actions.clone();
        TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
            if event.id().as_ref() == TRAY_ID
                && matches!(
                    event,
                    TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } | TrayIconEvent::DoubleClick {
                        button: MouseButton::Left,
                        ..
                    }
                )
            {
                let _ = tray_actions.send(DesktopAction::Show);
            }
        }));
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let action = match event.id.as_ref() {
                SHOW => DesktopAction::Show,
                HIDE => DesktopAction::Hide,
                EXIT => DesktopAction::Exit,
                _ => return,
            };
            let _ = actions.send(action);
        }));
        Ok(Self(icon))
    }

    pub(crate) fn hide(&self, window: &Window) -> Result<(), &'static str> {
        let hwnd = hwnd(window).ok_or("当前窗口无法隐藏到系统托盘。")?;
        // The tray library can keep a handle while Explorer is starting.
        // Verify the shell accepted the icon update before hiding the window.
        self.0
            .set_tooltip(Some(BACKGROUND))
            .map_err(|_| "系统托盘暂不可用，窗口已保留。")?;
        // SAFETY: the handle belongs to the live GPUI window on this thread.
        unsafe {
            if IsWindowVisible(hwnd) != 0 && ShowWindowAsync(hwnd, SW_HIDE) == 0 {
                let _ = self.0.set_tooltip(Some(TITLE));
                return Err("无法将窗口隐藏到系统托盘。");
            }
        }
        Ok(())
    }

    pub(crate) fn reset_tooltip(&self) {
        let _ = self.0.set_tooltip(Some(TITLE));
    }
}

pub(crate) fn show(window: &Window) {
    if let Some(hwnd) = hwnd(window) {
        // SAFETY: the handle belongs to the live GPUI window on this thread.
        unsafe {
            if IsIconic(hwnd) != 0 {
                ShowWindowAsync(hwnd, SW_RESTORE);
            } else if IsWindowVisible(hwnd) == 0 {
                ShowWindowAsync(hwnd, SW_SHOW);
            }
        }
    }
    window.activate_window();
}

fn hwnd(window: &Window) -> Option<HWND> {
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get() as HWND),
        _ => None,
    }
}
