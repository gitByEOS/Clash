use crate::claude;
use crate::config;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{self, Child};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

const CAPTURE_MODEL: &str = "claude-opus-4-8";
const MAX_LOCAL_FILES: usize = 120;
const MAX_LOCAL_FILE_BYTES: usize = 256 * 1024;

pub enum PromptOutput {
    Json,
    Html,
    HtmlOpen,
}

pub struct PromptCapture {
    pub method: String,
    pub path: String,
    pub headers: BTreeMap<String, String>,
    pub body_raw: String,
    pub body_json: Option<Value>,
}

struct LocalContext {
    rules: Vec<LocalFile>,
    skills: Vec<LocalFile>,
    deferred_tools: Vec<String>,
}

struct LocalFile {
    title: String,
    path: PathBuf,
    content: String,
    truncated: bool,
}

pub fn parse_prompt_output(args: &[String]) -> Result<PromptOutput, String> {
    if args.is_empty() {
        return Ok(PromptOutput::HtmlOpen);
    }
    if args.len() > 1 {
        return Err("用法: clash prompt [--json|--html]".to_string());
    }

    match args[0].as_str() {
        "--json" => Ok(PromptOutput::Json),
        "--html" => Ok(PromptOutput::Html),
        _ => Err("用法: clash prompt [--json|--html]".to_string()),
    }
}

pub fn capture_claude_prompt(print_red: fn(&str)) -> Result<PromptCapture, ()> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|err| {
        print_red(&format!("无法启动本地捕获服务: {err}"));
    })?;
    let addr = listener.local_addr().map_err(|err| {
        print_red(&format!("无法读取本地捕获端口: {err}"));
    })?;

    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = listener.set_nonblocking(true);
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut requested_deferred_tools = false;
        loop {
            if Instant::now() >= deadline {
                let _ = tx.send(Err("未收到 /v1/messages 请求".to_string()));
                break;
            }
            match listener.accept() {
                Ok((mut stream, _)) => match read_capture_stream(&mut stream) {
                    Ok(capture) if is_messages_request(&capture) => {
                        if !requested_deferred_tools {
                            if let Some(query) = deferred_tool_search_query(&capture) {
                                requested_deferred_tools = true;
                                write_tool_search_response(&mut stream, &query);
                                continue;
                            }
                        }
                        write_error_response(&mut stream);
                        let _ = tx.send(Ok(capture));
                        break;
                    }
                    Ok(_) => {
                        write_probe_response(&mut stream);
                    }
                    Err(err) => {
                        let _ = tx.send(Err(err));
                        break;
                    }
                },
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(20));
                }
                Err(err) => {
                    let _ = tx.send(Err(err.to_string()));
                    break;
                }
            }
        }
    });

    let claude_path = claude::find_claude_binary()?;
    let mut child = run_probe(&claude_path, &format!("http://{addr}")).map_err(|err| {
        print_red(&format!("无法启动 Claude: {err}"));
    })?;

    let result = match rx.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok(capture)) => Ok(capture),
        Ok(Err(err)) => {
            print_red(&format!("解析 Claude 请求失败: {err}"));
            Err(())
        }
        Err(_) => {
            print_red("未捕获到 Claude 请求");
            Err(())
        }
    };
    terminate_probe(&mut child);
    result
}

fn run_probe(claude_path: &str, base_url: &str) -> Result<Child, std::io::Error> {
    let mut cmd = process::Command::new(claude_path);
    cmd.env("ANTHROPIC_BASE_URL", base_url)
        .env("ANTHROPIC_AUTH_TOKEN", "sk-clash-prompt-capture")
        .env("ANTHROPIC_API_KEY", "sk-clash-prompt-capture")
        .env("ANTHROPIC_MODEL", CAPTURE_MODEL)
        .env("ANTHROPIC_SMALL_FAST_MODEL", CAPTURE_MODEL)
        .env("ANTHROPIC_DEFAULT_SONNET_MODEL", CAPTURE_MODEL)
        .env("ANTHROPIC_DEFAULT_OPUS_MODEL", CAPTURE_MODEL)
        .env("ANTHROPIC_DEFAULT_HAIKU_MODEL", CAPTURE_MODEL)
        .env("CLAUDE_CODE_SUBAGENT_MODEL", CAPTURE_MODEL)
        .env("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1")
        .env("CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS", "1")
        .env("CLAUDE_CODE_ATTRIBUTION_HEADER", "0")
        .env("CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS", "1")
        .env("CLAUDE_CODE_ENABLE_AUTO_MODE", "1")
        .arg("-p")
        .arg("ping")
        .arg("--permission-mode")
        .arg("bypassPermissions")
        .arg("--effort")
        .arg("max")
        .arg("--model")
        .arg(CAPTURE_MODEL)
        .stdout(process::Stdio::null())
        .stderr(process::Stdio::null());

    if config::read_system_prompt().is_some() {
        cmd.arg("--append-system-prompt-file")
            .arg(config::system_prompt_path());
    }

    cmd.spawn()
}

fn terminate_probe(child: &mut Child) {
    if matches!(child.try_wait(), Ok(Some(_))) {
        return;
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn read_capture_stream(stream: &mut TcpStream) -> Result<PromptCapture, String> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    let mut header_end = None;
    let mut content_length = 0usize;

    loop {
        let n = stream.read(&mut chunk).map_err(|err| err.to_string())?;
        if n == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..n]);
        if header_end.is_none() {
            header_end = find_header_end(&buffer);
            if let Some(end) = header_end {
                content_length = parse_content_length(&buffer[..end])?;
            }
        }
        if let Some(end) = header_end {
            if buffer.len() >= end + 4 + content_length {
                break;
            }
        }
    }

    parse_request(&buffer)
}

fn is_messages_request(capture: &PromptCapture) -> bool {
    capture.method == "POST"
        && capture.path.contains("/v1/messages")
        && !capture.body_raw.is_empty()
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn parse_content_length(headers: &[u8]) -> Result<usize, String> {
    let text = String::from_utf8_lossy(headers);
    for line in text.lines().skip(1) {
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                return value
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| "Content-Length 不是数字".to_string());
            }
        }
    }
    Ok(0)
}

fn parse_request(buffer: &[u8]) -> Result<PromptCapture, String> {
    let header_end = find_header_end(buffer).ok_or_else(|| "缺少 HTTP 头".to_string())?;
    let header_text = String::from_utf8_lossy(&buffer[..header_end]);
    let mut lines = header_text.lines();
    let request_line = lines.next().ok_or_else(|| "缺少请求行".to_string())?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").to_string();

    let mut headers = BTreeMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_string(), value.trim().to_string());
        }
    }

    let body_start = header_end + 4;
    let body_raw = String::from_utf8_lossy(&buffer[body_start..]).to_string();
    let body_json = serde_json::from_str::<Value>(&body_raw).ok();

    Ok(PromptCapture {
        method,
        path,
        headers,
        body_raw,
        body_json,
    })
}

fn write_error_response(stream: &mut TcpStream) {
    let body = r#"{"type":"error","error":{"type":"authentication_error","message":"clash prompt capture complete"}}"#;
    let response = format!(
        "HTTP/1.1 401 Unauthorized\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

fn write_tool_search_response(stream: &mut TcpStream, query: &str) {
    let body = json!({
        "id": "msg_clash_prompt_capture_toolsearch",
        "type": "message",
        "role": "assistant",
        "model": CAPTURE_MODEL,
        "content": [
            {
                "type": "tool_use",
                "id": "toolu_clash_prompt_capture_toolsearch",
                "name": "ToolSearch",
                "input": {
                    "query": query
                }
            }
        ],
        "stop_reason": "tool_use",
        "stop_sequence": Value::Null,
        "usage": {
            "input_tokens": 1,
            "cache_creation_input_tokens": 0,
            "cache_read_input_tokens": 0,
            "output_tokens": 1
        }
    })
    .to_string();
    let response = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

fn write_probe_response(stream: &mut TcpStream) {
    let response = "HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

pub fn print_capture(capture: &PromptCapture, output: PromptOutput) {
    match output {
        PromptOutput::Json => print_json(capture),
        PromptOutput::Html | PromptOutput::HtmlOpen => print_html(capture),
    }
}

pub fn write_html_report(capture: &PromptCapture) -> Result<PathBuf, String> {
    let path = config::config_dir().join("prompt-report.html");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    fs::write(&path, render_html(capture)).map_err(|err| err.to_string())?;
    Ok(path)
}

pub fn open_html_report(path: &Path) -> Result<(), String> {
    let status = if cfg!(target_os = "macos") {
        process::Command::new("open").arg(path).status()
    } else {
        process::Command::new("xdg-open").arg(path).status()
    }
    .map_err(|err| err.to_string())?;

    if status.success() {
        Ok(())
    } else {
        Err("打开 HTML 报告失败".to_string())
    }
}

fn print_json(capture: &PromptCapture) {
    let local_context = collect_local_context(capture);
    let envelope = json!({
        "method": capture.method,
        "path": capture.path,
        "headers": capture.headers,
        "body": capture.body_json.as_ref().unwrap_or(&Value::Null),
        "body_raw": capture.body_raw,
        "local_context": local_context_json(&local_context),
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&envelope).unwrap_or_else(|_| capture.body_raw.clone())
    );
}

fn print_html(capture: &PromptCapture) {
    print!("{}", render_html(capture));
}

fn render_html(capture: &PromptCapture) -> String {
    let local_context = collect_local_context(capture);
    let mut html = String::new();
    push_line(&mut html, "<!doctype html>");
    push_line(&mut html, "<html lang=\"zh-CN\">");
    push_line(&mut html, "<head>");
    push_line(&mut html, "<meta charset=\"utf-8\">");
    push_line(
        &mut html,
        "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">",
    );
    push_line(&mut html, "<title>Claude 请求捕获</title>");
    push_line(&mut html, "<style>");
    push_line(&mut html, HTML_STYLE);
    push_line(&mut html, "</style>");
    push_line(&mut html, "</head>");
    push_line(&mut html, "<body>");
    push_line(&mut html, "<main>");
    push_line(&mut html, "<h1>Claude 请求捕获</h1>");
    append_html_summary(&mut html, capture);
    append_html_system(&mut html, capture);
    append_html_messages(&mut html, capture);
    append_html_skills(&mut html, capture);
    append_html_tool_group(
        &mut html,
        "MCP Tools",
        "mcp",
        &mcp_tools(capture),
        "本次请求未捕获到 MCP 工具",
    );
    append_html_tool_group(&mut html, "Tools", "tool", &builtin_tools(capture), "无");
    append_html_local_context(&mut html, &local_context);
    append_html_headers(&mut html, capture);
    append_html_raw(&mut html, capture);
    append_html_drawer(&mut html);
    push_line(&mut html, "</main>");
    push_line(&mut html, "<script>");
    push_line(&mut html, HTML_SCRIPT);
    push_line(&mut html, "</script>");
    push_line(&mut html, "</body>");
    push_line(&mut html, "</html>");
    html
}

fn push_line(html: &mut String, line: &str) {
    html.push_str(line);
    html.push('\n');
}

fn append_html_summary(html: &mut String, capture: &PromptCapture) {
    let body = body(capture);
    push_line(html, "<section>");
    push_line(html, "<h2>摘要</h2>");
    push_line(html, "<div class=\"grid\">");
    append_metric(
        html,
        "模型",
        extract_model(capture).unwrap_or(CAPTURE_MODEL),
    );
    append_metric(
        html,
        "请求",
        &format!("{} {}", capture.method, capture.path),
    );
    append_metric(html, "Max Tokens", &json_number(body, "max_tokens"));
    append_metric(html, "Stream", &json_bool(body, "stream"));
    append_metric(html, "Raw Body", &format_bytes(capture.body_raw.len()));
    append_metric(
        html,
        "System 段数",
        &system_text_blocks(capture).len().to_string(),
    );
    append_metric(
        html,
        "Messages 数量",
        &message_summaries(capture).len().to_string(),
    );
    append_metric(html, "Skills 数量", &skills(capture).len().to_string());
    append_metric(
        html,
        "MCP Tools 数量",
        &mcp_tools(capture).len().to_string(),
    );
    append_metric(
        html,
        "Tools 数量",
        &builtin_tools(capture).len().to_string(),
    );
    push_line(html, "</div>");
    push_line(html, "</section>");
}

fn append_metric(html: &mut String, label: &str, value: &str) {
    push_line(
        html,
        &format!(
            "<div class=\"metric\"><span>{}</span><strong>{}</strong></div>",
            html_escape(label),
            html_escape(value)
        ),
    );
}

fn append_html_system(html: &mut String, capture: &PromptCapture) {
    push_line(html, "<section>");
    push_line(html, "<h2>System</h2>");
    let system_blocks = system_text_blocks(capture);
    if system_blocks.is_empty() {
        append_pre(html, &capture.body_raw);
    } else {
        for (idx, text) in system_blocks.iter().enumerate() {
            push_line(html, "<details open>");
            push_line(html, &format!("<summary>System {}</summary>", idx + 1));
            append_pre(html, text.trim());
            push_line(html, "</details>");
        }
    }
    push_line(html, "</section>");
}

fn append_html_messages(html: &mut String, capture: &PromptCapture) {
    push_line(html, "<section>");
    push_line(html, "<h2>Messages</h2>");
    let messages = message_summaries(capture);
    if messages.is_empty() {
        push_line(html, "<p class=\"muted\">无</p>");
    } else {
        for (idx, message) in messages.iter().enumerate() {
            push_line(html, "<details>");
            push_line(
                html,
                &format!(
                    "<summary>Message {} <span class=\"pill\">{}</span></summary>",
                    idx + 1,
                    html_escape(&message.role)
                ),
            );
            append_pre(html, message.text.trim());
            push_line(html, "</details>");
        }
    }
    push_line(html, "</section>");
}

fn append_html_skills(html: &mut String, capture: &PromptCapture) {
    push_line(html, "<section>");
    push_line(html, "<h2>Skills</h2>");
    let skills = skills(capture);
    if skills.is_empty() {
        push_line(html, "<p class=\"muted\">未捕获到 Skills 注入</p>");
    } else {
        push_line(html, "<div class=\"cards\">");
        for (idx, skill) in skills.iter().enumerate() {
            append_card(
                html,
                "skill",
                idx,
                &skill.name,
                &skill.description,
                &skill.description,
            );
        }
        push_line(html, "</div>");
    }
    push_line(html, "</section>");
}

fn append_html_tool_group(
    html: &mut String,
    title: &str,
    kind: &str,
    tools: &[&Value],
    empty: &str,
) {
    push_line(html, "<section>");
    push_line(html, &format!("<h2>{}</h2>", html_escape(title)));
    if tools.is_empty() {
        push_line(
            html,
            &format!("<p class=\"muted\">{}</p>", html_escape(empty)),
        );
    } else {
        push_line(html, "<div class=\"cards\">");
        for (idx, tool) in tools.iter().enumerate() {
            let name = tool
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let description = tool
                .get("description")
                .and_then(Value::as_str)
                .map(first_line)
                .unwrap_or_default();
            append_card(
                html,
                kind,
                idx,
                name,
                &description,
                &serde_json::to_string_pretty(tool).unwrap_or_default(),
            );
        }
        push_line(html, "</div>");
    }
    push_line(html, "</section>");
}

fn append_card(
    html: &mut String,
    kind: &str,
    idx: usize,
    title: &str,
    summary: &str,
    detail: &str,
) {
    let id = format!("{kind}-detail-{idx}");
    push_line(
        html,
        &format!(
            "<button class=\"card\" type=\"button\" data-title=\"{}\" data-detail-id=\"{}\">",
            html_escape(title),
            html_escape(&id)
        ),
    );
    push_line(
        html,
        &format!("<span class=\"card-kind\">{}</span>", html_escape(kind)),
    );
    push_line(html, &format!("<strong>{}</strong>", html_escape(title)));
    push_line(html, &format!("<p>{}</p>", html_escape(summary)));
    push_line(html, "</button>");
    push_line(
        html,
        &format!(
            "<template id=\"{}\"><pre>{}</pre></template>",
            html_escape(&id),
            html_escape(detail)
        ),
    );
}

fn append_html_headers(html: &mut String, capture: &PromptCapture) {
    push_line(html, "<section>");
    push_line(html, "<h2>Headers</h2>");
    append_pre(
        html,
        &serde_json::to_string_pretty(&redacted_headers(capture)).unwrap_or_default(),
    );
    push_line(html, "</section>");
}

fn append_html_raw(html: &mut String, capture: &PromptCapture) {
    push_line(html, "<section>");
    push_line(html, "<h2>Raw</h2>");
    push_line(html, "<details open>");
    push_line(html, "<summary>Body JSON</summary>");
    append_pre(html, &pretty_body(capture));
    push_line(html, "</details>");
    push_line(html, "</section>");
}

fn append_html_drawer(html: &mut String) {
    push_line(html, "<aside class=\"drawer\" aria-hidden=\"true\">");
    push_line(html, "<div class=\"drawer-panel\">");
    push_line(
        html,
        "<button class=\"drawer-close\" type=\"button\" aria-label=\"关闭\">×</button>",
    );
    push_line(html, "<h2 class=\"drawer-title\"></h2>");
    push_line(html, "<div class=\"drawer-body\"></div>");
    push_line(html, "</div>");
    push_line(html, "</aside>");
}

fn append_html_local_context(html: &mut String, local_context: &LocalContext) {
    push_line(html, "<section>");
    push_line(html, "<h2>本地补齐</h2>");
    push_line(
        html,
        "<p class=\"muted\">这些内容来自本机可读文件，不代表已经进入本次 API 请求</p>",
    );
    append_html_deferred_tools(html, &local_context.deferred_tools);
    append_html_local_files(html, "规则文件", "rule", &local_context.rules);
    append_html_local_files(html, "Skill 文件", "skill-file", &local_context.skills);
    push_line(html, "</section>");
}

fn append_html_deferred_tools(html: &mut String, tools: &[String]) {
    push_line(html, "<details>");
    push_line(
        html,
        &format!(
            "<summary>延迟工具 Schema <span class=\"pill\">{}</span></summary>",
            tools.len()
        ),
    );
    if tools.is_empty() {
        push_line(html, "<p class=\"muted\">未发现延迟工具提示</p>");
    } else {
        append_pre(html, &tools.join("\n"));
    }
    push_line(html, "</details>");
}

fn append_html_local_files(html: &mut String, title: &str, kind: &str, files: &[LocalFile]) {
    push_line(html, "<details open>");
    push_line(
        html,
        &format!(
            "<summary>{} <span class=\"pill\">{}</span></summary>",
            html_escape(title),
            files.len()
        ),
    );
    if files.is_empty() {
        push_line(html, "<p class=\"muted\">无</p>");
    } else {
        push_line(html, "<div class=\"cards\">");
        for (idx, file) in files.iter().enumerate() {
            let title = format!(
                "{}{}",
                file.title,
                if file.truncated { " (已截断)" } else { "" }
            );
            let detail = format!("{}\n\n{}", file.path.display(), file.content);
            append_card(
                html,
                kind,
                idx,
                &title,
                &file.path.display().to_string(),
                &detail,
            );
        }
        push_line(html, "</div>");
    }
    push_line(html, "</details>");
}

fn append_pre(html: &mut String, text: &str) {
    push_line(html, &format!("<pre>{}</pre>", html_escape(text)));
}

struct MessageSummary {
    role: String,
    text: String,
}

struct SkillInfo {
    name: String,
    description: String,
}

fn body(capture: &PromptCapture) -> Option<&Value> {
    capture.body_json.as_ref()
}

fn system_text_blocks(capture: &PromptCapture) -> Vec<String> {
    body(capture)
        .and_then(|body| body.get("system"))
        .and_then(text_blocks_from_value)
        .unwrap_or_default()
}

fn text_blocks_from_value(value: &Value) -> Option<Vec<String>> {
    let blocks = match value {
        Value::String(text) => vec![text.clone()],
        Value::Array(items) => items
            .iter()
            .filter_map(|item| {
                item.get("text")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or_else(|| item.as_str().map(str::to_string))
            })
            .collect(),
        value => vec![serde_json::to_string_pretty(value).ok()?],
    };
    Some(blocks)
}

fn message_summaries(capture: &PromptCapture) -> Vec<MessageSummary> {
    body(capture)
        .and_then(|body| body.get("messages"))
        .and_then(Value::as_array)
        .map(|messages| {
            messages
                .iter()
                .map(|message| MessageSummary {
                    role: message
                        .get("role")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown")
                        .to_string(),
                    text: message_text(message),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn message_text(message: &Value) -> String {
    let Some(content) = message.get("content") else {
        return String::new();
    };
    match content {
        Value::String(text) => text.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(|item| item.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n\n"),
        value => serde_json::to_string_pretty(value).unwrap_or_default(),
    }
}

fn extract_model(capture: &PromptCapture) -> Option<&str> {
    capture.body_json.as_ref()?.get("model")?.as_str()
}

fn tools(capture: &PromptCapture) -> Vec<&Value> {
    body(capture)
        .and_then(|body| body.get("tools"))
        .and_then(Value::as_array)
        .map(|items| items.iter().collect())
        .unwrap_or_default()
}

fn builtin_tools(capture: &PromptCapture) -> Vec<&Value> {
    tools(capture)
        .into_iter()
        .filter(|tool| !is_mcp_tool(tool))
        .collect()
}

fn mcp_tools(capture: &PromptCapture) -> Vec<&Value> {
    tools(capture)
        .into_iter()
        .filter(|tool| is_mcp_tool(tool))
        .collect()
}

fn deferred_tool_search_query(capture: &PromptCapture) -> Option<String> {
    if !has_tool(capture, "ToolSearch") {
        return None;
    }
    let names = deferred_tool_names(capture);
    if names.is_empty() {
        None
    } else {
        Some(format!("select:{}", names.join(",")))
    }
}

fn has_tool(capture: &PromptCapture, expected: &str) -> bool {
    tools(capture).into_iter().any(|tool| {
        tool.get("name")
            .and_then(Value::as_str)
            .is_some_and(|name| name == expected)
    })
}

fn is_mcp_tool(tool: &Value) -> bool {
    let name = tool.get("name").and_then(Value::as_str).unwrap_or("");
    name.starts_with("mcp__")
        || name.starts_with("MCP")
        || name.contains("Mcp")
        || name.contains("__mcp")
}

fn redacted_headers(capture: &PromptCapture) -> BTreeMap<String, String> {
    capture
        .headers
        .iter()
        .map(|(key, value)| {
            let redacted = if is_secret_header(key) {
                mask_secret(value)
            } else {
                value.clone()
            };
            (key.clone(), redacted)
        })
        .collect()
}

fn is_secret_header(key: &str) -> bool {
    key.eq_ignore_ascii_case("authorization") || key.eq_ignore_ascii_case("x-api-key")
}

fn mask_secret(value: &str) -> String {
    if value.len() <= 8 {
        return "***".to_string();
    }
    format!("{}***{}", &value[..4], &value[value.len() - 4..])
}

fn pretty_body(capture: &PromptCapture) -> String {
    body(capture)
        .and_then(|value| serde_json::to_string_pretty(value).ok())
        .unwrap_or_else(|| capture.body_raw.clone())
}

fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn json_number<'a>(body: Option<&'a Value>, key: &str) -> String {
    body.and_then(|body| body.get(key))
        .and_then(Value::as_i64)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "null".to_string())
}

fn json_bool<'a>(body: Option<&'a Value>, key: &str) -> String {
    body.and_then(|body| body.get(key))
        .and_then(Value::as_bool)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "null".to_string())
}

fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or("").trim().to_string()
}

fn format_bytes(bytes: usize) -> String {
    if bytes < 1000 {
        return format!("{bytes}B");
    }
    format!("{:.2}k", bytes as f64 / 1000.0)
}

fn skills(capture: &PromptCapture) -> Vec<SkillInfo> {
    message_summaries(capture)
        .into_iter()
        .filter(|message| {
            message.role == "system"
                && message
                    .text
                    .contains("The following skills are available for use with the Skill tool")
        })
        .flat_map(|message| parse_skills(&message.text))
        .collect()
}

fn parse_skills(text: &str) -> Vec<SkillInfo> {
    let mut skills = Vec::new();
    let mut current: Option<SkillInfo> = None;

    for line in text.lines() {
        if let Some(raw) = line.strip_prefix("- ") {
            if let Some(skill) = current.take() {
                skills.push(skill);
            }
            let (name, description) = raw
                .split_once(':')
                .map(|(name, description)| (name.trim(), description.trim()))
                .unwrap_or((raw.trim(), ""));
            current = Some(SkillInfo {
                name: name.to_string(),
                description: description.to_string(),
            });
            continue;
        }

        if let Some(skill) = current.as_mut() {
            let extra = line.trim();
            if !extra.is_empty() {
                if !skill.description.is_empty() {
                    skill.description.push(' ');
                }
                skill.description.push_str(extra);
            }
        }
    }

    if let Some(skill) = current {
        skills.push(skill);
    }
    skills
}

fn collect_local_context(capture: &PromptCapture) -> LocalContext {
    LocalContext {
        rules: collect_rule_files(),
        skills: collect_skill_files(),
        deferred_tools: deferred_tool_names(capture),
    }
}

fn collect_rule_files() -> Vec<LocalFile> {
    let mut files = Vec::new();
    if let Some(home) = home_dir() {
        read_claude_context_files(&mut files, &home.join(".claude"));
        read_tree_by_extensions(
            &mut files,
            &home.join(".claude/rules"),
            &["md", "mdc", "txt"],
        );
    }
    if let Ok(current) = env::current_dir() {
        for dir in current.ancestors() {
            read_named_file(&mut files, &dir.join("CLAUDE.md"));
            read_claude_context_files(&mut files, &dir.join(".claude"));
            read_tree_by_extensions(
                &mut files,
                &dir.join(".claude/rules"),
                &["md", "mdc", "txt"],
            );
        }
    }
    dedupe_files(files)
}

fn collect_skill_files() -> Vec<LocalFile> {
    let mut files = Vec::new();
    if let Some(home) = home_dir() {
        read_tree_named_files(&mut files, &home.join(".claude/skills"), "SKILL.md");
        read_tree_named_files(&mut files, &home.join(".cursor/skills"), "SKILL.md");
        read_tree_named_files(&mut files, &home.join(".cursor/skills-cursor"), "SKILL.md");
    }
    if let Ok(current) = env::current_dir() {
        read_tree_named_files(&mut files, &current.join(".claude/skills"), "SKILL.md");
        read_tree_named_files(&mut files, &current.join(".cursor/skills"), "SKILL.md");
    }
    dedupe_files(files)
}

fn read_claude_context_files(files: &mut Vec<LocalFile>, dir: &Path) {
    read_named_file(files, &dir.join("CLAUDE.md"));
    read_named_file(files, &dir.join("settings.json"));
    read_named_file(files, &dir.join("settings.local.json"));
}

fn read_tree_by_extensions(files: &mut Vec<LocalFile>, root: &Path, extensions: &[&str]) {
    if files.len() >= MAX_LOCAL_FILES || !root.is_dir() {
        return;
    }
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        if files.len() >= MAX_LOCAL_FILES {
            break;
        }
        let path = entry.path();
        if path.is_dir() {
            read_tree_by_extensions(files, &path, extensions);
        } else if path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|ext| extensions.contains(&ext))
        {
            read_named_file(files, &path);
        }
    }
}

fn read_tree_named_files(files: &mut Vec<LocalFile>, root: &Path, file_name: &str) {
    if files.len() >= MAX_LOCAL_FILES || !root.is_dir() {
        return;
    }
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        if files.len() >= MAX_LOCAL_FILES {
            break;
        }
        let path = entry.path();
        if path.is_dir() {
            read_tree_named_files(files, &path, file_name);
        } else if path.file_name().and_then(|value| value.to_str()) == Some(file_name) {
            read_named_file(files, &path);
        }
    }
}

fn read_named_file(files: &mut Vec<LocalFile>, path: &Path) {
    if files.len() >= MAX_LOCAL_FILES || !path.is_file() {
        return;
    }
    let Ok(bytes) = fs::read(path) else {
        return;
    };
    let truncated = bytes.len() > MAX_LOCAL_FILE_BYTES;
    let content =
        String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_LOCAL_FILE_BYTES)]).to_string();
    let title = path
        .parent()
        .and_then(Path::file_name)
        .and_then(|value| value.to_str())
        .or_else(|| path.file_name().and_then(|value| value.to_str()))
        .unwrap_or("context")
        .to_string();
    files.push(LocalFile {
        title,
        path: path.to_path_buf(),
        content: redact_local_content(&content),
        truncated,
    });
}

fn dedupe_files(files: Vec<LocalFile>) -> Vec<LocalFile> {
    let mut seen = BTreeMap::new();
    for file in files {
        seen.entry(file.path.clone()).or_insert(file);
    }
    seen.into_values().collect()
}

fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME").map(PathBuf::from)
}

fn redact_local_content(content: &str) -> String {
    content
        .lines()
        .map(|line| {
            let lower = line.to_ascii_lowercase();
            if lower.contains("api_key")
                || lower.contains("auth_token")
                || lower.contains("authorization")
                || lower.contains("secret")
                || lower.contains("token")
                || lower.contains("bearer")
                || lower.contains("password")
            {
                mask_secret_line(line)
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn mask_secret_line(line: &str) -> String {
    line.split_once(['=', ':'])
        .map(|(key, separator)| format!("{key}{separator} ***"))
        .unwrap_or_else(|| "***".to_string())
}

fn deferred_tool_names(capture: &PromptCapture) -> Vec<String> {
    message_summaries(capture)
        .into_iter()
        .flat_map(|message| parse_deferred_tools(&message.text))
        .collect()
}

fn parse_deferred_tools(text: &str) -> Vec<String> {
    text.lines()
        .skip_while(|line| !line.contains("deferred tools"))
        .skip(1)
        .take_while(|line| !line.trim().starts_with("</system-reminder>"))
        .filter_map(|line| {
            let name = line.trim();
            if name.is_empty() || name.contains(' ') || name.contains(':') {
                None
            } else {
                Some(name.to_string())
            }
        })
        .collect()
}

fn local_context_json(local_context: &LocalContext) -> Value {
    json!({
        "note": "local_context 来自本机文件补齐，不代表已经进入本次 API 请求",
        "deferred_tools": local_context.deferred_tools,
        "rules": local_files_json(&local_context.rules),
        "skills": local_files_json(&local_context.skills),
    })
}

fn local_files_json(files: &[LocalFile]) -> Value {
    Value::Array(
        files
            .iter()
            .map(|file| {
                json!({
                    "title": file.title,
                    "path": file.path,
                    "content": file.content,
                    "truncated": file.truncated,
                })
            })
            .collect(),
    )
}

const HTML_STYLE: &str = r#"
:root {
  color-scheme: light dark;
  --bg: #0f172a;
  --panel: #111827;
  --panel-soft: #1f2937;
  --text: #e5e7eb;
  --muted: #9ca3af;
  --border: #374151;
  --accent: #60a5fa;
}
body {
  margin: 0;
  background: var(--bg);
  color: var(--text);
  font: 14px/1.6 -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
}
main {
  max-width: 1180px;
  margin: 0 auto;
  padding: 32px 20px 80px;
}
h1, h2, h3 {
  line-height: 1.25;
}
h1 {
  margin: 0 0 24px;
  font-size: 32px;
}
section {
  margin: 18px 0;
  padding: 20px;
  background: var(--panel);
  border: 1px solid var(--border);
  border-radius: 14px;
}
.grid {
  display: grid;
  grid-template-columns: repeat(5, minmax(0, 1fr));
  gap: 12px;
  align-items: stretch;
}
.metric {
  padding: 12px;
  background: var(--panel-soft);
  border: 1px solid var(--border);
  border-radius: 10px;
  min-height: 72px;
  box-sizing: border-box;
}
.metric span {
  display: block;
  color: var(--muted);
  font-size: 12px;
}
.metric strong {
  display: block;
  margin-top: 4px;
  color: var(--accent);
  overflow-wrap: anywhere;
}
details {
  margin: 12px 0;
  padding: 12px;
  background: var(--panel-soft);
  border: 1px solid var(--border);
  border-radius: 10px;
}
summary {
  cursor: pointer;
  font-weight: 700;
}
pre {
  overflow: auto;
  max-height: 720px;
  padding: 14px;
  background: #020617;
  border: 1px solid var(--border);
  border-radius: 10px;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}
.cards {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: 12px;
  align-items: start;
}
.card {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  width: 100%;
  height: 360px;
  padding: 14px;
  background: var(--panel-soft);
  color: var(--text);
  border: 1px solid var(--border);
  border-radius: 10px;
  text-align: left;
  cursor: pointer;
}
.card:hover {
  border-color: var(--accent);
}
.card-kind {
  margin-bottom: 10px;
  padding: 2px 8px;
  color: #bfdbfe;
  background: #1e3a8a;
  border-radius: 999px;
  font-size: 12px;
}
.card strong {
  display: block;
  margin-bottom: 10px;
  color: var(--accent);
  font-size: 16px;
}
.card p {
  margin: 0;
  color: var(--muted);
  display: -webkit-box;
  -webkit-line-clamp: 14;
  -webkit-box-orient: vertical;
  overflow: hidden;
}
.pill {
  margin-left: 8px;
  padding: 2px 8px;
  color: #bfdbfe;
  background: #1e3a8a;
  border-radius: 999px;
  font-size: 12px;
}
.muted {
  color: var(--muted);
}
@media (max-width: 900px) {
  .grid {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
  .cards {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}
@media (max-width: 560px) {
  .grid,
  .cards {
    grid-template-columns: 1fr;
  }
}
.drawer {
  position: fixed;
  inset: 0;
  z-index: 50;
  pointer-events: none;
  background: rgba(2, 6, 23, 0);
  transition: background 180ms ease;
}
.drawer.open {
  pointer-events: auto;
  background: rgba(2, 6, 23, 0.58);
}
.drawer-panel {
  position: absolute;
  top: 0;
  right: 0;
  width: min(760px, 92vw);
  height: 100%;
  padding: 24px;
  box-sizing: border-box;
  background: var(--panel);
  border-left: 1px solid var(--border);
  transform: translateX(100%);
  transition: transform 220ms ease;
  overflow: auto;
}
.drawer.open .drawer-panel {
  transform: translateX(0);
}
.drawer-close {
  float: right;
  width: 36px;
  height: 36px;
  color: var(--text);
  background: var(--panel-soft);
  border: 1px solid var(--border);
  border-radius: 999px;
  cursor: pointer;
  font-size: 24px;
  line-height: 1;
}
.drawer-title {
  margin: 0 48px 18px 0;
}
"#;

const HTML_SCRIPT: &str = r#"
const drawer = document.querySelector('.drawer');
const drawerTitle = document.querySelector('.drawer-title');
const drawerBody = document.querySelector('.drawer-body');
const closeButton = document.querySelector('.drawer-close');

function closeDrawer() {
  drawer.classList.remove('open');
  drawer.setAttribute('aria-hidden', 'true');
  drawerBody.innerHTML = '';
}

document.querySelectorAll('.card').forEach((card) => {
  card.addEventListener('click', () => {
    const template = document.getElementById(card.dataset.detailId);
    drawerTitle.textContent = card.dataset.title || '';
    drawerBody.innerHTML = template ? template.innerHTML : '';
    drawer.classList.add('open');
    drawer.setAttribute('aria-hidden', 'false');
  });
});

closeButton.addEventListener('click', closeDrawer);
drawer.addEventListener('click', (event) => {
  if (event.target === drawer) closeDrawer();
});
document.addEventListener('keydown', (event) => {
  if (event.key === 'Escape') closeDrawer();
});
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn capture_from_body(body: Value) -> PromptCapture {
        PromptCapture {
            method: "POST".to_string(),
            path: "/v1/messages".to_string(),
            headers: BTreeMap::new(),
            body_raw: body.to_string(),
            body_json: Some(body),
        }
    }

    #[test]
    fn deferred_tool_search_query_selects_all_deferred_tools() {
        let capture = capture_from_body(json!({
            "messages": [
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "text",
                            "text": "<system-reminder>\nThe following deferred tools are now available via ToolSearch. Their schemas are NOT loaded:\nAskUserQuestion\nTodoWrite\nWebSearch\n</system-reminder>"
                        }
                    ]
                }
            ],
            "tools": [
                {
                    "name": "ToolSearch",
                    "description": "Fetches full schema definitions"
                }
            ]
        }));

        assert_eq!(
            deferred_tool_search_query(&capture),
            Some("select:AskUserQuestion,TodoWrite,WebSearch".to_string())
        );
    }

    #[test]
    fn deferred_tool_search_query_skips_when_toolsearch_is_absent() {
        let capture = capture_from_body(json!({
            "messages": [
                {
                    "role": "user",
                    "content": "ToolSearch\nAskUserQuestion"
                }
            ],
            "tools": []
        }));

        assert_eq!(deferred_tool_search_query(&capture), None);
    }
}
