use gpui::{Div, Window, WindowControlArea, prelude::*};

#[derive(Clone, Copy)]
pub enum DragArea {
    Brand,
    Space,
}

pub struct WindowFrame {
    #[cfg(windows)]
    native: Option<windows::HitTest>,
}

impl WindowFrame {
    pub fn new(window: &Window) -> Self {
        #[cfg(windows)]
        {
            let native = windows::HitTest::install(window);
            if native.is_none() {
                eprintln!("无法启用 Windows 自定义窗口边框，使用 GPUI 的拖动区域。");
            }
            Self { native }
        }
        #[cfg(not(windows))]
        {
            let _ = window;
            Self {}
        }
    }

    pub fn drag_area(&self, area: DragArea, content: Div) -> Div {
        #[cfg(windows)]
        if let Some(native) = &self.native {
            return native.drag_area(area, content);
        }
        let _ = area;
        content.window_control_area(WindowControlArea::Drag)
    }
}

pub fn toggle_maximize(window: &Window) {
    #[cfg(windows)]
    if windows::toggle_maximize(window) {
        return;
    }
    window.zoom_window();
}

#[cfg(windows)]
mod windows {
    use std::{cell::Cell, rc::Rc};

    use gpui::{Bounds, Div, Pixels, Window, canvas, point, prelude::*, px};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::Gdi::ScreenToClient,
        UI::{
            HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi},
            Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
            WindowsAndMessaging::{
                DefWindowProcW, GetWindowRect, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTCAPTION,
                HTCLIENT, HTLEFT, HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT, IsZoomed,
                SM_CXPADDEDBORDER, SM_CXSIZEFRAME, SM_CYSIZEFRAME, SW_MAXIMIZE, SW_RESTORE,
                ShowWindowAsync, WM_NCDESTROY, WM_NCHITTEST, WM_NCLBUTTONDBLCLK, WM_NCLBUTTONDOWN,
            },
        },
    };

    use super::DragArea;

    const SUBCLASS_ID: usize = 1;
    const BASE_DPI: u32 = 96;

    #[derive(Default)]
    struct DragRegions([Cell<Option<Bounds<Pixels>>>; 2]);

    pub(super) struct HitTest(Rc<DragRegions>);

    impl HitTest {
        pub(super) fn install(window: &Window) -> Option<Self> {
            let hwnd = hwnd(window)?;
            let regions = Rc::new(DragRegions::default());
            let state = Rc::into_raw(Rc::clone(&regions));
            // GPUI opens this window on its UI thread. The callback owns this reference until WM_NCDESTROY.
            if unsafe { SetWindowSubclass(hwnd, Some(frame_proc), SUBCLASS_ID, state as usize) }
                == 0
            {
                // SAFETY: installation failed, so no callback can own or access this reference.
                unsafe { drop(Rc::from_raw(state)) };
                return None;
            }
            Some(Self(regions))
        }

        pub(super) fn drag_area(&self, area: DragArea, content: Div) -> Div {
            let regions = Rc::clone(&self.0);
            content.relative().child(
                canvas(
                    move |bounds, _, _| regions.0[area as usize].set(Some(bounds)),
                    |_, (), _, _| {},
                )
                .absolute()
                .inset_0(),
            )
        }
    }

    pub(super) fn toggle_maximize(window: &Window) -> bool {
        let Some(hwnd) = hwnd(window) else {
            return false;
        };
        // SAFETY: the live GPUI Window owns this handle throughout the call.
        unsafe {
            let command = if IsZoomed(hwnd) != 0 {
                SW_RESTORE
            } else {
                SW_MAXIMIZE
            };
            ShowWindowAsync(hwnd, command) != 0
        }
    }

    fn hwnd(window: &Window) -> Option<HWND> {
        match HasWindowHandle::window_handle(window).ok()?.as_raw() {
            RawWindowHandle::Win32(handle) => Some(handle.hwnd.get() as HWND),
            _ => None,
        }
    }

    unsafe extern "system" fn frame_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        id: usize,
        data: usize,
    ) -> LRESULT {
        let state = data as *const DragRegions;
        match message {
            WM_NCDESTROY => {
                // SAFETY: remove this subclass on its owning thread before releasing its state.
                unsafe {
                    RemoveWindowSubclass(hwnd, Some(frame_proc), id);
                    drop(Rc::from_raw(state));
                }
            }
            WM_NCHITTEST => {
                // SAFETY: the subclass retains a strong reference until WM_NCDESTROY.
                if let Some(hit) = hit_test(hwnd, lparam, unsafe { &*state }) {
                    return hit as LRESULT;
                }
            }
            WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK
                if matches!(
                    wparam as u32,
                    HTCAPTION
                        | HTLEFT
                        | HTRIGHT
                        | HTTOP
                        | HTTOPLEFT
                        | HTTOPRIGHT
                        | HTBOTTOM
                        | HTBOTTOMLEFT
                        | HTBOTTOMRIGHT
                ) =>
            {
                // Send non-client drag and resize gestures directly to the system window manager.
                return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
            }
            _ => {}
        }
        // SAFETY: these are the parameters of the current message in the subclass chain.
        unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
    }

    fn hit_test(hwnd: HWND, lparam: LPARAM, regions: &DragRegions) -> Option<u32> {
        let mut mouse = POINT {
            x: (lparam as i16) as i32,
            y: ((lparam >> 16) as i16) as i32,
        };
        let mut rect = RECT::default();
        // SAFETY: the HWND is the live window receiving this message, and the output pointers are valid.
        unsafe {
            if GetWindowRect(hwnd, &mut rect) == 0
                || !(rect.left..rect.right).contains(&mouse.x)
                || !(rect.top..rect.bottom).contains(&mouse.y)
            {
                return None;
            }

            let dpi = match GetDpiForWindow(hwnd) {
                0 => BASE_DPI,
                dpi => dpi,
            };
            if IsZoomed(hwnd) == 0 {
                let padding = GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi);
                // Include the visible edge beside GPUI's inset client area.
                let grip = (3 * dpi).div_ceil(BASE_DPI) as i32;
                let width = GetSystemMetricsForDpi(SM_CXSIZEFRAME, dpi) + padding + grip;
                let height = GetSystemMetricsForDpi(SM_CYSIZEFRAME, dpi) + padding + grip;
                let column = edge_slot(mouse.x, rect.left, rect.right, width);
                let row = edge_slot(mouse.y, rect.top, rect.bottom, height);
                let edge = [
                    [HTTOPLEFT, HTTOP, HTTOPRIGHT],
                    [HTLEFT, HTCLIENT, HTRIGHT],
                    [HTBOTTOMLEFT, HTBOTTOM, HTBOTTOMRIGHT],
                ][row][column];
                if edge != HTCLIENT {
                    return Some(edge);
                }
            }

            if ScreenToClient(hwnd, &mut mouse) == 0 {
                return None;
            }
            let scale = BASE_DPI as f32 / dpi as f32;
            let point = point(px(mouse.x as f32 * scale), px(mouse.y as f32 * scale));
            let on_title = regions
                .0
                .iter()
                .filter_map(Cell::get)
                .any(|bounds| bounds.contains(&point));
            Some(if on_title { HTCAPTION } else { HTCLIENT })
        }
    }

    fn edge_slot(point: i32, from: i32, to: i32, thickness: i32) -> usize {
        if point < from + thickness {
            0
        } else if point >= to - thickness {
            2
        } else {
            1
        }
    }
}
