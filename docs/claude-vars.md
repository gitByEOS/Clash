# Claude Code 环境变量与 CLI 参数参考

> 数据来源：本机 Claude Code `2.1.163` 的 `claude --help` 和 native binary 可见环境变量。

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

环境变量可从本机 native binary 提取候选项，再人工筛掉内部测试/遥测/实验开关：

```bash
claude --version
strings "$(command -v claude)" | sort -u
```

## clash 使用的变量

`clash` 启动 Claude Code 时会设置：


| 变量                                         | 说明                    |
| ------------------------------------------ | --------------------- |
| `ANTHROPIC_BASE_URL`                       | Anthropic 兼容 API 地址   |
| `ANTHROPIC_AUTH_TOKEN`                     | 当前渠道 API Key          |
| `ANTHROPIC_MODEL`                          | 当前选择的主模型              |
| `ANTHROPIC_SMALL_FAST_MODEL`               | 当前选择的小模型              |
| `ANTHROPIC_DEFAULT_SONNET_MODEL`           | Sonnet 默认模型           |
| `ANTHROPIC_DEFAULT_OPUS_MODEL`             | Opus 默认模型             |
| `ANTHROPIC_DEFAULT_HAIKU_MODEL`            | Haiku 默认模型            |
| `CLAUDE_CODE_SUBAGENT_MODEL`               | Subagent 使用模型         |
| `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC` | 禁用非必要网络流量             |
| `CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS`   | 禁用实验性 beta header     |
| `CLAUDE_CODE_ATTRIBUTION_HEADER`           | 禁用 attribution header |
| `CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS`     | 启用 Agent Teams        |
| `CLAUDE_CODE_ENABLE_AUTO_MODE`             | 启用 auto 模式            |


## 常见相关变量


| 变量                                     | 说明                       |
| -------------------------------------- | ------------------------ |
| `ANTHROPIC_API_KEY`                    | Anthropic API Key        |
| `ANTHROPIC_CONFIG_DIR`                 | 覆盖 Claude 配置目录           |
| `ANTHROPIC_CUSTOM_HEADERS`             | 追加自定义请求头                 |
| `ANTHROPIC_BETAS`                      | API beta headers         |
| `ANTHROPIC_LOG`                        | Anthropic SDK 日志级别       |
| `ANTHROPIC_VERTEX_BASE_URL`            | Vertex 兼容入口              |
| `ANTHROPIC_VERTEX_PROJECT_ID`          | Vertex 项目 ID             |
| `ANTHROPIC_BEDROCK_BASE_URL`           | Bedrock 兼容入口             |
| `ANTHROPIC_FOUNDRY_BASE_URL`           | Foundry 兼容入口             |
| `ANTHROPIC_FOUNDRY_API_KEY`            | Foundry API Key          |
| `ANTHROPIC_FOUNDRY_AUTH_TOKEN`         | Foundry Auth Token       |
| `CLAUDE_CODE_MAX_CONTEXT_TOKENS`       | 覆盖最大上下文 token 数          |
| `CLAUDE_CODE_MAX_OUTPUT_TOKENS`        | 覆盖最大输出 token 数           |
| `CLAUDE_CODE_DISABLE_1M_CONTEXT`       | 禁用 1M 上下文窗口              |
| `CLAUDE_CODE_SHELL`                    | 覆盖自动 shell 检测            |
| `CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN` | 禁用全屏 alternate-screen 渲染 |
| `CLAUDE_CODE_DISABLE_TERMINAL_TITLE`   | 阻止修改终端标题                 |
| `CLAUDE_CODE_AUTO_CONNECT_IDE`         | 控制 IDE 自动连接              |
| `CLAUDE_CODE_MANAGED_SETTINGS_PATH`    | 指定托管 settings 路径         |
| `CLAUDE_CODE_PLUGIN_CACHE_DIR`         | 覆盖 plugin 缓存目录           |
| `CLAUDE_CODE_DISABLE_AUTO_MEMORY`      | 禁用自动记忆                   |
| `CLAUDE_CODE_DISABLE_BACKGROUND_TASKS` | 禁用后台任务                   |
| `CLAUDE_CODE_DISABLE_CLAUDE_MDS`       | 禁用 CLAUDE.md 自动发现        |
| `CLAUDE_CODE_DISABLE_MOUSE`            | 禁用鼠标交互                   |
| `CLAUDE_CODE_DISABLE_WORKFLOWS`        | 禁用 workflows             |
| `CLAUDE_CODE_ENABLE_PROMPT_SUGGESTION` | 启用 prompt suggestions    |
| `CLAUDE_CODE_ENABLE_TASKS`             | 启用 tasks                 |
| `CLAUDE_CODE_HTTP_PROXY`               | HTTP 代理                  |
| `CLAUDE_CODE_HTTPS_PROXY`              | HTTPS 代理                 |
| `CLAUDE_CODE_PROXY_URL`                | 统一代理地址                   |
| `CLAUDE_CODE_SESSION_ID`               | 指定会话 ID                  |
| `CLAUDE_CODE_SESSION_NAME`             | 指定会话显示名                  |
| `CLAUDE_CODE_SIMPLE`                   | 简化模式，`--bare` 会设置        |
| `CLAUDE_CODE_MCP_SERVER_NAME`          | 传递给 MCP helper 的服务器名称    |
| `CLAUDE_CODE_MCP_SERVER_URL`           | 传递给 MCP helper 的服务器 URL  |
| `BASH_DEFAULT_TIMEOUT_MS`              | Bash 工具默认超时              |
| `BASH_MAX_TIMEOUT_MS`                  | Bash 工具最大超时              |
| `BASH_MAX_OUTPUT_LENGTH`               | Bash 工具最大输出长度            |
| `DISABLE_TELEMETRY`                    | 禁用遥测                     |


## 常用 CLI 参数


| 参数                                | 说明                                                                       |
| --------------------------------- | ------------------------------------------------------------------------ |
| `--model <model>`                 | 指定模型                                                                     |
| `--fallback-model <model>`        | print 模式下指定过载/不可用时的 fallback 模型                                          |
| `--effort <level>`                | 推理力度：`low`、`medium`、`high`、`xhigh`、`max`                                 |
| `--permission-mode <mode>`        | 权限模式：`acceptEdits`、`auto`、`bypassPermissions`、`default`、`dontAsk`、`plan` |
| `-c, --continue`                  | 继续当前目录最近一次会话                                                             |
| `-r, --resume [value]`            | 恢复会话                                                                     |
| `--fork-session`                  | resume / continue 时创建新 session ID                                        |
| `--session-id <uuid>`             | 使用指定 session ID                                                          |
| `-n, --name <name>`               | 设置会话显示名                                                                  |
| `-p, --print`                     | 非交互模式                                                                    |
| `--input-format <format>`         | print 模式输入格式：`text`、`stream-json`                                        |
| `--output-format <format>`        | print 模式输出格式：`text`、`json`、`stream-json`                                 |
| `--json-schema <schema>`          | 结构化输出 JSON Schema                                                        |
| `--max-budget-usd <amount>`       | print 模式最大 API 花费                                                        |
| `--mcp-config <configs...>`       | 加载 MCP 配置                                                                |
| `--strict-mcp-config`             | 只使用 `--mcp-config` 指定的 MCP                                               |
| `--mcp-debug`                     | 已废弃，改用 `--debug`                                                         |
| `--add-dir <directories...>`      | 增加可访问目录                                                                  |
| `--settings <file-or-json>`       | 指定 settings                                                              |
| `--setting-sources <sources>`     | 指定 settings 来源：`user`、`project`、`local`                                  |
| `--agent <agent>`                 | 指定当前 session 的 agent                                                     |
| `--agents <json>`                 | 以内联 JSON 定义自定义 agents                                                    |
| `--tools <tools...>`              | 指定可用内置工具列表                                                               |
| `--allowed-tools <tools...>`      | 允许的工具列表                                                                  |
| `--disallowed-tools <tools...>`   | 禁止的工具列表                                                                  |
| `--system-prompt <prompt>`        | 覆盖 system prompt                                                         |
| `--append-system-prompt <prompt>` | 追加 system prompt                                                         |
| `--file <specs...>`               | 启动时下载文件资源                                                                |
| `--plugin-dir <path>`             | 加载本地 plugin                                                              |
| `--plugin-url <url>`              | 从 URL 加载 plugin zip                                                      |
| `--bare`                          | 极简模式，跳过 hooks、LSP、plugin sync、auto-memory、CLAUDE.md 自动发现等                |
| `--ide`                           | 启动时自动连接 IDE                                                              |
| `--chrome` / `--no-chrome`        | 启用或禁用 Chrome 集成                                                          |
| `--remote-control [name]`         | 启用 Remote Control                                                        |
| `--worktree [name]`               | 为 session 创建 git worktree                                                |
| `--tmux`                          | 配合 `--worktree` 创建 tmux session                                          |
| `--debug [filter]`                | 启用 debug，可指定过滤器                                                          |
| `--debug-file <path>`             | 写 debug 日志到指定文件                                                          |
| `--verbose`                       | 启用详细输出                                                                   |


## 常用子命令


| 命令                     | 说明                                        |
| ---------------------- | ----------------------------------------- |
| `agents`               | 管理 background agents                      |
| `auth`                 | 管理认证                                      |
| `auto-mode`            | 查看 auto mode classifier 配置                |
| `doctor`               | 检查 Claude Code 自动更新健康状态                   |
| `install [target]`     | 安装 native build，可指定 `stable`、`latest` 或版本 |
| `mcp`                  | 配置和管理 MCP servers                         |
| `plugin` / `plugins`   | 管理 Claude Code plugins                    |
| `project`              | 管理项目状态                                    |
| `setup-token`          | 设置长期认证 token                              |
| `ultrareview [target]` | 运行云端多 agent code review                   |
| `update` / `upgrade`   | 检查并安装更新                                   |


