# Material Icon Theme 文件图标素材

来源：[Material Icon Theme](https://github.com/material-extensions/vscode-material-icon-theme)，npm 包 `material-icon-theme@5.40.0`。许可证为 MIT，完整授权文本保留在同目录 `LICENSE` 中。

本目录仅保留远程文件列表常用的图标及相关文件名、扩展名、目录名映射，不引入包的 JavaScript 运行时。素材在客户端本地打包，文件名不会发送到第三方服务。

- `associations.json` 从该版本发布包的 `dist/material-icons.json` 提取，记录来源与版本。
- 保留 73 个 SVG，包含常用文件、目录和可用的浅色变体。
- 映射补充了常见 Shell 点号文件、Linux 配置文件、Cargo 文件及复合压缩包后缀；不读取文件内容进行识别。
- 匹配顺序：目录名或完整文件名 → 复合扩展名 → 普通扩展名 → 通用图标。
- 浅色变体跟随应用主题；未知类型使用通用图标；符号链接叠加链接标记。
- 新增类型时一并维护图标与映射，更新上游版本时保留许可证及来源说明。
