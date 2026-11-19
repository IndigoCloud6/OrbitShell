# OrbitShell

OrbitShell 是面向 Windows 的原生 SSH / SFTP 工作台。它把多会话终端、远程文件管理与编辑、传输队列和 AI 协作放在同一个桌面窗口中。当前版本使用 Rust、GPUI 和 `alacritty_terminal`。

![OrbitShell GPUI 工作台，左侧是连接列表，中间是 SSH 终端，右侧是远程文件面板](design/gpui-workbench.png)

*图中使用演示连接和文件。*

## 选择分支

| 分支 | 版本 |
| --- | --- |
| [`gpui`](https://github.com/IndigoCloud6/OrbitShell/tree/gpui) | 默认分支，Rust + GPUI 原生桌面版 |
| [`main`](https://github.com/IndigoCloud6/OrbitShell/tree/main) | 原有 Tauri + React 版，保留原来的源码、启动方式和安装包构建说明 |

## 主要功能

- **SSH 工作台**支持多标签终端、连接分组、密码和 OpenSSH 私钥、主机指纹确认及本机凭据保存。
- **SFTP 与远程编辑**支持文件浏览、拖放上传和下载、传输进度与记录，以及远程文本编辑和语法着色。
- **堡垒机联动**支持从会话链接或 Xshell 风格参数启动临时 SSH 会话。可按需导出只包含协议元数据的 SSH 诊断。
- **AI 与 MCP**支持独立 AI 终端、经用户确认接管已有终端、危险命令拦截、会话内只读授权和后台 SFTP 传输。MCP 在本机回环地址上运行，使用 Bearer 令牌认证。
- **日常操作**包括快捷指令、命令历史、终端查找、深浅主题、可调面板和 Windows 系统托盘。

## 在 Windows 从源码启动

安装 Rust MSVC 工具链和 Visual Studio C++ Build Tools（含 Windows SDK）。在 PowerShell 中执行以下命令。

```powershell
git clone --branch gpui https://github.com/IndigoCloud6/OrbitShell.git
cd OrbitShell
cargo run --locked --manifest-path prototypes/gpui-terminal/Cargo.toml
```

已有本地仓库时，可先运行 `git fetch origin`，再用 `git switch gpui` 切换到 GPUI 分支。原 Tauri 版的开发和安装包命令见 [`main` 分支文档](https://github.com/IndigoCloud6/OrbitShell/tree/main#从源码启动)。

## 构建 Windows 可执行文件

在仓库根目录执行以下命令。

```powershell
cargo build --release --locked --manifest-path prototypes/gpui-terminal/Cargo.toml --bin orbitshell-gpui-terminal
```

产物位于 `prototypes/gpui-terminal/target/release/orbitshell-gpui-terminal.exe`。可以将它复制到常用目录并命名为 `OrbitShell.exe`。

如果堡垒机助手要求 Xshell 风格的客户端入口，可通过相同清单构建转接器。

```powershell
cargo build --release --locked --manifest-path prototypes/gpui-terminal/Cargo.toml --bin xshell
```

将生成的 `xshell.exe` 与 `OrbitShell.exe` 放在同一个目录，再在助手里选择转接器。转接器会转发启动参数。

## 开发与使用文档

- [GPUI 工作台使用说明](prototypes/gpui-terminal/README.md)覆盖登录、堡垒机、SFTP、MCP 和开发入口。
- [共享 Rust 核心](crates/orbitshell-ssh)提供 SSH、SFTP、连接凭据、远程编辑和传输逻辑。
- [第三方许可说明](THIRD_PARTY_NOTICES.md)列出随项目使用的第三方资源。
