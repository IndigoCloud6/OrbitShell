use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConnectionProfile {
    pub id: String,
    pub name: String,
    pub group: String,
    pub hostname: String,
    pub port: u16,
    pub username: String,
    pub auth_method: String,
    pub private_key_path: Option<String>,
}

impl ConnectionProfile {
    pub fn group_name(name: &str) -> Result<&str, String> {
        let name = name.trim();
        if name.len() > 120 {
            return Err("分组名称过长".into());
        }
        Ok(if name.is_empty() { "未分组" } else { name })
    }

    pub fn validate(&mut self) -> Result<(), String> {
        self.name = self.name.trim().to_owned();
        self.hostname = self.hostname.trim().trim_matches(['[', ']']).to_owned();
        self.username = self.username.trim().to_owned();
        if self.id.is_empty() {
            self.id = Uuid::new_v4().to_string();
        }
        Uuid::parse_str(&self.id).map_err(|_| "连接标识无效")?;
        if self.name.is_empty() || self.name.len() > 160 {
            return Err("请填写有效的连接名称".into());
        }
        self.group = Self::group_name(&self.group)?.to_owned();
        if self.hostname.is_empty()
            || self.hostname.len() > 253
            || self.hostname.chars().any(char::is_whitespace)
        {
            return Err("请填写有效的服务器地址".into());
        }
        if self.port == 0 || self.username.is_empty() || self.username.len() > 4096 {
            return Err("请填写有效的端口和用户名".into());
        }
        if !matches!(self.auth_method.as_str(), "password" | "privateKey") {
            return Err("不支持的认证方式".into());
        }
        if self.auth_method == "privateKey"
            && self
                .private_key_path
                .as_deref()
                .unwrap_or("")
                .trim()
                .is_empty()
        {
            return Err("请选择私钥文件".into());
        }
        if self.auth_method == "password" {
            self.private_key_path = None;
        }
        Ok(())
    }
}
