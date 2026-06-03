# Clash

`clash` 是 Claude Code 的启动器，用于快速切换 Anthropic 兼容 API 渠道和模型。

命名来自 `Claude-Shell`的缩写，该项目是由 [cc-claude](https://github.com/gitByEOS/open-part-skills) 发展而来。

## 平台支持


| 平台            | 实现                      | 安装方式          |
| ------------- | ----------------------- | ------------- |
| macOS / Linux | Rust 原生二进制              | `install.sh`  |
| Windows       | PowerShell 脚本（需 `pwsh`） | `install.ps1` |


## 安装

### macOS / Linux

默认安装到 `~/.local/bin/clash`。

远程一键安装

```bash
curl -fsSL https://raw.githubusercontent.com/gitByEOS/Clash/master/install.sh | bash
```

### Windows

默认安装到 `%LOCALAPPDATA%\Programs\clash\`，并写入 `clash.cmd` 到用户 PATH。

远程一键安装：

```powershell
irm https://raw.githubusercontent.com/gitByEOS/Clash/master/install.ps1 | iex
```

## 使用

首次运行进入配置向导：

```bash
clash
```

未指定项会保留，指定项会覆盖：

```bash
clash config --url https://api.example.com/anthropic
clash config --key sk-xxx
clash config --models model-a,model-b
```

写入配置后会自动对 `MODELS` 列表逐个执行连通测试（等同 `clash test`）。跳过：`CLASH_SKIP_AUTO_TEST=1`。

常用命令：

```bash
clash                  # 选模型并启动，同 clash run
clash run              # 选模型并启动 Claude Code
clash version          # 查看当前版本
clash update           # 检查 Cargo.toml，发现新版本后自动更新
clash config           # 查看当前配置
clash reset            # 删除配置文件
clash test                    # 对 MODELS 列表逐个执行连通测试（不指定 --model 时）
clash test --model glm-5      # 只测单个模型
```

## 配置路径

macOS / Linux：

```text
~/.config/clash/auth
```

Windows：

```text
%APPDATA%\clash\auth
```

## 凭据存储

- **macOS / Linux**：`AUTH_TOKEN` 使用本机 hostname + 用户名派生密钥 AES 加密
- **Windows**：`AUTH_TOKEN` 使用 DPAPI 加密

## 文档

Claude Code 相关环境变量和 CLI 参数见 [docs/claude-vars.md](docs/claude-vars.md)。