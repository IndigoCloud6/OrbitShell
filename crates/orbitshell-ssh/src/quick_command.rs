use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QuickCommand {
    pub id: String,
    pub name: String,
    pub group: String,
    pub command: String,
    pub append_newline: bool,
}

pub fn group_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name.len() > 120 || name.chars().any(char::is_control) {
        return Err("请填写有效的分组名称（不超过 120 字节）".into());
    }
    Ok(name.into())
}

impl QuickCommand {
    pub fn validate(mut self) -> Result<Self, String> {
        Uuid::parse_str(&self.id).map_err(|_| "指令标识无效")?;
        self.name = self.name.trim().into();
        if self.name.is_empty() || self.name.len() > 160 || self.name.chars().any(char::is_control)
        {
            return Err("请填写有效的指令名称（不超过 160 字节）".into());
        }
        self.group = group_name(if self.group.trim().is_empty() {
            "未分组"
        } else {
            &self.group
        })?;
        if self.command.trim().is_empty()
            || self.command.len() > 64 * 1024
            || self.command.contains('\0')
        {
            return Err("指令内容不能为空，且不能超过 64 KiB 或包含空字符".into());
        }
        Ok(self)
    }
}
