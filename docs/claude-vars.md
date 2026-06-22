# Claude Code 环境变量与 CLI 参数参考

> 数据来源：本机 Claude Code `2.1.185` 的 `claude --help` 和 native binary 可见环境变量。

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


> clash 使用 `--permission-mode bypassPermissions`，不启用 auto mode。

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
| `CLAUDE_CODE_ENABLE_AUTO_MODE`         | 启用 auto mode classifier |
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
| `--fallback-model <model>`        | 指定 fallback 模型（仅 `--print`）                                               |
| `--effort <level>`                | 推理力度：`low`、`medium`、`high`、`xhigh`、`max`                                 |
| `--permission-mode <mode>`        | 权限模式：`acceptEdits`、`auto`、`bypassPermissions`、`default`、`dontAsk`、`plan` |
| `--dangerously-skip-permissions`  | 跳过权限检查                                                                   |
| `--allow-dangerously-skip-permissions` | 允许使用 bypass 模式                                                       |
| `-c, --continue`                  | 继续当前目录最近一次会话                                                             |
| `-r, --resume [value]`            | 恢复会话                                                                     |
| `--from-pr [value]`               | 恢复 PR 关联会话                                                               |
| `--fork-session`                  | resume / continue 时创建新 session ID                                        |
| `--session-id <uuid>`             | 使用指定 session ID                                                          |
| `-n, --name <name>`               | 设置会话显示名                                                                  |
| `-p, --print`                     | 非交互模式                                                                    |
| `--input-format <format>`         | print 模式输入格式：`text`、`stream-json`                                        |
| `--output-format <format>`        | print 模式输出格式：`text`、`json`、`stream-json`                                 |
| `--include-partial-messages`      | 输出 partial 消息块                                                           |
| `--include-hook-events`           | 输出 hook 事件                                                               |
| `--replay-user-messages`          | 回显 stdin 用户消息                                                           |
| `--json-schema <schema>`          | 结构化输出 JSON Schema                                                        |
| `--max-budget-usd <amount>`       | print 模式最大 API 花费                                                        |
| `--no-session-persistence`        | 禁用会话持久化                                                                  |
| `--mcp-config <configs...>`       | 加载 MCP 配置                                                                |
| `--strict-mcp-config`             | 只使用 `--mcp-config` 指定的 MCP                                               |
| `--add-dir <directories...>`      | 增加可访问目录                                                                  |
| `--settings <file-or-json>`       | 指定 settings                                                              |
| `--setting-sources <sources>`     | 指定 settings 来源：`user`、`project`、`local`                                  |
| `--agent <agent>`                 | 指定当前 session 的 agent                                                     |
| `--agents <json>`                 | 以内联 JSON 定义自定义 agents                                                    |
| `--tools <tools...>`              | 指定可用内置工具列表                                                               |
| `--allowed-tools <tools...>`      | 允许的工具列表                                                                  |
| `--disallowed-tools <tools...>`   | 禁止的工具列表                                                                  |
| `--system-prompt <prompt>`        | 覆盖 system prompt                                                         |
| `--system-prompt-file <path>`     | 从文件加载 system prompt                                                     |
| `--append-system-prompt <prompt>` | 追加 system prompt                                                         |
| `--append-system-prompt-file <path>` | 从文件追加 system prompt                                                   |
| `--exclude-dynamic-system-prompt-sections` | 排除动态 system prompt 段落                                             |
| `--file <specs...>`               | 启动时下载文件资源                                                                |
| `--plugin-dir <path>`             | 加载本地 plugin                                                              |
| `--plugin-url <url>`              | 从 URL 加载 plugin zip                                                      |
| `--bare`                          | 极简模式                                                                     |
| `--safe-mode`                     | 安全模式                                                                     |
| `--betas <betas...>`              | 追加 API beta headers                                                       |
| `--brief`                         | 启用 SendUserMessage 工具                                                     |
| `--prompt-suggestions`            | 启用 prompt 建议                                                              |
| `--ide`                           | 启动时自动连接 IDE                                                              |
| `--chrome` / `--no-chrome`        | 启用或禁用 Chrome 集成                                                          |
| `--remote-control [name]`         | 启用 Remote Control                                                        |
| `--remote-control-session-name-prefix <prefix>` | 设置 Remote Control 会话名前缀                                      |
| `--worktree [name]`               | 为 session 创建 git worktree                                                |
| `--tmux`                          | 配合 `--worktree` 创建 tmux session                                          |
| `--ax-screen-reader`              | 适配读屏器                                                                    |
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


## Hooks 类型

Claude Code 支持 27 种 Hook 事件，按分类整理如下：

### 工具相关

| Hook 类型 | 说明 |
|-----------|------|
| `PreToolUse` | 工具调用前执行，可阻止操作 |
| `PostToolUse` | 工具调用后执行，可处理结果 |
| `PermissionRequest` | 权限请求时执行 |
| `PermissionDenied` | 权限被拒绝时执行 |

### 会话生命周期

| Hook 类型 | 说明 |
|-----------|------|
| `SessionStart` | 会话开始时执行 |
| `SessionEnd` | 会话结束时执行 |
| `Stop` | 会话停止时执行 |
| `StopFailure` | API 错误导致停止时执行 |
| `Setup` | 通过 `--init/--maintenance` 触发 |

### 子代理相关

| Hook 类型 | 说明 |
|-----------|------|
| `SubagentStart` | 子代理启动时执行 |
| `SubagentStop` | 子代理停止时执行 |
| `TeammateIdle` | 队友进入空闲状态时执行 |
| `TaskCreated` | 任务创建时执行 |
| `TaskCompleted` | 任务完成时执行 |

### 消息相关

| Hook 类型 | 说明 |
|-----------|------|
| `MessageDisplay` | 消息显示时可转换/隐藏内容 |
| `UserPromptSubmit` | 用户提交提示词时执行 |
| `Notification` | 通知事件触发时执行 |

### 文件/环境相关

| Hook 类型 | 说明 |
|-----------|------|
| `FileChanged` | 文件变更时执行 |
| `CwdChanged` | 工作目录变更时执行 |
| `InstructionsLoaded` | `CLAUDE.md/rules` 加载时执行 |
| `ConfigChange` | 配置文件变更时执行 |

### Git Worktree

| Hook 类型 | 说明 |
|-----------|------|
| `WorktreeCreate` | 创建 worktree 时执行 |
| `WorktreeRemove` | 删除 worktree 时执行 |

### 压缩相关

| Hook 类型 | 说明 |
|-----------|------|
| `PreCompact` | 上下文压缩前执行 |
| `PostCompact` | 上下文压缩后执行 |

### 交互相关

| Hook 类型 | 说明 |
|-----------|------|
| `Elicitation` | 用户交互请求时执行 |
| `ElicitationResult` | 用户交互结果时执行 |

### 配置示例

在 `~/.claude/settings.json` 中配置：

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash",
        "hooks": [
          { "type": "command", "command": "echo 'Pre Bash'" }
        ]
      }
    ],
    "SessionStart": [
      {
        "hooks": [
          { "type": "command", "command": "echo 'Session started'" }
        ]
      }
    ]
  }
}
```

### Hook Type 类型

| type | 说明 | 适用事件 | 配置字段 |
|------|------|---------|---------|
| `command` | 执行命令 | 全部 | `command` |
| `prompt` | 提示词判断 | Stop, SubagentStop | `prompt`, `model` |
| `agent` | 代理判断 | Stop, SubagentStop | `agent`, `model` |
| `mcp_tool` | MCP 工具调用 | 全部 | `mcp_tool`, `arguments` |
| `http` | HTTP 请求 | 全部 | `url`, `method`, `headers`, `body` |

**command 类型示例**：
```json
{ "type": "command", "command": "/path/to/script.sh" }
```

**prompt 类型示例** (仅 Stop/SubagentStop)：
```json
{ "type": "prompt", "prompt": "检查是否有未提交的代码", "model": "claude-sonnet-4-20250514" }
```

**agent 类型示例** (仅 Stop/SubagentStop)：
```json
{ "type": "agent", "agent": "code-reviewer", "model": "claude-sonnet-4-20250514" }
```

**mcp_tool 类型示例**：
```json
{ "type": "mcp_tool", "mcp_tool": "mcp__slack__send_message", "arguments": { "channel": "#alerts", "text": "Hook triggered" } }
```

**http 类型示例**：
```json
{ "type": "http", "url": "https://api.example.com/hook", "method": "POST", "headers": { "Authorization": "Bearer token" }, "body": "{\"event\": \"PreToolUse\"}" }
```

使用 `clash hooks` 命令打开浏览器可视化编辑。


