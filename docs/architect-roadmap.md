# Clash 架构完善路线图

这份路线图来自 Claude 架构指南的几个核心判断：代理系统先管好上下文、工具边界、结构化输出和失败路径，再谈更大的编排能力。`clash` 当前已经有启动器、提示词捕获、Hooks、飞书会话这几条主线，下一步应把它们收束成可诊断、可控制、可验收的产品能力。

## 当前定位

`clash` 不是 Claude Code 的替代品，而是 Claude Code 的生产化启动器：

- 渠道和模型选择由 `clash` 前置完成
- 凭据、环境变量、状态栏、系统提示词由 `clash` 管理
- `clash prompts` 用于观察真实请求
- `clash lark` 用飞书承载远程会话和多会话协作

因此，路线图不应扩成通用 Agent SDK。主线应该是：让 Claude Code 在第三方渠道、多模型、多会话环境里更可控。

## 灵感映射

| 指南主题 | 对 Clash 的启发 | 落地点 |
| --- | --- | --- |
| 上下文窗口 | 请求体、工具定义、规则和技能都会吃上下文 | 增强 `clash prompts` 诊断 |
| 工具设计 | 工具描述和最小权限影响模型行为 | 支持启动 Profile 管理工具白名单 |
| MCP 集成 | MCP 需要和内置工具形成明确边界 | 暴露 MCP 配置和工具分组 |
| 结构化输出 | JSON Schema 比纯文本更适合自动化链路 | 用于飞书任务状态和报告摘要 |
| 代理编排 | 子代理需要显式上下文传递 | 飞书会话要显示上下文和任务来源 |
| 可靠性 | 失败、超时、人工介入要成为设计的一部分 | 飞书错误卡片和管理群升级 |

## 版本路线

### v0.3.1：提示词诊断

目标：让 `clash prompts` 从“抓包工具”升级为“上下文体检工具”。

可交付：

- 在 HTML 报告增加上下文诊断区
- 统计 System、Messages、Tools、MCP Tools、Skills、本地规则的字节占比
- 列出体积最大的工具定义和本地文件
- 标记潜在浪费：超长工具 schema、空 messages、重复 system 片段
- JSON 输出同步包含诊断数据，便于脚本消费

验收：

```bash
clash prompts --json
clash prompts --html
```

成功标准：报告能回答“本次请求上下文主要被谁占用”。

### v0.3.2：启动 Profile

目标：把 Claude Code 参数从硬编码启动参数里抽出来，用 Profile 表达不同工作模式。

建议模型：

```text
~/.config/clash/profiles/default.json
~/.config/clash/profiles/review.json
~/.config/clash/profiles/safe.json
```

Profile 只管理行为，不管理密钥：

- `permission_mode`
- `effort`
- `mcp_config`
- `allowed_tools`
- `disallowed_tools`
- `append_system_prompt_file`
- `setting_sources`
- `max_budget_usd`

命令形态：

```bash
clash run --profile review
clash profile list
clash profile show review
```

验收：同一个账号和模型，可以通过不同 Profile 启动出不同工具边界。

### v0.3.3：飞书会话可靠性

目标：让 `clash lark` 像远程控制台，而不是只做消息转发。

可交付：

- 卡片展示当前模型、会话名、最后一次工具调用、最后错误
- Claude stream-json 中的错误事件要落到飞书消息
- 超时后给出明确状态，而不是只返回一段错误文本
- 管理群支持“暂停会话”“重启会话”“转人工”三类指令
- 会话创建时把触发人、来源群、初始提示词显式写入上下文

验收：Claude 不返回、工具报错、进程退出时，飞书侧都能看到原因和下一步动作。

### v0.3.4：结构化任务摘要

目标：让飞书会话输出可被机器读取的任务状态，减少纯文本不可控的问题。

可交付：

- 为飞书任务定义最小 JSON Schema：`state`、`summary`、`next_action`、`risks`
- Claude 完成一轮后生成结构化摘要
- 管理群聚合所有会话状态
- HTML 报告可展示结构化摘要原文

验收：管理群能稳定看到“哪些会话在做什么、是否卡住、下一步是什么”。

### v0.3.5：MCP 与工具治理

目标：让工具暴露成为显式配置，而不是所有能力默认进入上下文。

可交付：

- Profile 支持 MCP 配置文件
- Profile 支持工具允许/禁止列表
- `clash prompts` 报告按 Builtin Tools 和 MCP Tools 分组展示
- 报告提示可能的工具冲突，比如 MCP 搜索工具和内置 `Grep` 同时存在

验收：用户能为 Review、写代码、飞书远程控制三类场景配置不同工具集。

## 不做什么

- 不把 `clash` 做成完整 Agent SDK
- 不在当前配置文件里继续塞复杂结构
- 不用临时参数堆出长期能力
- 不默认开启所有 MCP 和工具
- 不为未发布的中间设计写兼容层

## 优先级判断

先做 `clash prompts`。它风险最低，能直接暴露真实请求结构，也能反过来指导 Profile、MCP、飞书会话的设计。

第二步做 Profile。它是工具边界、MCP、预算、权限模式的承载点。

飞书可靠性放第三步。它依赖前两步，否则只是把不可控的 Claude 会话搬到飞书里。

## 验证清单

- `cargo test`
- `cargo build --release`
- `clash prompts --json` 输出包含诊断字段
- `clash prompts --html` 页面能定位上下文大户
- `clash run --profile review` 能按 Profile 注入参数
- `clash lark --once` 能在失败路径给出明确错误

