# OrbitShell GPUI 工作台

这是 OrbitShell 默认分支上的 Windows 桌面应用。GPUI 负责窗口、输入和绘制，`alacritty_terminal` 按字节解析 ANSI / VT 输出并维护终端网格。SSH 和 SFTP 逻辑位于共享模块 `crates/orbitshell-ssh`。分支介绍与 Release 构建见[项目 README](../../README.md)。

在 Windows 上安装 Rust MSVC 工具链和 Visual Studio C++ Build Tools（含 Windows SDK）后，从仓库根目录启动。

```powershell
cargo run --locked --manifest-path prototypes/gpui-terminal/Cargo.toml
```

开发时可从仓库根目录启动自动重载。保存 GPUI 源码、共用 SSH 源码或图标资源后会自动编译，成功后重开窗口。编译失败会保留上一个可用版本。重开窗口会断开 SSH 会话，手动关闭窗口会结束监听。

```powershell
.\prototypes\gpui-terminal\dev.ps1
```

- **工作台**左侧活动栏控制连接、文件和底部工具区的显隐。拖动终端两侧的分隔栏可调节面板宽度。
- **会话标签**点击连接可打开登录或切换终端。Ctrl+Tab 向右切换，Ctrl+Shift+Tab 向左切换。每个标签保留自己的终端缓冲和文件位置，中键或关闭按钮可关闭标签。
- **连接配置**所有 SSH 连接在同一个工作区中管理。首次启动会合并先前单独保存的 GPUI 连接，并保留连接 ID。可在「管理连接」中新增或编辑。改名称和分组会保留已保存的凭据，修改连接身份会清除旧凭据。
- **SSH 登录**支持密码和 OpenSSH 私钥。选择连接后先确认目标。存在匹配的已保存凭据时默认使用它，也可以切换到手动输入。确认后才发起 SSH。首次连接时请核对服务器指纹。窗口或终端字号变化会更新远程 PTY 的行列数。终端使用 Ctrl+Shift+C 复制可见内容，Ctrl+V 粘贴。
- **远程文件**文件图标可打开面板。编辑顶部路径后按 Enter 跳转。选中文件后可点下载，也可将文件拖到 Windows 资源管理器。可用上传按钮选文件，或从资源管理器把文件拖到列表或目标子目录。文件上传同名时需要确认，传输队列显示实际字节数和速度。
- **堡垒机**可从「设置 → 堡垒机联动」粘贴会话链接或注册 Windows 链接入口。已有系统关联时，在 Windows 默认应用里选择 OrbitShell GPUI。堡垒机助手可直接指向安装目录中的 `OrbitShell.exe`，主程序接收 `-url`、`-newtab` 和会话链接。外部链接优先唤起已有窗口，确认目标后发起临时 SSH 会话。
- **MCP**在「设置 → AI 与 MCP」启用，端点为 `http://127.0.0.1:43123/mcp`，客户端用 Bearer 令牌认证。可以发起连接、使用独立 AI 终端、接管或交回原终端，以及提交 SFTP 传输。桌面确认接管和操作后，Agent 可在原标签或独立 AI 标签查看输出。独立终端返回命令退出码，原终端仍保留当前目录和堡垒机交互状态。底部「AI 工具」显示客户端及所属终端。
- **MCP 只读授权**在独立 AI 终端执行可识别的只读排查命令时，可以选「仅本次」或「允许本会话只读」。只读授权绑定当前 Agent、SSH 会话和独立终端。关闭终端或断开连接时失效，也可点击标签上的「撤销只读授权」。写操作、接管和提交交互输入须单独确认，高风险规则继续先于授权检查。
- **终端外观**在「设置 → 外观」调整终端字号和浅色或深色配色。字号范围为 11–24 px，默认 13 px。终端内仍可用 Ctrl+- 和 Ctrl+= 调整，按 Ctrl+0 恢复默认值。当前窗口中的所有终端共用这些设置。
- **终端查找与快捷指令**按 Ctrl+F 查找终端内容，Enter 和 Shift+Enter 切换匹配。底部「快捷指令」可按组筛选和搜索，将选中的指令填入当前终端。
- **菜单与设置**顶部提供文件、编辑、视图、会话和帮助菜单。设置可调文件列表密度、隐藏文件和终端字号。上传、下载、Agent 状态和堡垒机入口提示当前后端接入状态。

## 堡垒机 SSH 诊断

在堡垒机临时会话中，打开底部「连接日志」，点击「保存本次 SSH 诊断」。JSON 记录面板交互、SSH 通道与消息类型、PTY 行列数和 SFTP 请求时间。诊断只保存协议元数据。拖动分隔线时本地终端随面板调整，松手后才同步有变化的最终行列数。

## 开发参考

GPUI 工作台使用独立的 Cargo 锁文件。原 Tauri 版的使用与构建说明见 [`main` 分支](https://github.com/IndigoCloud6/OrbitShell/tree/main)。

MCP 回环集成测试需要本机 Python 和 Paramiko。测试使用随机回环端口、临时工作区及隔离的 SSH/SFTP 服务。

```powershell
cargo test --locked --manifest-path prototypes/gpui-terminal/Cargo.toml --bin orbitshell-gpui-terminal mcp_loopback -- --ignored
```
