use crate::AppState;
use serde::Serialize;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};
use zeroize::Zeroizing;
pub use orbitshell_ssh::launch::{PendingLaunch, parse_link};
use orbitshell_ssh::launch::strip_wrapping_quotes;
pub fn enqueue(app: &AppHandle, launch: PendingLaunch) -> Result<(), String> {
    let state = app.state::<AppState>();
    let mut queue = state.launches.lock().map_err(|_| "启动队列暂不可用")?;
    queue.retain(|entry| entry.created.elapsed() < Duration::from_secs(120));
    if queue.len() >= 16 { return Err("等待打开的会话过多，请先处理现有会话".into()); }
    queue.push(launch);
    drop(queue);
    let _ = crate::desktop::show_main(app);
    app.emit_to("main", "orbitshell:launch-available", ()).map_err(|_| "无法通知工作台".into())
}

pub fn ingest_arguments(app: &AppHandle, arguments: Vec<String>) {
    let arguments = Zeroizing::new(arguments);
    let title = arguments.iter().position(|argument| argument.eq_ignore_ascii_case("-newtab"))
        .and_then(|index| arguments.get(index + 1)).filter(|value| !value.starts_with('-') && !value.contains("://")).map(String::as_str);
    for argument in arguments.iter() {
        let argument = strip_wrapping_quotes(argument.trim());
        let Some((scheme, _)) = argument.split_once("://") else { continue; };
        if scheme.eq_ignore_ascii_case("ssh") || scheme.eq_ignore_ascii_case("xshell") || scheme.eq_ignore_ascii_case("orbitshell") {
            if let Err(error) = parse_link(argument, "preserve", title).and_then(|launch| enqueue(app, launch)) {
                let _ = app.emit_to("main", "orbitshell:launch-error", error);
            }
        }
    }
    let _ = crate::desktop::show_main(app);
}

#[tauri::command]
pub fn open_external_link(url: String, username_mode: String, app: AppHandle) -> Result<(), String> {
    let url = Zeroizing::new(url);
    enqueue(&app, parse_link(url.as_str(), &username_mode, None)?)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuickConnectionResult { profile_id: Option<String> }

#[tauri::command]
pub fn open_quick_connection(value: String, app: AppHandle) -> Result<QuickConnectionResult, String> {
    let input = Zeroizing::new(value);
    let value = strip_wrapping_quotes(input.trim());
    if value.is_empty() || value.len() > 32 * 1024 || value.chars().any(char::is_control) {
        return Err("请输入有效的主机地址或 SSH 会话链接".into());
    }
    let is_link = value.contains("://");
    let mut normalized = Zeroizing::new(String::new());
    if is_link { normalized.push_str(value); } else {
        let (username, address) = value.rsplit_once('@').map(|(user, host)| (Some(user), host)).unwrap_or((None, value));
        normalized.push_str("ssh://");
        if let Some(user) = username { normalized.push_str(user); normalized.push('@'); }
        if address.parse::<std::net::Ipv6Addr>().is_ok() {
            normalized.push('['); normalized.push_str(address); normalized.push(']');
        } else { normalized.push_str(address); }
    }
    let mut launch = parse_link(normalized.as_str(), "preserve", None)?;
    let ordinary_target = !is_link && launch.credential.is_none() && !launch.profile.username.starts_with("b64>>");
    if ordinary_target {
        let state = app.state::<AppState>();
        let store = state.storage.lock().map_err(|_| "连接配置暂不可用")?;
        let mut matches = store.connections.iter().filter(|profile| profile.hostname.eq_ignore_ascii_case(&launch.profile.hostname)
            && profile.port == launch.profile.port && (launch.profile.username.is_empty() || profile.username == launch.profile.username));
        let first = matches.next();
        if let (Some(profile), None) = (first, matches.next()) {
            return Ok(QuickConnectionResult { profile_id: Some(profile.id.clone()) });
        }
    }
    if ordinary_target { launch.profile.group = "快速连接".into(); }
    enqueue(&app, launch)?;
    Ok(QuickConnectionResult { profile_id: None })
}

#[tauri::command]
pub fn register_ssh_protocol(app: AppHandle) -> Result<String, String> {
    use tauri_plugin_deep_link::DeepLinkExt;
    app.deep_link().register("ssh").map_err(|_| "无法注册 SSH 链接入口".to_owned())?;
    Ok("SSH 链接入口已注册，请在系统默认应用设置中选择 OrbitShell。".into())
}

#[tauri::command]
pub fn take_launch_requests(state: State<'_, AppState>) -> Result<Vec<PendingLaunch>, String> {
    let mut queue = state.launches.lock().map_err(|_| "启动队列暂不可用")?;
    queue.retain(|entry| entry.created.elapsed() < Duration::from_secs(120));
    Ok(std::mem::take(&mut *queue))
}
