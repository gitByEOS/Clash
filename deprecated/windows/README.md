# Windows PowerShell 实现已废弃

`bin/clash.ps1` 维护的是另一套 Clash 逻辑，已停止使用。

Windows 从 `v0.1.5` 开始使用 Rust 构建产物：

- 发布产物：`bin/clash-x86_64-pc-windows-msvc.exe`
- 安装入口：`install.ps1`
- 用户命令：`clash` / `clash.exe`

PowerShell 只保留安装脚本，不再承载业务逻辑。

旧 PowerShell 业务脚本和对应 E2E 测试已移除，避免继续维护两套实现。
