use crate::cli::{print_cyan, print_green, print_red, print_yellow};
use serde_json::{json, Value};
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process;
use std::thread;
use std::time::Duration;

/// Hook 分类 (直接定义每个分类的内容)
const HOOK_CATEGORIES: [(&str, &[(&str, &str)]); 8] = [
    (
        "工具相关",
        &[
            ("PreToolUse", "工具调用前执行，可阻止操作"),
            ("PostToolUse", "工具调用后执行，可处理结果"),
            ("PermissionRequest", "权限请求时执行"),
            ("PermissionDenied", "权限被拒绝时执行"),
        ],
    ),
    (
        "文件/环境相关",
        &[
            ("FileChanged", "文件变更时执行"),
            ("CwdChanged", "工作目录变更时执行"),
            ("InstructionsLoaded", "CLAUDE.md/rules 加载时执行"),
            ("ConfigChange", "配置文件变更时执行"),
        ],
    ),
    (
        "会话生命周期",
        &[
            ("SessionStart", "会话开始时执行"),
            ("SessionEnd", "会话结束时执行"),
            ("Stop", "会话停止时执行"),
            ("StopFailure", "API 错误导致停止时执行"),
            ("Setup", "通过 --init/--maintenance 触发"),
        ],
    ),
    (
        "消息相关",
        &[
            ("MessageDisplay", "消息显示时可转换/隐藏内容"),
            ("UserPromptSubmit", "用户提交提示词时执行"),
            ("Notification", "通知事件触发时执行"),
        ],
    ),
    (
        "子代理相关",
        &[
            ("SubagentStart", "子代理启动时执行"),
            ("SubagentStop", "子代理停止时执行"),
            ("TeammateIdle", "队友进入空闲状态时执行"),
            ("TaskCreated", "任务创建时执行"),
            ("TaskCompleted", "任务完成时执行"),
        ],
    ),
    (
        "Git Worktree",
        &[
            ("WorktreeCreate", "创建 worktree 时执行"),
            ("WorktreeRemove", "删除 worktree 时执行"),
        ],
    ),
    (
        "压缩相关",
        &[
            ("PreCompact", "上下文压缩前执行"),
            ("PostCompact", "上下文压缩后执行"),
        ],
    ),
    (
        "交互相关",
        &[
            ("Elicitation", "用户交互请求时执行"),
            ("ElicitationResult", "用户交互结果时执行"),
        ],
    ),
];

/// settings.json 路径
pub fn settings_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    PathBuf::from(home).join(".claude/settings.json")
}

fn hooks_tools_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    PathBuf::from(home).join(".config/clash/tools")
}

fn dont_rm_script_path() -> PathBuf {
    hooks_tools_dir().join("dont_rm.sh")
}

fn notify_script_path() -> PathBuf {
    hooks_tools_dir().join("notify.sh")
}

fn backup_file_script_path() -> PathBuf {
    hooks_tools_dir().join("backup_file.sh")
}

fn hook_detail_script_path() -> PathBuf {
    hooks_tools_dir().join("hook_detail.sh")
}

fn execute_confirm_script_path() -> PathBuf {
    hooks_tools_dir().join("execute_confirm.sh")
}

fn ensure_hook_tools() -> Result<(), String> {
    ensure_hook_tool(&dont_rm_script_path(), DONT_RM_SCRIPT)?;
    ensure_hook_tool(&notify_script_path(), NOTIFY_SCRIPT)?;
    ensure_hook_tool(&backup_file_script_path(), BACKUP_FILE_SCRIPT)?;
    ensure_hook_tool(&hook_detail_script_path(), HOOK_DETAIL_SCRIPT)?;
    ensure_hook_tool(&execute_confirm_script_path(), EXECUTE_CONFIRM_SCRIPT)?;
    Ok(())
}

fn ensure_hook_tool(path: &PathBuf, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    if !path.exists() {
        fs::write(path, content).map_err(|e| e.to_string())?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path).map_err(|e| e.to_string())?.permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 读取当前 settings.json
pub fn read_settings() -> Value {
    let path = settings_path();
    if !path.exists() {
        return json!({});
    }
    let content = fs::read_to_string(&path).unwrap_or_default();
    serde_json::from_str(&content).unwrap_or(json!({}))
}

/// 写入 settings.json
pub fn write_settings(settings: &Value) -> Result<(), String> {
    let path = settings_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let content = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    fs::write(&path, content).map_err(|e| e.to_string())?;
    Ok(())
}

/// 启动 hooks 编辑服务
pub fn do_hooks() -> Result<(), ()> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|err| {
        print_red(&format!("无法启动本地服务: {err}"));
    })?;
    let addr = listener.local_addr().map_err(|err| {
        print_red(&format!("无法读取端口: {err}"));
    })?;

    print_cyan(&format!("Hooks 编辑服务已启动: http://{addr}"));
    print_yellow("请在浏览器中编辑，保存后服务将自动关闭");

    ensure_hook_tools().map_err(|err| {
        print_red(&format!("初始化 hooks 工具失败: {err}"));
    })?;

    // 打开浏览器
    open_browser(&format!("http://{addr}")).map_err(|err| {
        print_red(&format!("打开浏览器失败: {err}"));
    })?;

    // 处理请求
    handle_requests(listener)?;

    print_green("Hooks 配置已保存");
    Ok(())
}

fn open_browser(url: &str) -> Result<(), String> {
    let status = if cfg!(target_os = "macos") {
        process::Command::new("open").arg(url).status()
    } else {
        process::Command::new("xdg-open").arg(url).status()
    }
    .map_err(|e| e.to_string())?;

    if status.success() {
        Ok(())
    } else {
        Err("打开浏览器失败".to_string())
    }
}

fn handle_requests(listener: TcpListener) -> Result<(), ()> {
    let mut saved = false;
    let deadline = std::time::Instant::now() + Duration::from_secs(300); // 5分钟超时

    listener.set_nonblocking(true).map_err(|err| {
        print_red(&format!("设置非阻塞失败: {err}"));
    })?;

    while !saved && std::time::Instant::now() < deadline {
        match listener.accept() {
            Ok((mut stream, _)) => match handle_stream(&mut stream) {
                Ok(should_close) => {
                    saved = should_close;
                }
                Err(err) => {
                    print_red(&format!("处理请求失败: {err}"));
                }
            },
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(100));
            }
            Err(err) => {
                print_red(&format!("接受连接失败: {err}"));
                return Err(());
            }
        }
    }

    if !saved {
        print_yellow("超时未保存，已关闭");
    }

    Ok(())
}

fn handle_stream(stream: &mut TcpStream) -> Result<bool, String> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];

    loop {
        let n = stream.read(&mut chunk).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..n]);
        if buffer.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }

    let request = String::from_utf8_lossy(&buffer);
    let first_line = request.lines().next().unwrap_or("");

    if first_line.contains("GET /data") {
        // 返回当前 hooks 数据
        let settings = read_settings();
        let hooks = settings.get("hooks").cloned().unwrap_or(json!({}));
        let body = serde_json::to_string(&hooks).map_err(|e| e.to_string())?;
        write_response(stream, 200, "application/json", &body)?;
        Ok(false)
    } else if first_line.contains("GET /") {
        // 返回 HTML 页面
        let html = render_hooks_html();
        write_response(stream, 200, "text/html", &html)?;
        Ok(false)
    } else if first_line.contains("POST /save") {
        // 解析 body 并保存
        let body_start = buffer
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .unwrap_or(0)
            + 4;
        let body = String::from_utf8_lossy(&buffer[body_start..]);
        let hooks: Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;

        let mut settings = read_settings();
        if let Value::Object(ref mut map) = settings {
            map.insert("hooks".to_string(), hooks);
        }
        write_settings(&settings)?;

        write_response(stream, 200, "application/json", "{\"ok\":true}")?;
        Ok(true) // 关闭服务
    } else {
        write_response(stream, 404, "text/plain", "Not Found")?;
        Ok(false)
    }
}

fn write_response(
    stream: &mut TcpStream,
    code: u16,
    content_type: &str,
    body: &str,
) -> Result<(), String> {
    let response = format!(
        "HTTP/1.1 {code} OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .map_err(|e| e.to_string())?;
    stream.flush().map_err(|e| e.to_string())?;
    Ok(())
}

fn render_hooks_html() -> String {
    let settings = read_settings();
    let current_hooks = settings.get("hooks").cloned().unwrap_or(json!({}));

    let mut html = String::new();
    push_line(&mut html, "<!doctype html>");
    push_line(&mut html, "<html lang=\"zh-CN\">");
    push_line(&mut html, "<head>");
    push_line(&mut html, "<meta charset=\"utf-8\">");
    push_line(
        &mut html,
        "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">",
    );
    push_line(&mut html, "<title>Clash Hooks 编辑器</title>");
    push_line(&mut html, "<style>");
    push_line(&mut html, HTML_STYLE);
    push_line(&mut html, "</style>");
    push_line(&mut html, "</head>");
    push_line(&mut html, "<body>");
    push_line(&mut html, "<main>");
    push_line(&mut html, "<h1>Clash Hooks 编辑器</h1>");
    push_line(
        &mut html,
        "<p class=\"hint\">按分类编辑 Hook，点击编辑有示例参考</p>",
    );

    // 分类导航
    push_line(&mut html, "<nav class=\"categories\">");
    for (idx, (category, _)) in HOOK_CATEGORIES.iter().enumerate() {
        let class = if idx == 0 {
            "category active"
        } else {
            "category"
        };
        push_line(
            &mut html,
            &format!("<button class=\"{class}\" data-category=\"{idx}\">{category}</button>"),
        );
    }
    push_line(&mut html, "</nav>");

    // 分类内容区域
    push_line(&mut html, "<section id=\"category-content\">");
    for (cat_idx, (category, hooks_in_cat)) in HOOK_CATEGORIES.iter().enumerate() {
        push_line(
            &mut html,
            &format!(
                "<div class=\"category-panel\" data-category=\"{cat_idx}\" style=\"display: {}\">",
                if cat_idx == 0 { "block" } else { "none" }
            ),
        );
        push_line(&mut html, &format!("<h2>{category}</h2>"));

        // 该分类下的 hook 类型表格
        push_line(&mut html, "<table class=\"hook-table\">");
        push_line(
            &mut html,
            "<thead><tr><th>Hook 类型</th><th>说明</th><th>状态</th><th>操作</th></tr></thead>",
        );
        push_line(&mut html, "<tbody>");

        for (hook_type, description) in hooks_in_cat.iter() {
            let has_hooks = current_hooks.get(hook_type).is_some();
            let status = if has_hooks { "已配置" } else { "未配置" };
            let status_class = if has_hooks {
                "status-active"
            } else {
                "status-empty"
            };

            push_line(&mut html, "<tr>");
            push_line(
                &mut html,
                &format!(
                    "<td><button class=\"hook-name\" data-type=\"{}\">{}</button></td>",
                    hook_type, hook_type
                ),
            );
            push_line(
                &mut html,
                &format!("<td class=\"hook-desc\">{description}</td>"),
            );
            push_line(
                &mut html,
                &format!("<td class=\"{status_class}\">{status}</td>"),
            );
            push_line(
                &mut html,
                &format!(
                    "<td><button class=\"edit-hook\" data-type=\"{}\">编辑</button></td>",
                    hook_type
                ),
            );
            push_line(&mut html, "</tr>");
        }

        push_line(&mut html, "</tbody>");
        push_line(&mut html, "</table>");
        push_line(&mut html, "</div>");
    }
    push_line(&mut html, "</section>");

    // Hooks 编辑弹窗
    push_line(
        &mut html,
        "<div class=\"modal\" id=\"edit-modal\" style=\"display: none\">",
    );
    push_line(&mut html, "<div class=\"modal-content\">");
    push_line(&mut html, "<div class=\"modal-header\">");
    push_line(&mut html, "<h3 id=\"modal-title\"></h3>");
    push_line(&mut html, "<p id=\"modal-desc\" class=\"hook-desc\"></p>");
    push_line(
        &mut html,
        "<button class=\"modal-close\" id=\"close-modal\">×</button>",
    );
    push_line(&mut html, "</div>");
    push_line(&mut html, "<div class=\"modal-body\">");
    push_line(
        &mut html,
        "<div class=\"hook-examples\" id=\"hook-examples\"></div>",
    );
    push_line(&mut html, "<div id=\"hooks-list\"></div>");
    push_line(
        &mut html,
        "<button class=\"add-hook\" id=\"add-hook\">+ 添加 Hook Entry</button>",
    );
    push_line(&mut html, "</div>");
    push_line(&mut html, "<div class=\"modal-footer\">");
    push_line(
        &mut html,
        "<button class=\"save-btn\" id=\"save-hook\">保存</button>",
    );
    push_line(&mut html, "</div>");
    push_line(&mut html, "</div>");
    push_line(&mut html, "</div>");

    // 全局保存按钮
    push_line(&mut html, "<section class=\"global-save\">");
    push_line(
        &mut html,
        "<button id=\"save-all-btn\" class=\"save-btn\">保存全部配置</button>",
    );
    push_line(&mut html, "</section>");

    push_line(&mut html, "</main>");
    push_line(&mut html, "<script>");
    let dont_rm_command = format!(
        "sh {}",
        shell_single_quote(&dont_rm_script_path().to_string_lossy())
    );
    let notify_command = format!(
        "sh {}",
        shell_single_quote(&notify_script_path().to_string_lossy())
    );
    let backup_file_command = format!(
        "sh {}",
        shell_single_quote(&backup_file_script_path().to_string_lossy())
    );
    let hook_detail_command = format!(
        "sh {}",
        shell_single_quote(&hook_detail_script_path().to_string_lossy())
    );
    let execute_confirm_command = format!(
        "sh {}",
        shell_single_quote(&execute_confirm_script_path().to_string_lossy())
    );
    push_line(
        &mut html,
        &format!(
            "const DONT_RM_SCRIPT_COMMAND = {};",
            js_string_literal(&dont_rm_command)
        ),
    );
    push_line(
        &mut html,
        &format!(
            "const NOTIFY_SCRIPT_COMMAND = {};",
            js_string_literal(&notify_command)
        ),
    );
    push_line(
        &mut html,
        &format!(
            "const BACKUP_FILE_SCRIPT_COMMAND = {};",
            js_string_literal(&backup_file_command)
        ),
    );
    push_line(
        &mut html,
        &format!(
            "const HOOK_DETAIL_SCRIPT_COMMAND = {};",
            js_string_literal(&hook_detail_command)
        ),
    );
    push_line(
        &mut html,
        &format!(
            "const EXECUTE_CONFIRM_SCRIPT_COMMAND = {};",
            js_string_literal(&execute_confirm_command)
        ),
    );
    push_line(&mut html, HTML_SCRIPT_TEMPLATE);
    push_line(&mut html, "</script>");
    push_line(&mut html, "</body>");
    push_line(&mut html, "</html>");

    html
}

fn push_line(html: &mut String, line: &str) {
    html.push_str(line);
    html.push('\n');
}

fn js_string_literal(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

const DONT_RM_SCRIPT: &str = r#"#!/bin/sh
set -eu

payload_file="$(mktemp)"
trap 'rm -f "$payload_file"' EXIT
cat > "$payload_file"

python3 - "$payload_file" <<'PY'
import datetime
import json
import os
import pathlib
import shlex
import shutil
import sys

with open(sys.argv[1], "r", encoding="utf-8") as payload_handle:
    payload = json.load(payload_handle)

command = (payload.get("tool_input") or {}).get("command") or ""

try:
    parts = shlex.split(command)
except ValueError:
    sys.exit(0)

if not parts or parts[0] != "rm":
    sys.exit(0)

targets = [part for part in parts[1:] if not part.startswith("-")]
if not targets:
    sys.exit(0)

cwd = pathlib.Path(payload.get("cwd") or os.getcwd())
session_id = str(payload.get("session_id") or "unknown-session")
stamp = datetime.datetime.now().strftime("%Y-%m-%d-H-%H-%M-%S")
trash_dir = cwd / ".trash" / session_id / stamp
trash_dir.mkdir(parents=True, exist_ok=True)

def next_path(path):
    if not path.exists():
        return path
    index = 1
    while True:
        candidate = path.with_name(path.name + "." + str(index))
        if not candidate.exists():
            return candidate
        index += 1

moved = []
for target in targets:
    source = pathlib.Path(target)
    if not source.is_absolute():
        source = cwd / source
    if not source.exists():
        continue
    dest = next_path(trash_dir / source.name)
    shutil.move(str(source), str(dest))
    moved.append((source, dest))

if moved:
    print("已拦截 rm，目标已移入 " + str(trash_dir), file=sys.stderr)
    for source, dest in moved:
        print(str(source) + " -> " + str(dest), file=sys.stderr)
    sys.exit(2)
PY
"#;

const NOTIFY_SCRIPT: &str = r#"#!/bin/sh
set -eu

title="${CLASH_NOTIFY_TITLE:-Claude 回复完成}"
sound="${CLASH_NOTIFY_SOUND:-/System/Library/Sounds/Glass.aiff}"

if command -v afplay >/dev/null 2>&1 && [ -f "$sound" ]; then
  afplay "$sound" >/dev/null 2>&1 &
fi
"#;

const BACKUP_FILE_SCRIPT: &str = r#"#!/bin/sh
set -eu

payload_file="$(mktemp)"
trap 'rm -f "$payload_file"' EXIT
cat > "$payload_file"

python3 - "$payload_file" <<'PY'
import datetime
import json
import os
import pathlib
import shutil
import sys

with open(sys.argv[1], "r", encoding="utf-8") as payload_handle:
    payload = json.load(payload_handle)

def first_string(*values):
    for value in values:
        if isinstance(value, str) and value:
            return value
    return ""

tool_input = payload.get("tool_input") or {}
file_path = first_string(
    payload.get("file_path"),
    payload.get("filePath"),
    payload.get("path"),
    payload.get("file"),
    tool_input.get("file_path"),
    tool_input.get("filePath"),
    tool_input.get("path"),
    tool_input.get("file"),
)

if not file_path:
    sys.exit(0)

cwd = pathlib.Path(payload.get("cwd") or os.getcwd())
source = pathlib.Path(file_path)
if not source.is_absolute():
    source = cwd / source
source = source.resolve()

if not source.is_file():
    sys.exit(0)

session_id = str(payload.get("session_id") or "unknown-session")
stamp = datetime.datetime.now().strftime("%Y-%m-%d-H-%H-%M-%S")

try:
    relative = source.relative_to(cwd.resolve())
except ValueError:
    relative = pathlib.Path(source.name)

dest = cwd / ".backup" / session_id / stamp / relative
if dest.exists():
    sys.exit(0)

dest.parent.mkdir(parents=True, exist_ok=True)
shutil.copy2(source, dest)
print("已备份文件 " + str(source) + " -> " + str(dest), file=sys.stderr)
PY
"#;

const HOOK_DETAIL_SCRIPT: &str = r#"#!/bin/sh
set -eu

log_dir="$HOME/.config/clash/logs"
log_file="$log_dir/hooks_detail.log"
mkdir -p "$log_dir"

{
  printf '\n===== %s =====\n' "$(date '+%Y-%m-%d %H:%M:%S')"
  cat
  printf '\n'
} >> "$log_file"
"#;

const EXECUTE_CONFIRM_SCRIPT: &str = r#"#!/bin/sh
set -eu

cat >/dev/null
printf '%s\n' '说出你的理解和接下来的动作，确认后执行'
"#;

const HTML_STYLE: &str = r#"
:root {
  color-scheme: light dark;
  --bg: #1a1f2e;
  --bg-deep: #141822;
  --panel: #232a3b;
  --panel-strong: #20273a;
  --panel-hover: #2a3347;
  --section: #171f30;
  --section-strong: #151c2a;
  --card: #283044;
  --card-border: #3a4560;
  --readable-card: #1f2937;
  --readable-code: #020617;
  --readable-surface: #1b2537;
  --editor-card: #283044;
  --text: #c8d0e0;
  --text-muted: #8895a8;
  --accent: #5a7aa0;
  --accent-soft: #4a6588;
  --accent-hover: #6a8ab8;
  --strong: #60a5fa;
  --border: #3a4560;
  --border-soft: #2a3347;
  --success: #5a8a6a;
  --danger: #8a5a5a;
  --shadow: 0 14px 38px rgba(4, 8, 18, 0.2);
  --shadow-soft: 0 10px 28px rgba(4, 8, 18, 0.18);
  --radius-lg: 16px;
  --radius-md: 12px;
  --radius-sm: 8px;
}
* {
  box-sizing: border-box;
}
html {
  min-height: 100%;
}
body {
  min-height: 100%;
  margin: 0;
  background:
    radial-gradient(circle at 16% 0%, rgba(90, 122, 160, 0.1), transparent 34%),
    radial-gradient(circle at 82% 12%, rgba(74, 101, 136, 0.08), transparent 30%),
    linear-gradient(180deg, #172033 0%, #141c2b 52%, var(--bg-deep) 100%);
  color: var(--text);
  font: 14px/1.6 -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
  letter-spacing: 0.02em;
}
main {
  max-width: 1180px;
  margin: 0 auto;
  padding: 48px 24px 56px;
}
h1 {
  margin: 0 0 8px;
  font-size: clamp(28px, 4vw, 40px);
  font-weight: 700;
  color: var(--text);
  letter-spacing: 0.01em;
  line-height: 1.15;
}
h2 {
  margin: 0 0 18px;
  font-size: 18px;
  font-weight: 650;
  color: var(--text);
  letter-spacing: 0.02em;
}
h3 {
  margin: 0 0 10px;
  font-size: 15px;
  font-weight: 500;
  color: var(--text);
}
.hint {
  max-width: 560px;
  margin: 0 0 26px;
  color: var(--text-muted);
  font-size: 13px;
}
.hook-desc {
  color: var(--text-muted);
  font-size: 12px;
  line-height: 1.5;
}
/* 分类导航 */
.categories {
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
  margin: 28px 0 22px;
  padding: 10px;
  background: linear-gradient(180deg, var(--section), var(--section-strong));
  border: 1px solid var(--border-soft);
  border-radius: var(--radius-lg);
  box-shadow: var(--shadow-soft);
  backdrop-filter: blur(14px);
}
.category {
  padding: 9px 15px;
  background: var(--readable-card);
  color: var(--text-muted);
  border: 1px solid var(--border);
  border-radius: 999px;
  cursor: pointer;
  font-size: 13px;
  font-weight: 600;
  letter-spacing: 0.02em;
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.03);
  transition: transform 0.16s ease, background 0.16s ease, border-color 0.16s ease, color 0.16s ease;
}
.category:hover {
  background: var(--readable-card);
  border-color: var(--strong);
  color: var(--text);
  transform: translateY(-1px);
}
.category.active {
  background: rgba(74, 101, 136, 0.52);
  border-color: var(--strong);
  color: #dbe7f6;
  box-shadow: 0 8px 22px rgba(4, 8, 18, 0.22);
}
/* 分类内容 */
#category-content {
  margin: 22px 0;
}
.category-panel {
  padding: 22px;
  background: linear-gradient(180deg, var(--section), var(--section-strong));
  border: 1px solid var(--border-soft);
  border-radius: var(--radius-lg);
  box-shadow: var(--shadow);
  overflow: hidden;
}
/* 表格样式 */
.hook-table {
  width: 100%;
  min-width: 760px;
  table-layout: fixed;
  border-collapse: separate;
  border-spacing: 0;
  overflow: hidden;
  font-size: 13px;
}
.hook-table th,
.hook-table td {
  padding: 14px 16px;
  text-align: left;
  border-bottom: 1px solid var(--border-soft);
}
.hook-table th {
  color: var(--text-muted);
  font-size: 11px;
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 0.08em;
  background: var(--readable-surface);
}
.hook-table th:first-child {
  border-top-left-radius: var(--radius-sm);
}
.hook-table th:last-child {
  border-top-right-radius: var(--radius-sm);
}
.hook-table tr:hover td {
  background: var(--readable-card);
}
.hook-table th:first-child,
.hook-table td:first-child {
  width: 175px;
}
.hook-table th:nth-child(3),
.hook-table td:nth-child(3) {
  width: 92px;
  text-align: center;
}
.hook-table th:nth-child(4),
.hook-table td:nth-child(4) {
  width: 88px;
  text-align: center;
}
.hook-name {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 100%;
  padding: 7px 11px;
  background: var(--readable-card);
  color: var(--strong);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  cursor: pointer;
  font-size: 13px;
  font-weight: 500;
  letter-spacing: 0.02em;
  transition: transform 0.16s ease, background 0.16s ease, color 0.16s ease, border-color 0.16s ease;
}
.hook-name:hover {
  background: var(--readable-card);
  color: var(--strong);
  border-color: var(--strong);
  transform: translateY(-1px);
}
.status-active {
  color: #b7dcc2;
  font-weight: 700;
  font-size: 12px;
  white-space: nowrap;
}
.status-empty {
  color: var(--text-muted);
  font-size: 12px;
  white-space: nowrap;
}
.status-active::before,
.status-empty::before {
  content: "";
  display: inline-block;
  width: 6px;
  height: 6px;
  margin-right: 6px;
  border-radius: 999px;
  vertical-align: 1px;
}
.status-active::before {
  background: var(--success);
  box-shadow: 0 0 0 4px rgba(90, 138, 106, 0.12);
}
.status-empty::before {
  background: var(--text-muted);
  opacity: 0.58;
}
.edit-hook {
  padding: 7px 12px;
  background: rgba(74, 101, 136, 0.52);
  color: #dbe7f6;
  border: none;
  border-radius: var(--radius-sm);
  cursor: pointer;
  font-size: 12px;
  font-weight: 700;
  box-shadow: 0 8px 18px rgba(74, 101, 136, 0.24);
  transition: transform 0.16s ease, filter 0.16s ease;
}
.edit-hook:hover {
  filter: brightness(1.12);
  transform: translateY(-1px);
}
/* 弹窗样式 */
.modal {
  position: fixed;
  inset: 0;
  z-index: 100;
  display: flex;
  align-items: stretch;
  justify-content: flex-end;
  padding: 0;
  background: rgba(10, 12, 18, 0);
  backdrop-filter: blur(12px);
  transition: background 180ms ease;
}
.modal.open {
  background: rgba(10, 12, 18, 0.72);
}
.modal-content {
  display: flex;
  flex-direction: column;
  width: min(680px, 92vw);
  height: 100%;
  background: linear-gradient(180deg, var(--panel), var(--panel-strong));
  border: 1px solid var(--border-soft);
  border-top: none;
  border-right: none;
  border-bottom: none;
  border-radius: 0;
  overflow: hidden;
  box-shadow: 0 28px 80px rgba(0, 0, 0, 0.48);
  transform: translateX(100%);
  transition: transform 220ms ease;
}
.modal.open .modal-content {
  transform: translateX(0);
}
.modal-header {
  padding: 22px 26px;
  background: var(--readable-surface);
  border-bottom: 1px solid var(--border-soft);
  position: relative;
}
.modal-header h3 {
  margin: 0 0 8px;
  font-size: 18px;
  font-weight: 600;
  letter-spacing: 0.03em;
  color: var(--strong);
}
#modal-desc {
  max-width: 560px;
  margin: 0;
  color: var(--text-muted);
  font-size: 13px;
  line-height: 1.5;
}
.modal-close {
  position: absolute;
  top: 18px;
  right: 20px;
  width: 32px;
  height: 32px;
  background: var(--readable-card);
  color: var(--text-muted);
  border: 1px solid var(--border-soft);
  border-radius: 999px;
  cursor: pointer;
  font-size: 18px;
  line-height: 1;
  transition: background 0.16s ease, color 0.16s ease, border-color 0.16s ease, transform 0.16s ease;
}
.modal-close:hover {
  color: var(--text);
  border-color: var(--strong);
  background: var(--readable-card);
  transform: rotate(90deg);
}
.modal-body {
  flex: 1;
  padding: 22px;
  overflow-y: auto;
}
.modal-footer {
  padding: 18px 22px;
  background: var(--readable-surface);
  border-top: 1px solid var(--border-soft);
}
/* Hook entry 样式 */
.hook-entry {
  margin: 14px 0;
  padding: 16px;
  background: var(--editor-card);
  border: 1px solid var(--card-border);
  border-radius: var(--radius-md);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.03);
}
.matcher-row, .type-row, .config-row {
  display: flex;
  align-items: center;
  gap: 12px;
  margin: 8px 0;
}
.matcher-row label, .type-row label, .config-row label {
  min-width: 76px;
  color: var(--text-muted);
  font-size: 12px;
  font-weight: 700;
  letter-spacing: 0.02em;
}
.matcher-input, .type-select, .config-input {
  flex: 1;
  min-width: 0;
  padding: 10px 12px;
  background: var(--readable-surface);
  color: var(--text);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  font-size: 13px;
  transition: background 0.16s ease, border-color 0.16s ease, box-shadow 0.16s ease;
}
.config-textarea {
  flex: 1;
  min-width: 0;
  padding: 11px 12px;
  background: var(--readable-surface);
  color: var(--text);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  font-size: 13px;
  min-height: 84px;
  resize: vertical;
  line-height: 1.5;
  transition: background 0.16s ease, border-color 0.16s ease, box-shadow 0.16s ease;
}
.matcher-input:focus, .type-select:focus, .config-input:focus, .config-textarea:focus {
  outline: none;
  border-color: var(--strong);
  background: var(--readable-surface);
  box-shadow: 0 0 0 3px rgba(96, 165, 250, 0.12);
}
.matcher-input::placeholder, .config-input::placeholder, .config-textarea::placeholder {
  color: var(--text-muted);
  opacity: 0.6;
}
.hook-examples {
  display: none;
  margin: 0 0 16px;
  padding: 12px;
  background: var(--readable-surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
}
.hook-examples.is-visible {
  display: block;
}
.examples-title {
  margin: 0 0 10px;
  color: var(--text-muted);
  font-size: 12px;
  font-weight: 700;
}
.example-card {
  display: block;
  width: 100%;
  margin: 8px 0;
  padding: 10px 12px;
  background: var(--editor-card);
  color: var(--text);
  border: 1px solid var(--card-border);
  border-radius: var(--radius-sm);
  cursor: pointer;
  text-align: left;
  transition: border-color 0.16s ease, transform 0.16s ease;
}
.example-card:hover {
  border-color: var(--strong);
  transform: translateY(-1px);
}
.example-card strong {
  display: block;
  margin-bottom: 4px;
  color: var(--strong);
  font-size: 13px;
}
.example-card span {
  color: var(--text-muted);
  font-size: 12px;
  line-height: 1.5;
}
.hooks-list {
  margin: 10px 0;
}
.hook-item {
  margin: 10px 0;
  padding: 12px;
  background: var(--readable-surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
}
.type-hint {
  font-size: 11px;
  color: var(--text-muted);
  margin: 4px 0 8px;
  opacity: 0.8;
}
.type-config {
  margin: 6px 0;
}
.add-hook {
  padding: 9px 15px;
  background: var(--editor-card);
  color: var(--strong);
  border: 1px solid var(--card-border);
  border-radius: var(--radius-sm);
  cursor: pointer;
  font-size: 12px;
  font-weight: 700;
  transition: background 0.16s ease, border-color 0.16s ease, color 0.16s ease, transform 0.16s ease;
}
.add-hook:hover {
  background: var(--editor-card);
  border-color: var(--strong);
  color: var(--strong);
  transform: translateY(-1px);
}
.add-hook-item {
  padding: 7px 12px;
  background: transparent;
  color: var(--strong);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  cursor: pointer;
  font-size: 12px;
  font-weight: 700;
  transition: background 0.16s ease, color 0.16s ease, transform 0.16s ease;
}
.add-hook-item:hover {
  background: rgba(74, 101, 136, 0.52);
  color: #dbe7f6;
  border-color: var(--strong);
  transform: translateY(-1px);
}
.remove-hook, .remove-hook-item {
  padding: 7px 11px;
  border-radius: var(--radius-sm);
  cursor: pointer;
  font-size: 12px;
  font-weight: 700;
  transition: background 0.16s ease, color 0.16s ease, border-color 0.16s ease, transform 0.16s ease;
}
.remove-hook {
  background: transparent;
  color: var(--text-muted);
  border: 1px solid var(--border);
}
.remove-hook:hover {
  background: rgba(74, 101, 136, 0.52);
  color: #dbe7f6;
  border-color: var(--strong);
  transform: translateY(-1px);
}
.remove-hook-item {
  background: rgba(138, 90, 90, 0.08);
  color: var(--danger);
  border: 1px solid rgba(138, 90, 90, 0.3);
}
.remove-hook-item:hover {
  background: var(--danger);
  color: #f0e8e8;
  border-color: var(--danger);
  transform: translateY(-1px);
}
.save-btn {
  display: block;
  width: 100%;
  padding: 13px;
  background: rgba(74, 101, 136, 0.52);
  color: #dbe7f6;
  border: none;
  border-radius: var(--radius-sm);
  cursor: pointer;
  font-size: 14px;
  font-weight: 700;
  letter-spacing: 0.03em;
  box-shadow: 0 10px 24px rgba(74, 101, 136, 0.26);
  transition: transform 0.16s ease, filter 0.16s ease, box-shadow 0.16s ease;
}
.save-btn:hover {
  filter: brightness(1.12);
  box-shadow: 0 14px 30px rgba(4, 8, 18, 0.28);
  transform: translateY(-1px);
}
.global-save {
  margin: 26px 0 0;
  padding: 16px;
  background: linear-gradient(180deg, var(--section), var(--section-strong));
  border: 1px solid var(--border-soft);
  border-radius: var(--radius-lg);
  box-shadow: var(--shadow-soft);
}
/* 滚动条 */
::-webkit-scrollbar {
  width: 8px;
  height: 8px;
}
::-webkit-scrollbar-track {
  background: rgba(26, 31, 46, 0.5);
}
::-webkit-scrollbar-thumb {
  background: var(--border);
  border-radius: 999px;
}
::-webkit-scrollbar-thumb:hover {
  background: var(--accent-soft);
}
@media (max-width: 760px) {
  main {
    padding: 32px 14px 40px;
  }
  .category-panel {
    padding: 16px;
    overflow-x: auto;
  }
  .modal {
    padding: 0;
  }
  .modal-content {
    width: 100%;
  }
  .matcher-row, .type-row, .config-row {
    align-items: stretch;
    flex-direction: column;
    gap: 6px;
  }
  .matcher-row label, .type-row label, .config-row label {
    min-width: 0;
  }
}
"#;

const HTML_SCRIPT_TEMPLATE: &str = r#"
// Hook type 配置说明
const HOOK_TYPE_CONFIG = {
  command: { label: '命令', fields: ['command'], hint: '执行命令' },
  prompt: { label: '提示词', fields: ['prompt', 'model'], hint: '仅适用于 Stop/SubagentStop' },
  agent: { label: '代理', fields: ['agent', 'model'], hint: '仅适用于 Stop/SubagentStop' },
  mcp_tool: { label: 'MCP 工具', fields: ['mcp_tool', 'arguments'], hint: '调用 MCP 工具' },
  http: { label: 'HTTP', fields: ['url', 'method', 'headers', 'body'], hint: '发送 HTTP 请求' }
};

// 常用示例
const HOOK_EXAMPLES = {
  PreToolUse: [
    {
      title: 'rm 移入 .trash',
      description: '拦截 Bash 的 rm，把目标移动到 ./.trash/sessionId/YYYY-MM-DD-H-HH-MM-SS/',
      entry: {
        matcher: 'Bash',
        hooks: [
          {
            type: 'command',
            command: DONT_RM_SCRIPT_COMMAND
          }
        ]
      }
    },
    {
      title: '修改文件自动备份',
      description: 'Write/Edit/MultiEdit 执行前备份原文件到 ./.backup/sessionId/YYYY-MM-DD-H-HH-MM-SS/',
      entries: ['Write', 'Edit', 'MultiEdit'].map((matcher) => ({
        matcher,
        hooks: [
          {
            type: 'command',
            command: BACKUP_FILE_SCRIPT_COMMAND
          }
        ]
      }))
    }
  ],
  PostToolUse: [
    {
      title: '记录工具 Hook payload',
      description: '把工具调用后的完整输入写入 ~/.config/clash/logs/hooks_detail.log',
      entry: {
        hooks: [
          {
            type: 'command',
            command: HOOK_DETAIL_SCRIPT_COMMAND
          }
        ]
      }
    }
  ],
  PermissionRequest: [
    {
      title: '记录权限请求 payload',
      description: '把权限请求完整输入写入 ~/.config/clash/logs/hooks_detail.log',
      entry: {
        hooks: [
          {
            type: 'command',
            command: HOOK_DETAIL_SCRIPT_COMMAND
          }
        ]
      }
    }
  ],
  UserPromptSubmit: [
    {
      title: '执行确认',
      description: '提交提示词时追加：说出你的理解和接下来的动作，确认后执行',
      entry: {
        hooks: [
          {
            type: 'command',
            command: EXECUTE_CONFIRM_SCRIPT_COMMAND
          }
        ]
      }
    }
  ],
  Stop: [
    {
      title: '回复结束提醒',
      description: '每轮回复结束后播放系统音效',
      entry: {
        hooks: [
          {
            type: 'command',
            command: NOTIFY_SCRIPT_COMMAND
          }
        ]
      }
    },
    {
      title: '回复结束后运行项目检查',
      description: '每轮回复结束后执行项目内 ./tools/check.sh，减少无意义token消耗',
      entry: {
        hooks: [
          {
            type: 'command',
            command: 'sh ./tools/check.sh'
          }
        ]
      }
    }
  ],
};

// 当前编辑的 hook 数据
let currentHooksData = {};
let currentEditType = '';

// 初始化数据
fetch('/data')
  .then(r => r.json())
  .then(data => { currentHooksData = data; })
  .catch(() => { currentHooksData = {}; });

// 分类切换
const categories = document.querySelectorAll('.category');
const categoryPanels = document.querySelectorAll('.category-panel');

categories.forEach(btn => {
  btn.addEventListener('click', () => {
    const idx = btn.dataset.category;
    categories.forEach(b => b.classList.remove('active'));
    btn.classList.add('active');
    categoryPanels.forEach(p => {
      p.style.display = p.dataset.category === idx ? 'block' : 'none';
    });
  });
});

// 编辑按钮点击
document.querySelectorAll('.edit-hook').forEach(btn => {
  btn.addEventListener('click', () => {
    currentEditType = btn.dataset.type;
    openModal(currentEditType);
  });
});

// Hook 名称点击也可以编辑
document.querySelectorAll('.hook-name').forEach(btn => {
  btn.addEventListener('click', () => {
    currentEditType = btn.dataset.type;
    openModal(currentEditType);
  });
});

// 打开弹窗
function openModal(type) {
  const modal = document.getElementById('edit-modal');
  const title = document.getElementById('modal-title');
  const desc = document.getElementById('modal-desc');
  const list = document.getElementById('hooks-list');

  const hookInfo = getHookInfo(type);
  title.textContent = type;
  desc.textContent = hookInfo.description;
  renderHookExamples(type);

  const hooksArray = currentHooksData[type] || [];
  list.innerHTML = '';
  hooksArray.forEach((entry, idx) => {
    addHookEntry(list, entry, idx, type);
  });

  modal.classList.remove('open');
  modal.style.display = 'flex';
  modal.offsetHeight;
  modal.classList.add('open');
}

// 获取 hook 信息
function getHookInfo(type) {
  const allHooks = [
    ["PreToolUse","工具调用前执行，可阻止操作"],
    ["PostToolUse","工具调用后执行，可处理结果"],
    ["PermissionRequest","权限请求时执行"],
    ["PermissionDenied","权限被拒绝时执行"],
    ["SessionStart","会话开始时执行"],
    ["SessionEnd","会话结束时执行"],
    ["Stop","会话停止时执行"],
    ["StopFailure","API 错误导致停止时执行"],
    ["Setup","通过 --init/--maintenance 触发"],
    ["SubagentStart","子代理启动时执行"],
    ["SubagentStop","子代理停止时执行"],
    ["TeammateIdle","队友进入空闲状态时执行"],
    ["TaskCreated","任务创建时执行"],
    ["TaskCompleted","任务完成时执行"],
    ["MessageDisplay","消息显示时可转换/隐藏内容"],
    ["UserPromptSubmit","用户提交提示词时执行"],
    ["Notification","通知事件触发时执行"],
    ["FileChanged","文件变更时执行"],
    ["CwdChanged","工作目录变更时执行"],
    ["InstructionsLoaded","CLAUDE.md/rules 加载时执行"],
    ["ConfigChange","配置文件变更时执行"],
    ["WorktreeCreate","创建 worktree 时执行"],
    ["WorktreeRemove","删除 worktree 时执行"],
    ["PreCompact","上下文压缩前执行"],
    ["PostCompact","上下文压缩后执行"],
    ["Elicitation","用户交互请求时执行"],
    ["ElicitationResult","用户交互结果时执行"]
  ];
  const found = allHooks.find(h => h[0] === type);
  return found ? { name: found[0], description: found[1] } : { name: type, description: '' };
}

// 渲染当前 Hook 的示例
function renderHookExamples(type) {
  const container = document.getElementById('hook-examples');
  const examples = HOOK_EXAMPLES[type] || [];
  if (examples.length === 0) {
    container.classList.remove('is-visible');
    container.innerHTML = '';
    return;
  }

  container.classList.add('is-visible');
  container.innerHTML = `
    <p class="examples-title">示例</p>
    ${examples.map((example, idx) => `
      <button class="example-card" type="button" data-example-index="${idx}">
        <strong>${escapeHtml(example.title)}</strong>
        <span>${escapeHtml(example.description)}</span>
      </button>
    `).join('')}
  `;

  container.querySelectorAll('[data-example-index]').forEach(btn => {
    btn.addEventListener('click', () => {
      addHookExample(type, Number(btn.dataset.exampleIndex));
    });
  });
}

// 追加示例配置，不覆盖当前内容
function addHookExample(type, idx) {
  const example = (HOOK_EXAMPLES[type] || [])[idx];
  if (!example) return;
  const list = document.getElementById('hooks-list');
  const entries = example.entries || [example.entry];
  entries.forEach((rawEntry) => {
    const entry = JSON.parse(JSON.stringify(rawEntry));
    addHookEntry(list, entry, list.children.length, type);
  });
}

// 添加 hook entry
function addHookEntry(container, entry, idx, hookType) {
  const div = document.createElement('div');
  div.className = 'hook-entry';
  div.dataset.index = idx;

  const matcher = entry.matcher || '';
  const hooks = entry.hooks || [];

  // 检查是否需要 matcher (SessionStart 等)
  const needsMatcher = !['SessionStart', 'SessionEnd', 'Setup', 'SubagentStart', 'Notification', 'UserPromptSubmit', 'StopFailure', 'TeammateIdle', 'TaskCreated', 'TaskCompleted', 'FileChanged', 'PreCompact', 'PostCompact', 'Elicitation', 'ElicitationResult'].includes(hookType);

  div.innerHTML = `
    ${needsMatcher ? `
      <div class="matcher-row">
        <label>Matcher:</label>
        <input type="text" class="matcher-input" value="${escapeHtml(matcher)}" placeholder="工具名称 (如 Bash)" />
      </div>
    ` : ''}
    <div class="hooks-items">
      ${hooks.map((hook, hidx) => renderHookItem(hook, hidx, hookType)).join('')}
    </div>
    <button class="add-hook-item">+ 添加 Hook</button>
    <button class="remove-hook">删除此 Entry</button>
  `;

  container.appendChild(div);

  // 绑定事件
  div.querySelector('.add-hook-item').addEventListener('click', () => {
    const items = div.querySelector('.hooks-items');
    const newIdx = items.children.length;
    const newDiv = document.createElement('div');
    newDiv.className = 'hook-item';
    newDiv.dataset.hookIndex = newIdx;
    newDiv.innerHTML = renderHookItem({ type: 'command' }, newIdx, hookType);
    items.appendChild(newDiv);
    bindHookItemEvents(newDiv, hookType);
  });

  div.querySelector('.remove-hook').addEventListener('click', () => div.remove());

  div.querySelectorAll('.hook-item').forEach(item => bindHookItemEvents(item, hookType));
}

// 渲染单个 hook
function renderHookItem(hook, idx, hookType) {
  const type = hook.type || 'command';
  const config = HOOK_TYPE_CONFIG[type] || HOOK_TYPE_CONFIG.command;

  let fieldsHtml = '';
  config.fields.forEach(field => {
    const value = hook[field] || '';
    if (field === 'arguments' || field === 'headers' || field === 'body') {
      const jsonValue = typeof value === 'object' ? JSON.stringify(value, null, 2) : value;
      fieldsHtml += `
        <div class="config-row">
          <label>${field}:</label>
          <textarea class="config-textarea config-${field}" data-field="${field}" placeholder="JSON 格式">${escapeHtml(jsonValue)}</textarea>
        </div>
      `;
    } else {
      fieldsHtml += `
        <div class="config-row">
          <label>${field}:</label>
          <input type="text" class="config-input config-${field}" data-field="${field}" value="${escapeHtml(value)}" placeholder="${getFieldPlaceholder(field)}" />
        </div>
      `;
    }
  });

  return `
    <div class="hook-item" data-hook-index="${idx}">
      <div class="type-row">
        <label>Type:</label>
        <select class="type-select" data-type-select>
          ${Object.entries(HOOK_TYPE_CONFIG).map(([t, c]) => `
            <option value="${t}" ${t === type ? 'selected' : ''} ${!isValidTypeForHook(t, hookType) ? 'disabled' : ''}>${c.label}</option>
          `).join('')}
        </select>
        <button class="remove-hook-item">×</button>
      </div>
      <p class="type-hint">${config.hint}</p>
      <div class="type-config">
        ${fieldsHtml}
      </div>
    </div>
  `;
}

// 检查 type 是否适用于 hook
function isValidTypeForHook(type, hookType) {
  if (type === 'prompt' || type === 'agent') {
    return ['Stop', 'SubagentStop'].includes(hookType);
  }
  return true;
}

// 获取字段 placeholder
function getFieldPlaceholder(field) {
  const placeholders = {
    command: '/path/to/script.sh',
    prompt: '检查是否有未提交的代码',
    model: 'claude-sonnet-4-20250514',
    agent: 'code-reviewer',
    mcp_tool: 'mcp__slack__send_message',
    url: 'https://api.example.com/hook',
    method: 'POST'
  };
  return placeholders[field] || '';
}

// 绑定 hook item 事件
function bindHookItemEvents(item, hookType) {
  const typeSelect = item.querySelector('[data-type-select]');
  typeSelect.addEventListener('change', () => {
    const newType = typeSelect.value;
    const config = HOOK_TYPE_CONFIG[newType];
    const configDiv = item.querySelector('.type-config');
    const hintP = item.querySelector('.type-hint');

    hintP.textContent = config.hint;
    configDiv.innerHTML = config.fields.map(field => {
      if (field === 'arguments' || field === 'headers' || field === 'body') {
        return `
          <div class="config-row">
            <label>${field}:</label>
            <textarea class="config-textarea config-${field}" data-field="${field}" placeholder="JSON 格式"></textarea>
          </div>
        `;
      }
      return `
        <div class="config-row">
          <label>${field}:</label>
          <input type="text" class="config-input config-${field}" data-field="${field}" placeholder="${getFieldPlaceholder(field)}" />
        </div>
      `;
    }).join('');
  });

  item.querySelector('.remove-hook-item').addEventListener('click', () => item.remove());
}

// HTML 转义
function escapeHtml(text) {
  if (!text) return '';
  return String(text).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

// 添加新 hook entry
document.getElementById('add-hook').addEventListener('click', () => {
  const list = document.getElementById('hooks-list');
  addHookEntry(list, { matcher: '', hooks: [{ type: 'command' }] }, list.children.length, currentEditType);
});

// 关闭弹窗
document.getElementById('close-modal').addEventListener('click', closeModal);
document.getElementById('edit-modal').addEventListener('click', (e) => {
  if (e.target.id === 'edit-modal') closeModal();
});

function closeModal() {
  const modal = document.getElementById('edit-modal');
  modal.classList.remove('open');
  setTimeout(() => {
    if (!modal.classList.contains('open')) {
      modal.style.display = 'none';
    }
  }, 220);
}

// 保存当前 hook
document.getElementById('save-hook').addEventListener('click', () => {
  const list = document.getElementById('hooks-list');
  const entries = [];

  list.querySelectorAll('.hook-entry').forEach(entry => {
    const matcherInput = entry.querySelector('.matcher-input');
    const matcher = matcherInput ? matcherInput.value.trim() : '';

    const hooks = [];
    entry.querySelectorAll('.hook-item').forEach(item => {
      const type = item.querySelector('[data-type-select]').value;
      const hookData = { type };

      item.querySelectorAll('.config-input, .config-textarea').forEach(input => {
        const field = input.dataset.field;
        let value = input.value.trim();
        if (field === 'arguments' || field === 'headers' || field === 'body') {
          try { value = JSON.parse(value || '{}'); } catch { value = {}; }
        }
        hookData[field] = value;
      });

      hooks.push(hookData);
    });

    if (hooks.length > 0) {
      if (currentEditType === 'FileChanged' || currentEditType === 'UserPromptSubmit' || (currentEditType === 'PostToolUse' && !matcher)) {
        entries.push({ hooks });
      } else {
        entries.push({ matcher, hooks });
      }
    }
  });

  if (entries.length > 0) {
    currentHooksData[currentEditType] = entries;
  } else {
    delete currentHooksData[currentEditType];
  }

  closeModal();
  updateStatus();
});

// 更新状态显示
function updateStatus() {
  document.querySelectorAll('.edit-hook').forEach(btn => {
    const type = btn.dataset.type;
    const row = btn.closest('tr');
    const statusCell = row.querySelector('.status-active, .status-empty');
    if (currentHooksData[type] && currentHooksData[type].length > 0) {
      statusCell.className = 'status-active';
      statusCell.textContent = '已配置';
    } else {
      statusCell.className = 'status-empty';
      statusCell.textContent = '未配置';
    }
  });
}

// 保存全部配置
document.getElementById('save-all-btn').addEventListener('click', async () => {
  try {
    const res = await fetch('/save', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(currentHooksData)
    });
    const data = await res.json();
    if (data.ok) {
      window.close();
      setTimeout(() => {
        document.body.innerHTML = '<main><h1>Hooks 配置已保存</h1><p class="hint">本地编辑服务已停止，可以关闭此页面</p></main>';
      }, 120);
    }
  } catch (err) {
    alert('保存失败: ' + err.message);
  }
});
"#;
