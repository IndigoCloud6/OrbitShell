//! OS-owned SSH credentials. Secrets are never part of workspace.json or IPC responses.
use crate::storage::ConnectionProfile;
use zeroize::Zeroizing;

pub const AVAILABLE: bool = cfg!(windows);

pub fn same_identity(left: &ConnectionProfile, right: &ConnectionProfile) -> bool {
    left.id == right.id && left.hostname == right.hostname && left.port == right.port &&
        left.username == right.username && left.auth_method == right.auth_method &&
        left.private_key_path == right.private_key_path
}

#[cfg(windows)]
mod native {
    use super::*;
    use keyring::{Entry, Error};
    use serde::{Deserialize, Serialize};
    use std::sync::Mutex;
    use zeroize::Zeroize;

    const SERVICE: &str = "OrbitShell.SSH";
    // Serialize native vault operations; keystore access to one entry must stay ordered.
    static ACCESS: Mutex<()> = Mutex::new(());

    #[derive(Serialize, Deserialize, PartialEq)]
    struct Identity {
        id: String, hostname: String, port: u16, username: String,
        auth_method: String, private_key_path: Option<String>,
    }
    impl From<&ConnectionProfile> for Identity {
        fn from(profile: &ConnectionProfile) -> Self {
            Self { id: profile.id.clone(), hostname: profile.hostname.clone(), port: profile.port,
                username: profile.username.clone(), auth_method: profile.auth_method.clone(),
                private_key_path: profile.private_key_path.clone() }
        }
    }
    #[derive(Deserialize)]
    struct Saved { identity: Identity, secret: String }
    impl Drop for Saved {
        fn drop(&mut self) { self.secret.zeroize(); }
    }
    #[derive(Serialize)]
    struct Saving<'a> { identity: Identity, secret: &'a str }

    fn entry(id: &str) -> Result<Entry, String> {
        uuid::Uuid::parse_str(id).map_err(|_| "连接标识无效")?;
        Entry::new(SERVICE, id).map_err(|_| "无法访问 Windows 凭据管理器".into())
    }

    pub fn load(profile: &ConnectionProfile) -> Result<Option<Zeroizing<String>>, String> {
        let _guard = ACCESS.lock().map_err(|_| "凭据管理器暂不可用")?;
        let value = match entry(&profile.id)?.get_password() {
            Ok(value) => Zeroizing::new(value),
            Err(Error::NoEntry) => return Ok(None),
            // Some native errors can contain raw bytes; never forward their Debug/Display.
            Err(_) => return Err("无法读取已保存的密码，可清除后重新输入".into()),
        };
        let mut saved: Saved = serde_json::from_str(value.as_str())
            .map_err(|_| "已保存的凭据格式无效，请清除后重新输入")?;
        if saved.identity != Identity::from(profile) { return Ok(None); }
        Ok(Some(Zeroizing::new(std::mem::take(&mut saved.secret))))
    }

    pub fn save(profile: &ConnectionProfile, secret: &str) -> Result<(), String> {
        if secret.is_empty() { return Err("请输入需要保存的密码或私钥口令".into()); }
        let _guard = ACCESS.lock().map_err(|_| "凭据管理器暂不可用")?;
        let value = Zeroizing::new(serde_json::to_string(&Saving {identity: Identity::from(profile), secret})
            .map_err(|_| "无法准备凭据数据")?);
        entry(&profile.id)?.set_password(value.as_str())
            .map_err(|_| "无法保存到 Windows 凭据管理器，请确认系统凭据服务可用且密码未超过存储限制".into())
    }

    pub fn forget(id: &str) -> Result<(), String> {
        let _guard = ACCESS.lock().map_err(|_| "凭据管理器暂不可用")?;
        match entry(id)?.delete_credential() {
            Ok(()) | Err(Error::NoEntry) => Ok(()),
            Err(_) => Err("无法清除 Windows 凭据管理器中的密码，请稍后重试".into()),
        }
    }
}

#[cfg(windows)]
pub use native::{forget, load, save};

#[cfg(not(windows))]
pub fn load(_profile: &ConnectionProfile) -> Result<Option<Zeroizing<String>>, String> { Ok(None) }
#[cfg(not(windows))]
pub fn save(_profile: &ConnectionProfile, _secret: &str) -> Result<(), String> {
    Err("当前平台尚未接入系统凭据库，密码仅用于本次连接".into())
}
#[cfg(not(windows))]
pub fn forget(_id: &str) -> Result<(), String> { Ok(()) }
