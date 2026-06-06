# Clash E2E 功能覆盖表

## 测试脚本

| 脚本 | 描述 | 运行方式 |
|------|------|----------|
| `test_features.py` | Rust CLI/TUI 测试 | `python3 e2e/test_features.py` |
| `test_features_pwsh.py` | PowerShell CLI/TUI 测试 | `python3 e2e/test_features_pwsh.py` |
| `test_statusline.py` | Statusline 功能测试 | `python3 e2e/test_statusline.py` |

## 命令覆盖

| 命令 | 功能描述 | Rust E2E | PowerShell E2E | 备注 |
|------|----------|:--------:|:--------------:|------|
| `version` | 显示版本号 | ✅ | ✅ | v0.x.x 格式 |
| `update` | 检查并更新 clash | ❌ | ❌ | 需网络交互，CI 不稳定 |
| `statusline` | 输出状态栏 | ✅ | ✅ | 独立 test_statusline.py |
| `run` | 选择模型并运行 Claude | ✅ | ✅ | TUI + 环境变量 |
| `config` | 配置管理 | ✅ | ✅ | 完整覆盖 |
| `reset` | 删除所有配置 | ✅ | ✅ | |
| `test` | API 连通测试 | ✅ | ✅ | Mock HTTP Server |
| `rename` | 重命名账户 | ✅ | ✅ | 通过 config 写入 NAME |
| 默认（无命令） | 同 `run` | ✅ | ✅ | |

## config 子命令覆盖

| 参数组合 | 功能描述 | Rust E2E | PowerShell E2E | 备注 |
|----------|----------|:--------:|:--------------:|------|
| `config --idx N` | 显示配置 N | ✅ | ✅ | |
| `config --idx N --url --key --models` | 完整设置配置 | ✅ | ✅ | |
| `config --url URL` | 部分 URL 更新 | ✅ | ✅ | |
| `config --key KEY` | 部分 Key 更新 | ✅ | ✅ | |
| `config --models MODELS` | 部分 models 更新 | ✅ | ✅ | |
| `config --models " , "` | 空模型列表拒绝 | ✅ | ✅ | |
| `config --idx abc` | 非法 idx 拒绝 | ✅ | ✅ | |
| 交互式配置向导 | stdin 输入 URL/Key/Models | ✅ | ✅ | 配置不存在时触发 |

## test 子命令覆盖

| 参数组合 | 功能描述 | Rust E2E | PowerShell E2E | 备注 |
|----------|----------|:--------:|:--------------:|------|
| `test` | 测试所有账户模型 | ✅ | ✅ | Mock Server |
| `test --idx N` | 测试指定账户 | ✅ | ✅ | |
| `test --idx abc` | 非法 idx 拒绝 | ✅ | ✅ | |

## TUI 功能覆盖

| 功能 | 功能描述 | Rust E2E | PowerShell E2E | 备注 |
|------|----------|:--------:|:--------------:|------|
| 单账户首帧 | 无账户标签，显示模型数 | ✅ | ✅ | |
| 多账户标签 | `[1st]` `[2st]` 格式 | ✅ | ✅ | |
| 重命名标签 | `[work]` 替代 `[1st]` | ✅ | ✅ | |
| 模型计数 | `1/N` 显示 | ✅ | ✅ | |
| 搜索过滤 | 输入关键字过滤模型 | ✅ | ✅ | 输入 kim 过滤 |
| 下箭头导航 | 切换选中模型 | ✅ | ✅ | |
| 上箭头导航 | 切换选中模型 | ✅ | ✅ | |
| Enter 确认 | 选择模型执行 | ✅ | ✅ | 隐式覆盖 |
| Esc 取消 | 退出 TUI | ✅ | ✅ | |

## 多账户功能覆盖

| 功能 | 功能描述 | Rust E2E | PowerShell E2E | 备注 |
|------|----------|:--------:|:--------------:|------|
| 多账户配置 | idx 0, 1, 2, 10... | ✅ | ✅ | |
| 账户数字排序 | auth2 排在 auth10 前 | ✅ | ✅ | |
| 账户命名 | NAME 字段 | ✅ | ✅ | |
| 账户选择 | TUI 显示账户标签 | ✅ | ✅ | |

## 环境变量覆盖

| 环境变量 | 功能描述 | Rust E2E | PowerShell E2E | 备注 |
|----------|----------|:--------:|:--------------:|------|
| `ANTHROPIC_BASE_URL` | API 地址 | ✅ | ✅ | run 验证 |
| `ANTHROPIC_AUTH_TOKEN` | API Key | ✅ | ✅ | run 验证 |
| `ANTHROPIC_MODEL` | 模型名 | ✅ | ✅ | run 验证 |
| `CLASH_SKIP_AUTO_TEST` | 跳过自动测试 | ✅ | ✅ | 测试控制 |
| `CLASH_TEST_CONFIG_HOME` | 测试配置目录 | ✅ | ❌ | Rust 专用 |
| `APPDATA` | Windows 配置目录 | ❌ | ✅ | PowerShell 专用 |

## 已移除命令覆盖

| 命令 | 功能描述 | Rust E2E | PowerShell E2E | 备注 |
|------|----------|:--------:|:--------------:|------|
| `add-model <model>` | 作为 Claude 参数传递 | ✅ | ✅ | 不再作为 clash 命令 |
| `change-token <key>` | 作为 Claude 参数传递 | ✅ | ✅ | 不再作为 clash 命令 |

## Statusline 功能覆盖 (test_statusline.py)

| 功能 | 功能描述 | 覆盖 | 备注 |
|------|----------|:----:|------|
| 空输入 | 显示 "Clash" | ✅ | |
| 基本 JSON | 模型名 + 进度条 + 百分比 | ✅ | |
| size marker 去除 | `[1m]` 从模型名移除 | ✅ | |
| 高百分比颜色 | >= 90% 显示红色 | ✅ | |
| session 时长 | 显示 ⏱ Xm | ✅ | |
| 输出格式 | `[model] 进度条 N% - size \| Clash` | ✅ | |
| 进度条颜色 | green/orange/yellow/red | ✅ | |
| 自动配置 | ensure_statusline_config | ✅ | |
| 保留已有 settings | 合并入现有配置 | ✅ | |
| 跳过有效 statusline | 不覆盖已有配置 | ✅ | |
| 修复空 statusline | `{}` 被替换 | ✅ | |
