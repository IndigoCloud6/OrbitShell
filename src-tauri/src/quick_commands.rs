use crate::AppState;
use serde::{Deserialize, Serialize};
use tauri::State;
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

fn group_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name.len() > 120 || name.chars().any(char::is_control) { return Err("请填写有效的分组名称（不超过 120 字节）".into()); }
    Ok(name.into())
}

#[derive(Serialize)]
pub struct QuickCommandCollection { commands: Vec<QuickCommand>, groups: Vec<String> }

#[tauri::command]
pub fn list_quick_commands(state: State<'_, AppState>) -> Result<QuickCommandCollection, String> {
    let store = state.storage.store.lock().map_err(|_| "配置暂不可用")?;
    Ok(QuickCommandCollection { commands: store.quick_commands.clone(), groups: store.quick_command_groups.clone() })
}

#[tauri::command]
pub fn save_quick_command(mut command: QuickCommand, state: State<'_, AppState>) -> Result<QuickCommand, String> {
    Uuid::parse_str(&command.id).map_err(|_| "指令标识无效")?;
    command.name = command.name.trim().into();
    if command.name.is_empty() || command.name.len() > 160 || command.name.chars().any(char::is_control) { return Err("请填写有效的指令名称（不超过 160 字节）".into()); }
    command.group = group_name(if command.group.trim().is_empty() { "未分组" } else { &command.group })?;
    if command.command.trim().is_empty() || command.command.len() > 64 * 1024 || command.command.contains('\0') { return Err("指令内容不能为空，且不能超过 64 KiB 或包含空字符".into()); }
    let mut store = state.storage.store.lock().map_err(|_| "配置暂不可用")?;
    let mut next = store.clone();
    if let Some(existing) = next.quick_commands.iter_mut().find(|entry| entry.id == command.id) { *existing = command.clone(); }
    else {
        if next.quick_commands.len() >= 500 { return Err("最多保存 500 个快捷指令".into()); }
        next.quick_commands.push(command.clone());
    }
    if command.group != "未分组" && !next.quick_command_groups.contains(&command.group) {
        if next.quick_command_groups.len() >= 100 { return Err("最多保存 100 个指令分组".into()); }
        next.quick_command_groups.push(command.group.clone());
    }
    state.storage.persist(&next).map_err(|_| "快捷指令保存失败，请重试")?;
    *store = next;
    Ok(command)
}

#[tauri::command]
pub fn delete_quick_command(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let mut store = state.storage.store.lock().map_err(|_| "配置暂不可用")?;
    let mut next = store.clone();
    next.quick_commands.retain(|entry| entry.id != id);
    state.storage.persist(&next).map_err(|_| "删除指令失败，请重试")?;
    *store = next;
    Ok(())
}

#[tauri::command]
pub fn save_quick_command_group(name: String, previous_name: Option<String>, state: State<'_, AppState>) -> Result<(), String> {
    let name = group_name(&name)?;
    if name == "未分组" { return Err("未分组是保留名称，请使用其他名称".into()); }
    let mut store = state.storage.store.lock().map_err(|_| "配置暂不可用")?;
    let mut next = store.clone();
    if next.quick_command_groups.contains(&name) && previous_name.as_deref() != Some(name.as_str()) { return Err("该分组已存在".into()); }
    if let Some(previous) = previous_name {
        if previous == "未分组" { return Err("未分组不能重命名".into()); }
        let existing = next.quick_command_groups.iter_mut().find(|group| **group == previous).ok_or("分组不存在")?;
        *existing = name.clone();
        for command in &mut next.quick_commands { if command.group == previous { command.group = name.clone(); } }
    } else {
        if next.quick_command_groups.len() >= 100 { return Err("最多保存 100 个指令分组".into()); }
        next.quick_command_groups.push(name);
    }
    state.storage.persist(&next).map_err(|_| "分组保存失败，请重试")?;
    *store = next;
    Ok(())
}

#[tauri::command]
pub fn delete_quick_command_group(name: String, state: State<'_, AppState>) -> Result<(), String> {
    if name == "未分组" { return Err("未分组不能删除".into()); }
    let mut store = state.storage.store.lock().map_err(|_| "配置暂不可用")?;
    let mut next = store.clone();
    next.quick_command_groups.retain(|group| group != &name);
    for command in &mut next.quick_commands { if command.group == name { command.group = "未分组".into(); } }
    state.storage.persist(&next).map_err(|_| "删除分组失败，请重试")?;
    *store = next;
    Ok(())
}
