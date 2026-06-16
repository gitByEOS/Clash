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
    ("工具相关", &[
        ("PreToolUse", "工具调用前执行，可阻止操作"),
        ("PostToolUse", "工具调用后执行，可处理结果"),
        ("PermissionRequest", "权限请求时执行"),
        ("PermissionDenied", "权限被拒绝时执行"),
    ]),
    ("会话生命周期", &[
        ("SessionStart", "会话开始时执行"),
        ("SessionEnd", "会话结束时执行"),
        ("Stop", "会话停止时执行"),
        ("StopFailure", "API 错误导致停止时执行"),
        ("Setup", "通过 --init/--maintenance 触发"),
    ]),
    ("子代理相关", &[
        ("SubagentStart", "子代理启动时执行"),
        ("SubagentStop", "子代理停止时执行"),
        ("TeammateIdle", "队友进入空闲状态时执行"),
        ("TaskCreated", "任务创建时执行"),
        ("TaskCompleted", "任务完成时执行"),
    ]),
    ("消息相关", &[
        ("MessageDisplay", "消息显示时可转换/隐藏内容"),
        ("UserPromptSubmit", "用户提交提示词时执行"),
        ("Notification", "通知事件触发时执行"),
    ]),
    ("文件/环境相关", &[
        ("FileChanged", "文件变更时执行"),
        ("CwdChanged", "工作目录变更时执行"),
        ("InstructionsLoaded", "CLAUDE.md/rules 加载时执行"),
        ("ConfigChange", "配置文件变更时执行"),
    ]),
    ("Git Worktree", &[
        ("WorktreeCreate", "创建 worktree 时执行"),
        ("WorktreeRemove", "删除 worktree 时执行"),
    ]),
    ("压缩相关", &[
        ("PreCompact", "上下文压缩前执行"),
        ("PostCompact", "上下文压缩后执行"),
    ]),
    ("交互相关", &[
        ("Elicitation", "用户交互请求时执行"),
        ("ElicitationResult", "用户交互结果时执行"),
    ]),
];

/// settings.json 路径
pub fn settings_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    PathBuf::from(home).join(".claude/settings.json")
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
            Ok((mut stream, _)) => {
                match handle_stream(&mut stream) {
                    Ok(should_close) => {
                        saved = should_close;
                    }
                    Err(err) => {
                        print_red(&format!("处理请求失败: {err}"));
                    }
                }
            }
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
        let body_start = buffer.windows(4).position(|w| w == b"\r\n\r\n").unwrap_or(0) + 4;
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

fn write_response(stream: &mut TcpStream, code: u16, content_type: &str, body: &str) -> Result<(), String> {
    let response = format!(
        "HTTP/1.1 {code} OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).map_err(|e| e.to_string())?;
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
    push_line(&mut html, "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">");
    push_line(&mut html, "<title>Clash Hooks 编辑器</title>");
    push_line(&mut html, "<style>");
    push_line(&mut html, HTML_STYLE);
    push_line(&mut html, "</style>");
    push_line(&mut html, "</head>");
    push_line(&mut html, "<body>");
    push_line(&mut html, "<main>");
    push_line(&mut html, "<h1>Clash Hooks 编辑器</h1>");
    push_line(&mut html, "<p class=\"hint\">按分类选择 Hook 类型，编辑配置后点击保存</p>");

    // 分类导航
    push_line(&mut html, "<nav class=\"categories\">");
    for (idx, (category, _)) in HOOK_CATEGORIES.iter().enumerate() {
        let class = if idx == 0 { "category active" } else { "category" };
        push_line(&mut html, &format!(
            "<button class=\"{class}\" data-category=\"{idx}\">{category}</button>"
        ));
    }
    push_line(&mut html, "</nav>");

    // 分类内容区域
    push_line(&mut html, "<section id=\"category-content\">");
    for (cat_idx, (category, hooks_in_cat)) in HOOK_CATEGORIES.iter().enumerate() {
        push_line(&mut html, &format!(
            "<div class=\"category-panel\" data-category=\"{cat_idx}\" style=\"display: {}\">",
            if cat_idx == 0 { "block" } else { "none" }
        ));
        push_line(&mut html, &format!("<h2>{category}</h2>"));

        // 该分类下的 hook 类型表格
        push_line(&mut html, "<table class=\"hook-table\">");
        push_line(&mut html, "<thead><tr><th>Hook 类型</th><th>说明</th><th>状态</th><th>操作</th></tr></thead>");
        push_line(&mut html, "<tbody>");

        for (hook_type, description) in hooks_in_cat.iter() {
            let has_hooks = current_hooks.get(hook_type).is_some();
            let status = if has_hooks { "已配置" } else { "未配置" };
            let status_class = if has_hooks { "status-active" } else { "status-empty" };

            push_line(&mut html, "<tr>");
            push_line(&mut html, &format!(
                "<td><button class=\"hook-name\" data-type=\"{}\">{}</button></td>",
                hook_type, hook_type
            ));
            push_line(&mut html, &format!("<td class=\"hook-desc\">{description}</td>"));
            push_line(&mut html, &format!("<td class=\"{status_class}\">{status}</td>"));
            push_line(&mut html, &format!("<td><button class=\"edit-hook\" data-type=\"{}\">编辑</button></td>", hook_type));
            push_line(&mut html, "</tr>");
        }

        push_line(&mut html, "</tbody>");
        push_line(&mut html, "</table>");
        push_line(&mut html, "</div>");
    }
    push_line(&mut html, "</section>");

    // Hooks 编辑弹窗
    push_line(&mut html, "<div class=\"modal\" id=\"edit-modal\" style=\"display: none\">");
    push_line(&mut html, "<div class=\"modal-content\">");
    push_line(&mut html, "<div class=\"modal-header\">");
    push_line(&mut html, "<h3 id=\"modal-title\"></h3>");
    push_line(&mut html, "<p id=\"modal-desc\" class=\"hook-desc\"></p>");
    push_line(&mut html, "<button class=\"modal-close\" id=\"close-modal\">×</button>");
    push_line(&mut html, "</div>");
    push_line(&mut html, "<div class=\"modal-body\">");
    push_line(&mut html, "<div id=\"hooks-list\"></div>");
    push_line(&mut html, "<button class=\"add-hook\" id=\"add-hook\">+ 添加 Hook Entry</button>");
    push_line(&mut html, "</div>");
    push_line(&mut html, "<div class=\"modal-footer\">");
    push_line(&mut html, "<button class=\"save-btn\" id=\"save-hook\">保存</button>");
    push_line(&mut html, "</div>");
    push_line(&mut html, "</div>");
    push_line(&mut html, "</div>");

    // 全局保存按钮
    push_line(&mut html, "<section class=\"global-save\">");
    push_line(&mut html, "<button id=\"save-all-btn\" class=\"save-btn\">保存全部配置</button>");
    push_line(&mut html, "</section>");

    push_line(&mut html, "</main>");
    push_line(&mut html, "<script>");
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

const HTML_STYLE: &str = r#"
:root {
  color-scheme: light dark;
  --bg: #1a1f2e;
  --bg-deep: #141822;
  --panel: #232a3b;
  --panel-hover: #2a3347;
  --card: #283044;
  --card-border: #3a4560;
  --text: #c8d0e0;
  --text-muted: #8895a8;
  --accent: #5a7aa0;
  --accent-soft: #4a6588;
  --accent-hover: #6a8ab8;
  --border: #3a4560;
  --border-soft: #2a3347;
  --success: #5a8a6a;
  --danger: #8a5a5a;
}
* {
  box-sizing: border-box;
}
body {
  margin: 0;
  background: var(--bg-deep);
  color: var(--text);
  font: 14px/1.6 -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
  letter-spacing: 0.02em;
}
main {
  max-width: 1200px;
  margin: 0 auto;
  padding: 40px 24px;
}
h1 {
  margin: 0 0 6px;
  font-size: 26px;
  font-weight: 500;
  color: var(--text);
  letter-spacing: 0.04em;
}
h2 {
  margin: 0 0 12px;
  font-size: 18px;
  font-weight: 500;
  color: var(--text-muted);
  letter-spacing: 0.03em;
}
h3 {
  margin: 0 0 10px;
  font-size: 15px;
  font-weight: 500;
  color: var(--text);
}
.hint {
  margin: 0 0 20px;
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
  gap: 6px;
  flex-wrap: wrap;
  margin: 24px 0;
  padding: 12px;
  background: var(--panel);
  border: 1px solid var(--border-soft);
  border-radius: 8px;
}
.category {
  padding: 8px 14px;
  background: var(--card);
  color: var(--text-muted);
  border: 1px solid var(--card-border);
  border-radius: 4px;
  cursor: pointer;
  font-size: 13px;
  font-weight: 500;
  letter-spacing: 0.02em;
  transition: all 0.2s ease;
}
.category:hover {
  background: var(--panel-hover);
  border-color: var(--accent-soft);
  color: var(--text);
}
.category.active {
  background: var(--accent-soft);
  border-color: var(--accent);
  color: #e8f0f8;
}
/* 分类内容 */
#category-content {
  margin: 20px 0;
}
.category-panel {
  padding: 20px;
  background: var(--panel);
  border: 1px solid var(--border-soft);
  border-radius: 8px;
}
/* 表格样式 */
.hook-table {
  width: 100%;
  table-layout: fixed;
  border-collapse: collapse;
  font-size: 13px;
}
.hook-table th,
.hook-table td {
  padding: 12px 14px;
  text-align: left;
  border-bottom: 1px solid var(--border-soft);
}
.hook-table th {
  color: var(--text-muted);
  font-size: 11px;
  font-weight: 500;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  background: var(--bg);
}
.hook-table tr:hover td {
  background: var(--panel-hover);
}
.hook-table th:first-child,
.hook-table td:first-child {
  width: 160px;
}
.hook-table th:nth-child(3),
.hook-table td:nth-child(3) {
  width: 80px;
}
.hook-table th:nth-child(4),
.hook-table td:nth-child(4) {
  width: 80px;
}
.hook-name {
  display: inline-block;
  width: 100%;
  padding: 5px 10px;
  background: transparent;
  color: #5a9a8a;
  border: 1px solid #4a8a7a;
  border-radius: 3px;
  cursor: pointer;
  font-size: 13px;
  font-weight: 500;
  letter-spacing: 0.02em;
  transition: all 0.15s ease;
}
.hook-name:hover {
  background: #3a7a6a;
  color: #d8f0e8;
}
.status-active {
  color: var(--success);
  font-weight: 500;
}
.status-empty {
  color: var(--text-muted);
}
.edit-hook {
  padding: 5px 10px;
  background: var(--accent-soft);
  color: #e8f0f8;
  border: none;
  border-radius: 3px;
  cursor: pointer;
  font-size: 12px;
  font-weight: 500;
  transition: all 0.15s ease;
}
.edit-hook:hover {
  background: var(--accent);
}
/* 弹窗样式 */
.modal {
  position: fixed;
  inset: 0;
  z-index: 100;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(10, 12, 18, 0.75);
  backdrop-filter: blur(4px);
}
.modal-content {
  width: 94%;
  max-width: 580px;
  max-height: 85vh;
  background: var(--panel);
  border: 1px solid var(--border);
  border-radius: 10px;
  overflow: hidden;
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.4);
}
.modal-header {
  padding: 18px 22px;
  background: var(--bg);
  border-bottom: 1px solid var(--border-soft);
  position: relative;
}
.modal-header h3 {
  margin: 0 0 8px;
  font-size: 18px;
  font-weight: 600;
  letter-spacing: 0.03em;
  color: var(--accent);
}
#modal-desc {
  margin: 0;
  color: var(--text-muted);
  font-size: 13px;
  line-height: 1.5;
}
.modal-close {
  position: absolute;
  top: 14px;
  right: 16px;
  width: 28px;
  height: 28px;
  background: transparent;
  color: var(--text-muted);
  border: 1px solid var(--border-soft);
  border-radius: 4px;
  cursor: pointer;
  font-size: 18px;
  line-height: 1;
  transition: all 0.15s ease;
}
.modal-close:hover {
  color: var(--text);
  border-color: var(--border);
}
.modal-body {
  padding: 20px;
  max-height: 55vh;
  overflow-y: auto;
}
.modal-footer {
  padding: 16px 22px;
  background: var(--bg);
  border-top: 1px solid var(--border-soft);
}
/* Hook entry 样式 */
.hook-entry {
  margin: 12px 0;
  padding: 16px;
  background: var(--card);
  border: 1px solid var(--card-border);
  border-radius: 6px;
  transition: all 0.15s ease;
}
.hook-entry:hover {
  border-color: var(--accent-soft);
}
.matcher-row, .type-row, .config-row {
  display: flex;
  align-items: center;
  gap: 10px;
  margin: 6px 0;
}
.matcher-row label, .type-row label, .config-row label {
  min-width: 72px;
  color: var(--text-muted);
  font-size: 12px;
  font-weight: 500;
  letter-spacing: 0.02em;
}
.matcher-input, .type-select, .config-input {
  flex: 1;
  padding: 8px 12px;
  background: var(--bg);
  color: var(--text);
  border: 1px solid var(--border-soft);
  border-radius: 4px;
  font-size: 13px;
  transition: all 0.15s ease;
}
.config-textarea {
  flex: 1;
  padding: 10px 12px;
  background: var(--bg);
  color: var(--text);
  border: 1px solid var(--border-soft);
  border-radius: 4px;
  font-size: 13px;
  min-height: 72px;
  resize: vertical;
  line-height: 1.5;
  transition: all 0.15s ease;
}
.matcher-input:focus, .type-select:focus, .config-input:focus, .config-textarea:focus {
  outline: none;
  border-color: var(--accent-soft);
  background: var(--bg-deep);
}
.matcher-input::placeholder, .config-input::placeholder, .config-textarea::placeholder {
  color: var(--text-muted);
  opacity: 0.6;
}
.hooks-list {
  margin: 10px 0;
}
.hook-item {
  margin: 8px 0;
  padding: 12px;
  background: var(--bg);
  border: 1px solid var(--border-soft);
  border-radius: 4px;
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
  padding: 6px 14px;
  background: var(--card);
  color: var(--accent);
  border: 1px solid var(--card-border);
  border-radius: 4px;
  cursor: pointer;
  font-size: 12px;
  font-weight: 500;
  transition: all 0.15s ease;
}
.add-hook:hover {
  background: var(--panel-hover);
  border-color: var(--accent-soft);
}
.add-hook-item {
  padding: 5px 12px;
  background: transparent;
  color: var(--accent);
  border: 1px solid var(--accent-soft);
  border-radius: 3px;
  cursor: pointer;
  font-size: 12px;
  transition: all 0.15s ease;
}
.add-hook-item:hover {
  background: var(--accent-soft);
  color: #e8f0f8;
}
.remove-hook, .remove-hook-item {
  padding: 4px 10px;
  background: transparent;
  color: var(--danger);
  border: 1px solid rgba(138, 90, 90, 0.3);
  border-radius: 3px;
  cursor: pointer;
  font-size: 12px;
  transition: all 0.15s ease;
}
.remove-hook:hover, .remove-hook-item:hover {
  background: var(--danger);
  color: #f0e8e8;
  border-color: var(--danger);
}
.save-btn {
  display: block;
  width: 100%;
  padding: 12px;
  background: var(--accent-soft);
  color: #e8f0f8;
  border: none;
  border-radius: 5px;
  cursor: pointer;
  font-size: 14px;
  font-weight: 500;
  letter-spacing: 0.03em;
  transition: all 0.15s ease;
}
.save-btn:hover {
  background: var(--accent);
}
.global-save {
  margin: 24px 0;
  padding: 16px;
  background: var(--panel);
  border: 1px solid var(--border-soft);
  border-radius: 8px;
}
/* 滚动条 */
::-webkit-scrollbar {
  width: 6px;
}
::-webkit-scrollbar-track {
  background: var(--bg);
}
::-webkit-scrollbar-thumb {
  background: var(--border-soft);
  border-radius: 3px;
}
::-webkit-scrollbar-thumb:hover {
  background: var(--border);
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

  const hooksArray = currentHooksData[type] || [];
  list.innerHTML = '';
  hooksArray.forEach((entry, idx) => {
    addHookEntry(list, entry, idx, type);
  });

  modal.style.display = 'flex';
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

// 添加 hook entry
function addHookEntry(container, entry, idx, hookType) {
  const div = document.createElement('div');
  div.className = 'hook-entry';
  div.dataset.index = idx;

  const matcher = entry.matcher || '';
  const hooks = entry.hooks || [];

  // 检查是否需要 matcher (SessionStart 等)
  const needsMatcher = !['SessionStart', 'SessionEnd', 'Setup', 'SubagentStart', 'Notification', 'StopFailure', 'TeammateIdle', 'TaskCreated', 'TaskCompleted', 'PreCompact', 'PostCompact', 'Elicitation', 'ElicitationResult'].includes(hookType);

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
  document.getElementById('edit-modal').style.display = 'none';
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
      entries.push({ matcher, hooks });
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
      alert('保存成功！');
    }
  } catch (err) {
    alert('保存失败: ' + err.message);
  }
});
"#;