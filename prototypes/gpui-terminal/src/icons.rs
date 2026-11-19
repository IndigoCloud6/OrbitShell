use gpui::{Context, Svg, Window, div, prelude::*, svg};
use rust_embed::RustEmbed;

use crate::theme::{ThemeColor, rgb};

#[derive(RustEmbed)]
#[folder = "assets/lucide/"]
#[prefix = "lucide/"]
#[include = "*.svg"]
pub struct LucideAssets;

#[derive(Clone, Copy)]
pub enum Icon {
    Connections,
    Files,
    Tools,
    Terminal,
    Bot,
    Search,
    History,
    Settings,
    Close,
    Minimize,
    Maximize,
    Restore,
    ArrowUp,
    House,
    FolderSearch,
    Refresh,
    Eye,
    EyeOff,
    Pencil,
    ChevronRight,
    Plus,
    Upload,
    Download,
    Copy,
    Trash,
}

impl Icon {
    fn path(self) -> &'static str {
        match self {
            Self::Connections => "lucide/server.svg",
            Self::Files => "lucide/folder.svg",
            Self::Tools => "lucide/panel-bottom.svg",
            Self::Terminal => "lucide/terminal.svg",
            Self::Bot => "lucide/bot.svg",
            Self::Search => "lucide/search.svg",
            Self::History => "lucide/history.svg",
            Self::Settings => "lucide/settings.svg",
            Self::Close => "lucide/x.svg",
            Self::Minimize => "lucide/minus.svg",
            Self::Maximize => "lucide/square.svg",
            Self::Restore | Self::Copy => "lucide/copy.svg",
            Self::ArrowUp => "lucide/arrow-up.svg",
            Self::House => "lucide/house.svg",
            Self::FolderSearch => "lucide/folder-search.svg",
            Self::Refresh => "lucide/refresh-cw.svg",
            Self::Eye => "lucide/eye.svg",
            Self::EyeOff => "lucide/eye-off.svg",
            Self::Pencil => "lucide/pencil.svg",
            Self::ChevronRight => "lucide/chevron-right.svg",
            Self::Plus => "lucide/plus.svg",
            Self::Upload => "lucide/upload.svg",
            Self::Download => "lucide/download.svg",
            Self::Trash => "lucide/trash-2.svg",
        }
    }
}

pub fn icon(kind: Icon, tint: ThemeColor) -> Svg {
    svg()
        .path(kind.path())
        .text_color(rgb(tint))
        .size(crate::theme::space::ICON_SIZE)
        .flex_none()
}

pub struct Tooltip(pub &'static str);

impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        use crate::theme::{color, space, text};

        div()
            .px(space::SM)
            .py(space::XS)
            .rounded(space::CONTROL_RADIUS)
            .border_1()
            .border_color(rgb(color::LINE))
            .bg(rgb(color::SURFACE))
            .text_size(text::DETAIL)
            .text_color(rgb(color::TEXT))
            .child(self.0)
    }
}
