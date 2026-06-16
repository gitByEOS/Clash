use crate::lark::types::{LarkChat, LarkMessage, EVENT_RECONNECT_SECS, MANAGER_CHAT_NAME};
use serde_json::{json, Value};
use std::env;
use std::io::{BufRead, BufReader, Write};
use std::process::{ChildStderr, Command, Stdio};
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

struct LarkIdentity {
    open_id: String,
    app_id: String,
}

pub fn ensure_manager_chat() -> Result<LarkChat, String> {
    if let Some(chat) = find_chat_by_name(MANAGER_CHAT_NAME)? {
        return Ok(chat);
    }
    println!("正在创建管理群：{MANAGER_CHAT_NAME}");
    let chat = create_lark_chat(MANAGER_CHAT_NAME)?;
    println!("管理群已就绪：{}", chat.name);
    Ok(chat)
}

pub fn discover_session_chats(prefix: &str) -> Result<Vec<LarkChat>, String> {
    Ok(discover_chats(prefix)?
        .into_iter()
        .filter(|chat| chat.name != MANAGER_CHAT_NAME)
        .collect())
}

pub fn find_chat_by_name(name: &str) -> Result<Option<LarkChat>, String> {
    Ok(discover_chats(name)?
        .into_iter()
        .find(|chat| chat.name == name))
}

pub fn create_lark_chat(name: &str) -> Result<LarkChat, String> {
    let identity = read_lark_identity()?;
    let created = run_lark_cli_json(&[
        "im",
        "+chat-create",
        "--name",
        name,
        "--users",
        &identity.open_id,
        "--bots",
        &identity.app_id,
        "--as",
        "user",
        "--json",
    ])?;
    let chat_id = created
        .pointer("/data/chat_id")
        .and_then(Value::as_str)
        .ok_or_else(|| "建群成功但未返回 chat_id".to_string())?
        .to_string();

    let data = json!({ "id_list": [identity.app_id] }).to_string();
    let params = json!({
        "chat_id": &chat_id,
        "member_id_type": "app_id",
        "succeed_type": 1
    })
    .to_string();
    let _ = run_lark_cli_json(&[
        "im",
        "chat.members",
        "create",
        "--params",
        &params,
        "--data",
        &data,
        "--as",
        "user",
        "--json",
    ]);

    Ok(LarkChat {
        chat_id,
        name: name.to_string(),
    })
}

pub fn spawn_lark_event_consumer(tx: Sender<LarkMessage>) {
    thread::spawn(move || {
        let mut is_start_error_reported = false;
        loop {
            let mut child = match Command::new("lark-cli")
                .args(["event", "consume", "im.message.receive_v1", "--as", "bot"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
            {
                Ok(child) => {
                    is_start_error_reported = false;
                    child
                }
                Err(err) => {
                    if !is_start_error_reported {
                        eprintln!("无法启动飞书监听");
                        eprintln!("原因：{err}");
                        eprintln!("处理：请先安装并完成 lark-cli auth login");
                        is_start_error_reported = true;
                    }
                    thread::sleep(Duration::from_secs(EVENT_RECONNECT_SECS));
                    continue;
                }
            };

            if let Some(stdin) = child.stdin.as_mut() {
                let _ = stdin.write_all(b"");
            }
            if let Some(stderr) = child.stderr.take() {
                drain_stderr("lark-cli[event]".to_string(), stderr);
            }

            if let Some(stdout) = child.stdout.take() {
                let reader = BufReader::new(stdout);
                for line in reader.lines().map_while(Result::ok) {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    match serde_json::from_str::<Value>(trimmed) {
                        Ok(value) => {
                            if let Some(message) = parse_lark_message(&value) {
                                if tx.send(message).is_err() {
                                    return;
                                }
                            }
                        }
                        Err(err) => log_verbose(&format!("飞书事件 JSON 解析失败：{err}")),
                    }
                }
            }

            let _ = child.wait();
            eprintln!("飞书事件连接断开，{} 秒后重连", EVENT_RECONNECT_SECS);
            thread::sleep(Duration::from_secs(EVENT_RECONNECT_SECS));
        }
    });
}

pub fn send_lark_reply(message_id: &str, text: &str) -> Result<(), String> {
    let content = json!({ "text": text }).to_string();
    run_lark_cli_json(&[
        "im",
        "+messages-reply",
        "--message-id",
        message_id,
        "--msg-type",
        "text",
        "--content",
        &content,
        "--as",
        "bot",
        "--json",
    ])?;
    Ok(())
}

pub fn send_lark_text(chat_id: &str, text: &str) -> Result<(), String> {
    let content = json!({ "text": text }).to_string();
    run_lark_cli_json(&[
        "im",
        "+messages-send",
        "--chat-id",
        chat_id,
        "--msg-type",
        "text",
        "--content",
        &content,
        "--as",
        "bot",
        "--json",
    ])?;
    Ok(())
}

pub fn run_lark_cli_json(args: &[&str]) -> Result<Value, String> {
    let output = Command::new("lark-cli")
        .args(args)
        .output()
        .map_err(|e| format!("无法执行 lark-cli: {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    if !output.status.success() {
        log_verbose(&format!("lark-cli 执行失败：{}", args.join(" ")));
        return Err(if stderr.is_empty() {
            format!("lark-cli 执行失败: {}", args.join(" "))
        } else {
            stderr
        });
    }
    if stdout.is_empty() {
        return Ok(json!({ "ok": true }));
    }

    let value: Value = serde_json::from_str(&stdout).map_err(|e| {
        format!(
            "lark-cli 返回非 JSON: {e}: {}",
            stdout.chars().take(200).collect::<String>()
        )
    })?;
    if value.get("ok") == Some(&Value::Bool(false)) {
        return Err(value
            .pointer("/error/message")
            .or_else(|| value.pointer("/error/hint"))
            .and_then(Value::as_str)
            .unwrap_or("lark-cli ok=false")
            .to_string());
    }
    Ok(value)
}

pub fn assert_lark_api_ok(value: &Value, label: &str) -> Result<(), String> {
    if value.get("ok") == Some(&Value::Bool(false)) {
        return Err(value
            .pointer("/error/message")
            .or_else(|| value.pointer("/error/hint"))
            .and_then(Value::as_str)
            .unwrap_or(label)
            .to_string());
    }
    if let Some(code) = value.get("code").and_then(Value::as_i64) {
        if code != 0 {
            let msg = value.get("msg").and_then(Value::as_str).unwrap_or(label);
            return Err(format!("{label} code={code}: {msg}"));
        }
    }
    Ok(())
}

pub fn drain_stderr(label: String, stderr: ChildStderr) {
    thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines().map_while(Result::ok) {
            log_verbose(&format!("{label}: {line}"));
        }
    });
}

pub fn log_verbose(message: &str) {
    if is_lark_verbose() {
        eprintln!("clash lark: {message}");
    }
}

fn is_lark_verbose() -> bool {
    env::var("CLASH_LARK_VERBOSE")
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "on"))
        .unwrap_or(false)
}

fn discover_chats(prefix: &str) -> Result<Vec<LarkChat>, String> {
    let value = run_lark_cli_json(&["im", "+chat-list", "--as", "bot", "--json"])?;
    let chats = value
        .pointer("/data/chats")
        .or_else(|| value.pointer("/data/items"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    Ok(chats
        .iter()
        .filter_map(parse_chat)
        .filter(|chat| chat.name.starts_with(prefix))
        .collect())
}

fn read_lark_identity() -> Result<LarkIdentity, String> {
    let value = run_lark_cli_json(&["auth", "status", "--json"])?;
    let open_id = value
        .pointer("/identities/user/openId")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| "user 身份未就绪：先 lark-cli auth login".to_string())?
        .to_string();
    let app_id = value
        .get("appId")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| "bot appId 缺失：检查 lark-cli config".to_string())?
        .to_string();

    Ok(LarkIdentity { open_id, app_id })
}

fn parse_chat(value: &Value) -> Option<LarkChat> {
    let chat_id = value
        .get("chat_id")
        .or_else(|| value.get("chatId"))
        .and_then(Value::as_str)?
        .to_string();
    let name = value
        .get("name")
        .or_else(|| value.get("chat_name"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    Some(LarkChat { chat_id, name })
}

fn parse_lark_message(value: &Value) -> Option<LarkMessage> {
    let sender_type = value
        .get("sender_type")
        .and_then(Value::as_str)
        .unwrap_or("");
    let sender_id = value.get("sender_id").and_then(Value::as_str).unwrap_or("");
    if sender_type == "app" || sender_id.starts_with("cli_") {
        return None;
    }

    let chat_id = value.get("chat_id").and_then(Value::as_str)?.to_string();
    let message_id = value
        .get("message_id")
        .or_else(|| value.get("id"))
        .and_then(Value::as_str)?
        .to_string();
    let message_type = value
        .get("message_type")
        .and_then(Value::as_str)
        .unwrap_or("text");
    let raw_content = value.get("content").and_then(Value::as_str).unwrap_or("");
    let text = parse_message_text(raw_content, message_type);
    if text.trim().is_empty() {
        return None;
    }

    Some(LarkMessage {
        message_id,
        chat_id,
        text,
    })
}

fn parse_message_text(raw_content: &str, message_type: &str) -> String {
    if message_type == "text" {
        if let Ok(value) = serde_json::from_str::<Value>(raw_content) {
            return value
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or(raw_content)
                .to_string();
        }
    }
    raw_content.to_string()
}
