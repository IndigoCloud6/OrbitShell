use std::collections::BTreeMap;

use orbitshell_ssh::quick_command::QuickCommand;
use serde_json::Value;

use super::QuickCommands;

pub(super) const VERSION: u64 = 2;
const VERSION_KEY: &str = "gpui_linux_quick_commands_version";
const LEGACY_MARKER: &str = "gpui_linux_quick_commands_seeded";
const EXTENDED_PRESETS: &str = include_str!("../assets/linux-quick-commands-v2.json");

const LINUX_PRESETS: &[(&str, &[(&str, &str)])] = &[
    (
        "Linux · 文件目录",
        &[
            ("查看当前目录", "pwd"),
            ("列出文件及隐藏项", "ls -lah"),
            ("按修改时间列文件", "ls -lAht"),
            (
                "查找两层内的文件",
                "find . -maxdepth 2 -type f -print | head -n 80",
            ),
            ("查看当前目录属性", "stat ."),
            (
                "搜索文件内容（替换参数）",
                "grep -nF -- '替换为关键字' '替换为文件路径'",
            ),
            (
                "查看文件末尾（替换路径）",
                "tail -n 80 -- '替换为日志文件路径'",
            ),
        ],
    ),
    (
        "Linux · 系统资源",
        &[
            ("查看系统与内核", "uname -srmo"),
            ("查看开机时长和系统负载", "uptime"),
            ("查看 CPU 型号和核心", "lscpu"),
            ("查看内存使用量", "free -h"),
            ("采样 CPU 和内存状态", "vmstat 1 5"),
            ("查看 PCI 显卡", "lspci | grep -Ei 'VGA|3D|Display'"),
        ],
    ),
    (
        "Linux · 磁盘空间",
        &[
            ("查看文件系统用量", "df -hT"),
            (
                "查看磁盘和分区",
                "lsblk -o NAME,SIZE,TYPE,FSTYPE,MOUNTPOINT",
            ),
            (
                "查看当前目录占用排行",
                "du -xh --max-depth=1 . | sort -hr | head -n 16",
            ),
        ],
    ),
    (
        "Linux · 进程服务",
        &[
            (
                "查看 CPU 占用最高的进程",
                "ps -eo pid,user,%cpu,%mem,comm --sort=-%cpu | head -n 16",
            ),
            (
                "查看内存占用最高的进程",
                "ps -eo pid,user,%cpu,%mem,comm --sort=-%mem | head -n 16",
            ),
            (
                "查看 D 状态进程",
                "ps -eo stat,pid,comm,wchan:24 | awk '$1 ~ /^D/'",
            ),
            ("查看失败的 systemd 单元", "systemctl --failed --no-pager"),
            (
                "查看运行中的服务",
                "systemctl list-units --type=service --state=running --no-pager",
            ),
        ],
    ),
    (
        "Linux · 网络状态",
        &[
            ("查看网卡和 IP 地址", "ip -br address"),
            ("查看路由表", "ip route"),
            ("查看本机监听端口", "ss -lntu"),
            ("查看网卡收发统计", "ip -s link"),
        ],
    ),
    (
        "Linux · 系统日志",
        &[
            ("查看本次启动的近期日志", "journalctl -b -n 80 --no-pager"),
            (
                "查看本次启动的告警和错误",
                "journalctl -b -p warning -n 80 --no-pager",
            ),
            (
                "查看本次启动的内核日志",
                "journalctl -k -b -n 80 --no-pager",
            ),
        ],
    ),
];

pub(super) fn installed_version(document: &Value) -> u64 {
    let legacy_version = (document[LEGACY_MARKER].as_bool() == Some(true)) as u64;
    document[VERSION_KEY]
        .as_u64()
        .unwrap_or(0)
        .max(legacy_version)
}

pub(super) fn seed_pending(
    collection: &mut QuickCommands,
    document: &mut Value,
) -> Result<(), String> {
    let version = installed_version(document);
    if version >= VERSION {
        return Ok(());
    }
    if version < 1 {
        for &(group, commands) in LINUX_PRESETS {
            seed_group(collection, group, commands.iter().copied())?;
        }
    }
    if version < 2 {
        let groups: BTreeMap<String, Vec<(String, String)>> =
            serde_json::from_str(EXTENDED_PRESETS).map_err(|_| "内置 Linux 快捷指令格式无效")?;
        for (group, commands) in &groups {
            seed_group(
                collection,
                group,
                commands
                    .iter()
                    .map(|(name, command)| (name.as_str(), command.as_str())),
            )?;
        }
    }
    document[LEGACY_MARKER] = true.into();
    document[VERSION_KEY] = VERSION.into();
    Ok(())
}

fn seed_group<'a>(
    collection: &mut QuickCommands,
    group: &str,
    commands: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Result<(), String> {
    let mut has_group = collection.groups.iter().any(|existing| existing == group);
    if !has_group && collection.groups.len() >= 100 {
        return Ok(());
    }
    for (name, command) in commands {
        if collection.commands.len() >= 500 {
            break;
        }
        if collection
            .commands
            .iter()
            .any(|existing| existing.command.trim() == command)
        {
            continue;
        }
        if !has_group {
            collection.groups.push(group.into());
            has_group = true;
        }
        collection.commands.push(
            QuickCommand {
                id: uuid::Uuid::new_v4().to_string(),
                name: name.into(),
                group: group.into(),
                command: command.into(),
                append_newline: false,
            }
            .validate()?,
        );
    }
    Ok(())
}
