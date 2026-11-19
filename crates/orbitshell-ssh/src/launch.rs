use crate::ConnectionProfile;
use base64::{
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD},
    Engine,
};
use percent_encoding::percent_decode_str;
use serde::Serialize;
use std::time::Instant;
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
        if let Some(credential) = self.credential.as_mut() {
            credential.zeroize();
        }
    }
}

pub fn strip_wrapping_quotes(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|inner| inner.strip_suffix('"'))
        .unwrap_or(value)
}

pub fn first_argument_link(arguments: &[String]) -> Option<Result<PendingLaunch, String>> {
    let title = arguments
        .iter()
        .position(|argument| argument.eq_ignore_ascii_case("-newtab"))
        .and_then(|index| arguments.get(index + 1))
        .filter(|value| !value.starts_with('-') && !value.contains("://"))
        .map(String::as_str);
    arguments.iter().find_map(|argument| {
        let value = strip_wrapping_quotes(argument.trim());
        let (scheme, _) = value.split_once("://")?;
        ["ssh", "xshell", "orbitshell"]
            .iter()
            .any(|supported| scheme.eq_ignore_ascii_case(supported))
            .then(|| parse_link(value, "preserve", title))
    })
}

fn decode(value: &str) -> Result<String, String> {
    // Pasted launcher URLs can retain field quotes that Windows normally removes from argv.
    // Unwrap before decoding so percent-encoded quotes remain part of the credential.
    let value = strip_wrapping_quotes(value);
    percent_decode_str(value)
        .decode_utf8()
        .map(|value| value.into_owned())
        .map_err(|_| "链接中的字符编码无效".into())
}

pub fn parse_link(
    value: &str,
    username_mode: &str,
    title: Option<&str>,
) -> Result<PendingLaunch, String> {
    let value = strip_wrapping_quotes(value.trim());
    if value.len() > 32 * 1024 {
        return Err("会话链接过长".into());
    }
    let (scheme, body) = value.split_once("://").ok_or("请使用 SSH 会话链接")?;
    let scheme = scheme.to_ascii_lowercase();
    if !matches!(scheme.as_str(), "ssh" | "xshell" | "orbitshell") {
        return Err("只支持 SSH 会话链接".into());
    }
    if scheme == "orbitshell" && body.starts_with("connect?") {
        let wrapper = url::Url::parse(value).map_err(|_| "OrbitShell 链接格式无效")?;
        let nested = wrapper
            .query_pairs()
            .find(|(name, _)| name == "url")
            .map(|(_, value)| value.into_owned())
            .ok_or("链接缺少 SSH 地址")?;
        return parse_link(&nested, username_mode, title);
    }
    // Split the final @ ourselves: bastion user strings can contain @, /, + and >.
    // Parsing the full string as a generic URL can rewrite these authentication bytes.
    let (userinfo, authority) = match body.rsplit_once('@') {
        Some((user, authority)) => (Some(user), authority),
        None => (None, body),
    };
    let destination =
        url::Url::parse(&format!("ssh://{authority}")).map_err(|_| "服务器地址或端口无效")?;
    if !matches!(destination.path(), "" | "/")
        || destination.query().is_some()
        || destination.fragment().is_some()
    {
        return Err("会话链接只能包含连接信息".into());
    }
    let hostname = destination
        .host_str()
        .ok_or("链接缺少服务器地址")?
        .trim_matches(['[', ']'])
        .to_owned();
    let port = destination.port().unwrap_or(22);
    if port == 0 {
        return Err("端口必须在 1–65535 之间".into());
    }
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
            let bytes = STANDARD
                .decode(encoded)
                .or_else(|_| STANDARD_NO_PAD.decode(encoded))
                .or_else(|_| URL_SAFE.decode(encoded))
                .or_else(|_| URL_SAFE_NO_PAD.decode(encoded))
                .map_err(|_| "b64>> 用户名编码无效；可改为原样发送")?;
            String::from_utf8(bytes).map_err(|_| "解码后的用户名不是有效的 UTF-8")?
        } else {
            user
        }
    } else if username_mode == "preserve" {
        user
    } else {
        return Err("不支持的用户名处理方式".into());
    };
    if username.len() > 4096 || username.chars().any(char::is_control) {
        return Err("链接中的用户名无效".into());
    }
    let name = title
        .map(strip_wrapping_quotes)
        .filter(|title| !title.trim().is_empty())
        .map(|title| {
            title
                .chars()
                .filter(|character| !character.is_control())
                .take(80)
                .collect()
        })
        .unwrap_or_else(|| {
            if username.starts_with("b64>>") {
                format!("堡垒机 · {hostname}")
            } else {
                format!("{hostname}:{port}")
            }
        });
    Ok(PendingLaunch {
        profile: ConnectionProfile {
            id: Uuid::new_v4().to_string(),
            name,
            group: "临时会话".into(),
            hostname,
            port,
            username,
            auth_method: "password".into(),
            private_key_path: None,
        },
        credential: credential.take(),
        created: Instant::now(),
    })
}
