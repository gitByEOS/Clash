# Claude Code 环境变量与 CLI 参数参考

> 数据来源：`anthropics/claude-code` 官方仓库、变更日志和 `claude --help`。

## 如何更新

环境变量可在 Claude Code 仓库中搜索：

```text
CLAUDE_CODE_
ANTHROPIC_
```

CLI 参数以本机安装版本为准：

```bash
claude --help
```

## clash 使用的变量

`clash` 启动 Claude Code 时会设置：

| 变量 | 说明 |
|---|---|
| `ANTHROPIC_BASE_URL` | Anthropic 兼容 API 地址 |
| `ANTHROPIC_AUTH_TOKEN` | 当前渠道 API Key |
| `ANTHROPIC_MODEL` | 当前选择的主模型 |
| `ANTHROPIC_SMALL_FAST_MODEL` | 当前选择的小模型 |
| `ANTHROPIC_DEFAULT_SONNET_MODEL` | Sonnet 默认模型 |
| `ANTHROPIC_DEFAULT_OPUS_MODEL` | Opus 默认模型 |
| `ANTHROPIC_DEFAULT_HAIKU_MODEL` | Haiku 默认模型 |
| `CLAUDE_CODE_SUBAGENT_MODEL` | Subagent 使用模型 |
| `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC` | 禁用非必要网络流量 |
| `CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS` | 禁用实验性 beta header |
| `CLAUDE_CODE_ATTRIBUTION_HEADER` | 禁用 attribution header |
| `CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS` | 启用 Agent Teams |
| `CLAUDE_CODE_ENABLE_AUTO_MODE` | 启用 auto 模式 |

## 常见相关变量

| 变量 | 说明 |
|---|---|
| `CLAUDE_CODE_MAX_CONTEXT_TOKENS` | 覆盖最大上下文 token 数 |
| `CLAUDE_CODE_MAX_OUTPUT_TOKENS` | 覆盖最大输出 token 数 |
| `CLAUDE_CODE_DISABLE_1M_CONTEXT` | 禁用 1M 上下文窗口 |
| `CLAUDE_CODE_SHELL` | 覆盖自动 shell 检测 |
| `CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN` | 禁用全屏 alternate-screen 渲染 |
| `CLAUDE_CODE_DISABLE_TERMINAL_TITLE` | 阻止修改终端标题 |
| `CLAUDE_CODE_AUTO_CONNECT_IDE` | 控制 IDE 自动连接 |
| `CLAUDE_CODE_MCP_SERVER_NAME` | 传递给 MCP helper 的服务器名称 |
| `CLAUDE_CODE_MCP_SERVER_URL` | 传递给 MCP helper 的服务器 URL |
| `DISABLE_TELEMETRY` | 禁用遥测 |

## 常用 CLI 参数

| 参数 | 说明 |
|---|---|
| `--model <model>` | 指定模型 |
| `--effort <level>` | 推理力度：`low`、`medium`、`high`、`xhigh`、`max` |
| `--permission-mode <mode>` | 权限模式 |
| `-c, --continue` | 继续当前目录最近一次会话 |
| `-r, --resume [value]` | 恢复会话 |
| `-p, --print` | 非交互模式 |
| `--mcp-config <configs...>` | 加载 MCP 配置 |
| `--add-dir <directories...>` | 增加可访问目录 |
| `--settings <file-or-json>` | 指定 settings |
| `--verbose` | 启用详细输出 |
