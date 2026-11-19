#![cfg_attr(windows, windows_subsystem = "windows")]

mod demo;
mod editor_format;
mod editor_input;
mod editor_search;
mod editor_syntax;
mod file_icons;
mod icons;
mod mcp;
mod mcp_terminal;
mod ssh_backend;
mod ssh_keys;
mod terminal;
mod terminal_view;
mod text_field;
mod theme;
mod transfer_queue;
mod viewport;
mod window_frame;
#[cfg(windows)]
mod windows_launch;
#[cfg(windows)]
mod windows_tray;
mod workbench;

use std::borrow::Cow;

use file_icons::MaterialAssets;
use gpui::{
    Application, AssetSource, Bounds, SharedString, TitlebarOptions, WindowBounds, WindowOptions,
    prelude::*, px, size,
};
use icons::LucideAssets;
use workbench::Workbench;

const BRAND_IMAGE: &str = "orbitshell.svg";

struct WorkbenchAssets;

impl AssetSource for WorkbenchAssets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        if path == BRAND_IMAGE {
            return Ok(Some(Cow::Borrowed(include_bytes!(
                "../../../assets/orbitshell.svg"
            ))));
        }
        Ok(MaterialAssets::get(path)
            .or_else(|| LucideAssets::get(path))
            .map(|asset| asset.data))
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(std::iter::once(Cow::Borrowed(BRAND_IMAGE))
            .chain(MaterialAssets::iter())
            .chain(LucideAssets::iter())
            .filter(|name| name.starts_with(path))
            .map(|name| name.into_owned().into())
            .collect())
    }
}

fn main() {
    let arguments = zeroize::Zeroizing::new(
        std::env::args_os()
            .skip(1)
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
    );
    let launch = orbitshell_ssh::launch::first_argument_link(&arguments);
    #[cfg(windows)]
    if launch.is_some() && windows_launch::forward(&arguments) {
        return;
    }
    drop(arguments);
    Application::new()
        .with_assets(WorkbenchAssets)
        .run(move |cx| {
            let bounds = Bounds::centered(None, size(px(1280.), px(780.)), cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(960.), px(490.))),
                titlebar: Some(TitlebarOptions {
                    title: Some("OrbitShell · GPUI 工作台".into()),
                    appears_transparent: true,
                    ..TitlebarOptions::default()
                }),
                ..WindowOptions::default()
            };
            if let Err(error) = cx.open_window(options, move |window, cx| {
                cx.new(|cx| {
                    let mut workbench = Workbench::new(window, cx);
                    workbench.install_desktop(window, cx);
                    if let Some(launch) = launch {
                        workbench.receive_bastion_launch(launch, window, cx);
                    }
                    workbench
                })
            }) {
                eprintln!("无法打开 GPUI 工作台窗口：{error:#}");
                cx.quit();
            }
        });
}
