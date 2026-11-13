use crate::{storage::ConnectionProfile, AppState};
use base64::{engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD}, Engine};
use percent_encoding::percent_decode_str;
use serde::Serialize;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, State};
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingLaunch {
    pub profile: ConnectionProfile,
    pub credential: Option<String>,
    #[serde(skip)]
    pub created: Instant,
}
impl Drop for PendingLaunch {
    fn drop(&mut self) {
        if let Some(credential) = self.credential.as_mut() { credential.zeroize(); }
    }
}

fn strip_wrapping_quotes(value: &str) -> &str {
    value.strip_prefix('"').and_then(|inner| inner.strip_suffix('"')).unwrap_or(value)
}

fn decode(value: &str) -> Result<String, String> {
    // Pasted launcher URLs can retain field quotes that Windows normally removes from argv.
    // Unwrap before decoding so percent-encoded quotes remain part of the credential.
    let value = strip_wrapping_quotes(value);
    percent_decode_str(value).decode_utf8().map(|value| value.into_owned()).map_err(|_| "链接中的字符编码无效".into())
}

pub fn parse_link(value: &str, username_mode: &str, title: Option<&str>) -> Result<PendingLaunch, String> {
    let value = strip_wrapping_quotes(value.trim());
    if value.len() > 32 * 1024 { return Err("会话链接过长".into()); }
    let (scheme, body) = value.split_once("://").ok_or("请使用 SSH 会话链接")?;
    let scheme = scheme.to_ascii_lowercase();
    if !matches!(scheme.as_str(), "ssh" | "xshell" | "orbitshell") { return Err("只支持 SSH 会话链接".into()); }
    if scheme == "orbitshell" && body.starts_with("connect?") {
        let wrapper = url::Url::parse(value).map_err(|_| "OrbitShell 链接格式无效")?;
        let nested = wrapper.query_pairs().find(|(name, _)| name == "url").map(|(_, value)| value.into_owned()).ok_or("链接缺少 SSH 地址")?;
        return parse_link(&nested, username_mode, title);
    }
    // Split the final @ ourselves: bastion user strings can contain @, /, + and >.
    // Parsing the full string as a generic URL can rewrite these authentication bytes.
    let (userinfo, authority) = match body.rsplit_once('@') {
        Some((user, authority)) => (Some(user), authority),
        None => (None, body),
    };
    let destination = url::Url::parse(&format!("ssh://{authority}")).map_err(|_| "服务器地址或端口无效")?;
    if !matches!(destination.path(), "" | "/") || destination.query().is_some() || destination.fragment().is_some() {
        return Err("会话链接只能包含连接信息".into());
    }
    let hostname = destination.host_str().ok_or("链接缺少服务器地址")?.trim_matches(['[', ']']).to_owned();
    let port = destination.port().unwrap_or(22);
    if port == 0 { return Err("端口必须在 1–65535 之间".into()); }
    let (user, credential) = match userinfo {
        Some(userinfo) => match userinfo.split_once(':') {
            Some((user, password)) => (decode(user)?, Some(decode(password)?)),
            None => (decode(userinfo)?, None),
        },
        None => (String::new(), None),
    };
    let mut credential = Zeroizing::new(credential);
    let username = if username_mode == "decodeBase64" {
        if let Some(encoded) = user.strip_prefix("b64>>") {
            let bytes = STANDARD.decode(encoded).or_else(|_| STANDARD_NO_PAD.decode(encoded))
                .or_else(|_| URL_SAFE.decode(encoded)).or_else(|_| URL_SAFE_NO_PAD.decode(encoded))
                .map_err(|_| "b64>> 用户名编码无效；可改为原样发送")?;
            String::from_utf8(bytes).map_err(|_| "解码后的用户名不是有效的 UTF-8")?
        } else { user }
    } else if username_mode == "preserve" { user }
    else { return Err("不支持的用户名处理方式".into()); };
    if username.len() > 4096 || username.chars().any(char::is_control) { return Err("链接中的用户名无效".into()); }
    let name = title.map(strip_wrapping_quotes).filter(|title| !title.trim().is_empty()).map(|title| title.chars().filter(|character| !character.is_control()).take(80).collect())
        .unwrap_or_else(|| if username.starts_with("b64>>") { format!("堡垒机 · {hostname}") } else { format!("{hostname}:{port}") });
    Ok(PendingLaunch {
        profile: ConnectionProfile { id: Uuid::new_v4().to_string(), name, group: "临时会话".into(),
            hostname, port, username, auth_method: "password".into(), private_key_path: None },
        credential: credential.take(), created: Instant::now(),
    })
}

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
        let store = state.storage.store.lock().map_err(|_| "连接配置暂不可用")?;
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
