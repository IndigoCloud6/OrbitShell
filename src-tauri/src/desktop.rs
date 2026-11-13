use crate::AppState;
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, sync::{atomic::{AtomicBool, Ordering}, Mutex}};
use tauri::{menu::{Menu, MenuItem, PredefinedMenuItem}, tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent}, AppHandle, Emitter, Manager, State};

const TRAY_ID: &str = "orbitshell-main-tray";
const SHOW_ID: &str = "orbitshell-tray-show";
const HIDE_ID: &str = "orbitshell-tray-hide";
const EXIT_ID: &str = "orbitshell-tray-exit";

#[derive(Default)]
struct ExitGuards { origin: u64, ids: HashSet<String> }
impl ExitGuards {
    fn register(&mut self, id: String, origin: u64) -> Result<(), String> {
        if origin < self.origin { return Err("窗口已刷新，旧页面不能注册退出保护".into()); }
        if origin > self.origin { self.ids.clear(); self.origin = origin; }
        if self.ids.len() >= 64 { return Err("窗口退出保护尚未释放，请稍后重试".into()); }
        self.ids.insert(id); Ok(())
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DesktopPreferences { pub close_to_tray: bool }
impl Default for DesktopPreferences { fn default() -> Self { Self { close_to_tray: true } } }

pub struct DesktopState {
    tray_ready: AtomicBool,
    close_to_tray: AtomicBool,
    pub exiting: AtomicBool,
    exit_pending: AtomicBool,
    // Leases keep a StrictMode/HMR cleanup from unregistering a newer UI guard.
    guards: Mutex<ExitGuards>,
    tray_error: Mutex<Option<String>>,
}
impl DesktopState {
    pub fn new(close_to_tray: bool) -> Self {
        Self { tray_ready: AtomicBool::new(false), close_to_tray: AtomicBool::new(close_to_tray), exiting: AtomicBool::new(false),
            exit_pending: AtomicBool::new(false), guards: Mutex::new(ExitGuards::default()), tray_error: Mutex::new(None) }
    }
    fn status(&self) -> Result<DesktopStatus, String> {
        Ok(DesktopStatus { tray_available: self.tray_ready.load(Ordering::Acquire), close_to_tray: self.close_to_tray.load(Ordering::Acquire),
            error: self.tray_error.lock().map_err(|_| "托盘状态不可用")?.clone() })
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopStatus { tray_available: bool, close_to_tray: bool, error: Option<String> }

pub fn show_main(app: &AppHandle) -> Result<(), String> {
    let window = app.get_webview_window("main").ok_or("主窗口不可用")?;
    window.show().map_err(|_| "无法显示主窗口")?;
    window.unminimize().map_err(|_| "无法还原主窗口")?;
    window.set_focus().map_err(|_| "窗口已显示，请从任务栏切换到 OrbitShell")?;
    if let Some(tray) = app.tray_by_id(TRAY_ID) { let _ = tray.set_tooltip(Some("OrbitShell")); }
    Ok(())
}
pub fn hide_main(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    if !state.desktop.tray_ready.load(Ordering::Acquire) || app.tray_by_id(TRAY_ID).is_none() {
        return Err("系统托盘当前不可用，窗口会保持显示。".into());
    }
    app.get_webview_window("main").ok_or("主窗口不可用")?.hide().map_err(|_| "无法将窗口隐藏到系统托盘")?;
    if let Some(tray) = app.tray_by_id(TRAY_ID) { let _ = tray.set_tooltip(Some("OrbitShell · 正在后台运行")); }
    Ok(())
}
pub fn shutdown(app: &AppHandle) {
    let state = app.state::<AppState>();
    let _ = state.history.flush();
    state.mcp_epoch.fetch_add(1, Ordering::AcqRel);
    state.mcp_enabled.store(false, Ordering::Release);
    state.ai.stop(app);
    if let Ok(sessions) = state.sessions.lock() { for session in sessions.values() { session.cancel.cancel(); } }
    if let Ok(mut runtime) = state.mcp_runtime.try_lock() { if let Some(server) = runtime.take() { server.stop(); } };
}
fn exit_now(app: &AppHandle) {
    let state = app.state::<AppState>();
    if state.desktop.exiting.swap(true, Ordering::AcqRel) { return; }
    shutdown(app);
    app.exit(0);
}
pub fn request_exit(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    if state.desktop.exiting.load(Ordering::Acquire) { return Ok(()); }
    // Show before notifying the renderer so a hidden window can present its draft dialog.
    let _ = show_main(app);
    let guarded = !state.desktop.guards.lock().map_err(|_| "退出保护状态不可用")?.ids.is_empty();
    if !guarded { exit_now(app); return Ok(()); }
    if state.desktop.exit_pending.swap(true, Ordering::AcqRel) { return Ok(()); }
    if app.emit_to("main", "orbitshell:request-exit", ()).is_err() {
        state.desktop.exit_pending.store(false, Ordering::Release);
        return Err("无法通知窗口处理未保存文件，请在主窗口重试退出。".into());
    }
    Ok(())
}
pub fn close_main(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    if state.desktop.close_to_tray.load(Ordering::Acquire) && state.desktop.tray_ready.load(Ordering::Acquire) {
        hide_main(app)
    } else { request_exit(app) }
}
fn build_tray(app: &AppHandle) -> anyhow::Result<()> {
    let show = MenuItem::with_id(app, SHOW_ID, "显示窗口", true, None::<&str>)?;
    let hide = MenuItem::with_id(app, HIDE_ID, "后台运行", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let exit = MenuItem::with_id(app, EXIT_ID, "退出 OrbitShell", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &hide, &separator, &exit])?;
    let icon = app.default_window_icon().ok_or_else(|| anyhow::anyhow!("应用图标不可用"))?;
    let builder = TrayIconBuilder::with_id(TRAY_ID).icon(icon.clone()).tooltip("OrbitShell").menu(&menu).show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            let _ = match event.id.as_ref() {
                SHOW_ID => show_main(app), HIDE_ID => hide_main(app), EXIT_ID => request_exit(app), _ => Ok(()),
            };
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(event, TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } | TrayIconEvent::DoubleClick { button: MouseButton::Left, .. }) {
                let _ = show_main(tray.app_handle());
            }
        });
    builder.build(app)?;
    Ok(())
}
pub fn install(app: &AppHandle) {
    let state = app.state::<AppState>();
    match build_tray(app) {
        Ok(()) => state.desktop.tray_ready.store(true, Ordering::Release),
        Err(_) => {
            state.desktop.tray_ready.store(false, Ordering::Release);
            if let Ok(mut error) = state.desktop.tray_error.lock() {
                *error = Some("系统托盘初始化失败。窗口会保持可访问，关闭窗口时将按退出流程处理。".into());
            }
        }
    }
}

pub fn renderer_loading(app: &AppHandle) {
    if let Some(state) = app.try_state::<AppState>() {
        if let Ok(mut guards) = state.desktop.guards.lock() { guards.ids.clear(); }
        state.desktop.exit_pending.store(false, Ordering::Release);
        state.editor.clear_renderer_files();
    }
}

#[tauri::command]
pub fn desktop_status(state: State<'_, AppState>) -> Result<DesktopStatus, String> { state.desktop.status() }
#[tauri::command]
pub fn desktop_set_close_to_tray(enabled: bool, state: State<'_, AppState>) -> Result<DesktopStatus, String> {
    if enabled && !state.desktop.tray_ready.load(Ordering::Acquire) { return Err("系统托盘不可用，暂不能启用关闭窗口后后台运行。".into()); }
    let mut store = state.storage.store.lock().map_err(|_| "设置暂不可用")?;
    let mut next = store.clone(); next.desktop.close_to_tray = enabled;
    state.storage.persist(&next).map_err(|_| "窗口行为未保存，请重试")?;
    *store = next;
    state.desktop.close_to_tray.store(enabled, Ordering::Release);
    state.desktop.status()
}
#[tauri::command]
pub fn desktop_close_window(app: AppHandle) -> Result<(), String> { close_main(&app) }
#[tauri::command]
pub fn desktop_hide_window(app: AppHandle) -> Result<(), String> { hide_main(&app) }
#[tauri::command]
pub fn desktop_request_exit(app: AppHandle) -> Result<(), String> { request_exit(&app) }
#[tauri::command]
pub fn desktop_confirm_exit(app: AppHandle) -> Result<(), String> {
    if !app.state::<AppState>().desktop.exit_pending.swap(false, Ordering::AcqRel) { return Err("当前没有待确认的退出请求".into()); }
    exit_now(&app); Ok(())
}
#[tauri::command]
pub fn desktop_cancel_exit(state: State<'_, AppState>) {
    state.desktop.exit_pending.store(false, Ordering::Release);
}
#[tauri::command]
pub fn desktop_register_exit_guard(id: String, origin: u64, state: State<'_, AppState>) -> Result<(), String> {
    uuid::Uuid::parse_str(&id).map_err(|_| "窗口退出保护标识无效")?;
    let mut guards = state.desktop.guards.lock().map_err(|_| "退出保护状态不可用")?;
    if origin > guards.origin { state.desktop.exit_pending.store(false, Ordering::Release); }
    guards.register(id, origin)
}
#[tauri::command]
pub fn desktop_unregister_exit_guard(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let mut guards = state.desktop.guards.lock().map_err(|_| "退出保护状态不可用")?;
    guards.ids.remove(&id);
    if guards.ids.is_empty() { state.desktop.exit_pending.store(false, Ordering::Release); }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn older_renderer_cannot_leave_a_stale_exit_guard_after_reload() {
        let mut guards = ExitGuards::default();
        guards.register("old".into(), 100).unwrap();
        guards.register("new".into(), 200).unwrap();
        assert!(!guards.ids.contains("old"));
        assert!(guards.register("late-old".into(), 100).is_err());
        assert_eq!(guards.ids.len(), 1);
        assert!(guards.ids.contains("new"));
    }
    #[test]
    fn strict_mode_cleanup_does_not_unregister_the_live_guard() {
        let mut guards = ExitGuards::default();
        guards.register("first-mount".into(), 100).unwrap();
        guards.register("second-mount".into(), 100).unwrap();
        guards.ids.remove("first-mount");
        assert_eq!(guards.ids.len(), 1);
        assert!(guards.ids.contains("second-mount"));
    }
    #[test]
    fn existing_workspace_defaults_to_background_and_preserves_explicit_exit_choice() {
        let legacy: crate::storage::Store = serde_json::from_str(r#"{"connections":[],"known_hosts":{}}"#).unwrap();
        assert!(legacy.desktop.close_to_tray);
        let preference: DesktopPreferences = serde_json::from_str(r#"{"closeToTray":false}"#).unwrap();
        assert!(!preference.close_to_tray);
    }
}
