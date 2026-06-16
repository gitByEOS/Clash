use crate::config;
use crate::crypto;
use crate::lark::card::CardStream;
use crate::lark::claude_session::ClaudeSession;
use crate::lark::command::{
    normalize_session_chat_name, parse_create_session_command, parse_lark_args,
};
use crate::lark::lark_cli::{
    create_lark_chat, discover_session_chats, ensure_manager_chat, find_chat_by_name, log_verbose,
    send_lark_reply, send_lark_text, spawn_lark_event_consumer,
};
use crate::lark::types::{
    AgentConfig, ChatWorker, LarkChat, LarkMessage, LarkOptions, DEFAULT_CARD_UPDATE_THROTTLE_MS,
};
use crate::model::remove_size_marker;
use std::collections::HashMap;
use std::env;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

pub fn do_lark(args: &[String]) -> Result<(), ()> {
    let options = parse_lark_args(args).map_err(|msg| {
        log_error_action(
            "参数错误",
            &msg,
            "用法：clash lark [--prefix Clash-] [--poll-secs 15] [--once]",
        );
    })?;

    let agent = load_default_agent().map_err(|msg| {
        log_error_action(
            "无法读取 Clash 配置",
            &msg,
            "请先运行 clash config 完成配置",
        );
    })?;

    run_lark_daemon(options, agent).map_err(|msg| {
        log_error_action(
            "无法启动飞书监听",
            &msg,
            "请先安装并完成 lark-cli auth login",
        );
    })
}

fn load_default_agent() -> Result<AgentConfig, String> {
    let slots = config::read_config_slots().map_err(|e| e.to_string())?;
    let slot = slots
        .first()
        .ok_or_else(|| "未找到 Clash 配置，请先运行 clash config".to_string())?;
    let model = slot
        .config
        .models
        .first()
        .map(|m| remove_size_marker(m))
        .filter(|m| !m.is_empty())
        .ok_or_else(|| "默认账户没有可用模型".to_string())?;
    let auth_token = crypto::decrypt_token(&slot.config.auth_token_encrypted)
        .map_err(|_| "无法解密 API Key".to_string())?;
    let system_prompt_file = config::read_system_prompt()
        .map(|_| config::system_prompt_path().to_string_lossy().to_string());

    Ok(AgentConfig {
        base_url: slot.config.base_url.clone(),
        auth_token,
        model,
        system_prompt_file,
    })
}

fn run_lark_daemon(options: LarkOptions, agent: AgentConfig) -> Result<(), String> {
    let chats = discover_session_chats(&options.prefix)?;
    if options.once {
        for chat in chats {
            println!("{} {}", chat.chat_id, chat.name);
        }
        return Ok(());
    }

    let manager = ensure_manager_chat()?;
    log_ready(&manager, &options, &agent, chats.len());

    let (event_tx, event_rx) = mpsc::channel::<LarkMessage>();
    spawn_lark_event_consumer(event_tx);

    let mut known_chats = chat_map(chats);
    let mut workers: HashMap<String, ChatWorker> = HashMap::new();

    let mut next_poll = Instant::now() + Duration::from_secs(options.poll_secs);
    loop {
        match event_rx.recv_timeout(Duration::from_millis(500)) {
            Ok(message) => {
                if message.chat_id == manager.chat_id {
                    handle_manager_message(&message, &options, &mut known_chats);
                } else if let Some(chat) = known_chats.get(&message.chat_id).cloned() {
                    dispatch_chat_message(&mut workers, chat, &agent, message)?;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return Err("飞书事件通道已断开".to_string());
            }
        }

        if Instant::now() >= next_poll {
            for chat in discover_session_chats(&options.prefix)? {
                known_chats.insert(chat.chat_id.clone(), chat);
            }
            next_poll = Instant::now() + Duration::from_secs(options.poll_secs);
        }
    }
}

fn handle_manager_message(
    message: &LarkMessage,
    options: &LarkOptions,
    known_chats: &mut HashMap<String, LarkChat>,
) {
    let Some(raw_name) = parse_create_session_command(&message.text) else {
        return;
    };
    log_command_received(&raw_name);
    let chat_name = normalize_session_chat_name(&raw_name, &options.prefix);
    if let Err(err) = create_or_register_session_chat(&chat_name, known_chats) {
        let _ = send_lark_reply(&message.message_id, &format!("创建会话失败: {err}"));
    }
}

fn create_or_register_session_chat(
    chat_name: &str,
    known_chats: &mut HashMap<String, LarkChat>,
) -> Result<(), String> {
    if let Some(chat) = find_chat_by_name(chat_name)? {
        known_chats.insert(chat.chat_id.clone(), chat.clone());
        send_lark_text(&chat.chat_id, "Claude已开启监听")?;
        return Ok(());
    }

    let chat = create_lark_chat(chat_name)?;
    known_chats.insert(chat.chat_id.clone(), chat.clone());
    send_lark_text(&chat.chat_id, "Claude已开启监听")?;
    Ok(())
}

fn ensure_worker(
    workers: &mut HashMap<String, ChatWorker>,
    chat: LarkChat,
    agent: &AgentConfig,
    first_message: Option<LarkMessage>,
) -> Result<(), String> {
    if workers.contains_key(&chat.chat_id) {
        return Ok(());
    }

    log_verbose(&format!("启动群会话 {} ({})", chat.name, chat.chat_id));
    let (tx, rx) = mpsc::channel::<LarkMessage>();
    if let Some(message) = first_message {
        let _ = tx.send(message);
    }
    let worker_chat = chat.clone();
    let worker_agent = agent.clone();
    thread::spawn(move || run_chat_worker(worker_chat, worker_agent, rx));
    workers.insert(chat.chat_id, ChatWorker { tx });
    Ok(())
}

fn dispatch_chat_message(
    workers: &mut HashMap<String, ChatWorker>,
    chat: LarkChat,
    agent: &AgentConfig,
    message: LarkMessage,
) -> Result<(), String> {
    let chat_id = chat.chat_id.clone();
    if let Some(tx) = workers.get(&chat_id).map(|worker| worker.tx.clone()) {
        match tx.send(message) {
            Ok(()) => return Ok(()),
            Err(err) => {
                log_recovering(&format!("会话进程已结束，正在重启：{}", chat.name));
                workers.remove(&chat_id);
                return ensure_worker(workers, chat, agent, Some(err.0));
            }
        }
    }

    ensure_worker(workers, chat, agent, Some(message))
}

fn chat_map(chats: Vec<LarkChat>) -> HashMap<String, LarkChat> {
    chats
        .into_iter()
        .map(|chat| (chat.chat_id.clone(), chat))
        .collect()
}

fn run_chat_worker(chat: LarkChat, agent: AgentConfig, rx: Receiver<LarkMessage>) {
    let mut session = match ClaudeSession::spawn(&chat, &agent) {
        Ok(session) => session,
        Err(err) => {
            log_error_action(
                "无法启动 ClaudeCli",
                &format!("{}：{err}", chat.name),
                "请检查 claude 是否已安装，以及 Clash 配置是否可用",
            );
            return;
        }
    };

    for message in rx {
        log_message_received(&chat, &message);
        let mut card = match CardStream::open(&message.message_id, "收到，开始处理...") {
            Ok(card) => card,
            Err(err) => {
                log_error_action(
                    "无法创建流式卡片",
                    &err,
                    "请检查飞书应用 cardkit 和消息权限后重试",
                );
                let _ = send_lark_reply(&message.message_id, &format!("流式卡片创建失败: {err}"));
                continue;
            }
        };

        let prompt = message.text.clone();
        let card_update_interval = Duration::from_millis(card_update_throttle_ms());
        let mut last_card_update = Instant::now() - card_update_interval;
        let response = session.ask_stream(&prompt, |text| {
            if last_card_update.elapsed() < card_update_interval {
                return Ok(());
            }
            last_card_update = Instant::now();
            card.update(text)
        });

        let reply = match response {
            Ok(text) if !text.trim().is_empty() => text,
            Ok(_) => "Claude 没有返回内容".to_string(),
            Err(err) => {
                let text = format!("处理已中断: {err}");
                let _ = card.close(&text);
                continue;
            }
        };

        if let Err(err) = card.close(&reply) {
            log_error_action(
                "无法关闭流式卡片",
                &err,
                "请检查飞书应用 cardkit 和消息权限后重试",
            );
        }
    }
}

fn card_update_throttle_ms() -> u64 {
    env::var("CLASH_LARK_CARD_UPDATE_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_CARD_UPDATE_THROTTLE_MS)
}

fn log_ready(manager: &LarkChat, options: &LarkOptions, agent: &AgentConfig, chat_count: usize) {
    println!("Lark 已启动");
    println!("管理群：{}", manager.name);
    println!("会话前缀：{}", options.prefix);
    println!("已发现会话：{chat_count} 个");
    println!("使用模型：{}", agent.model);
    println!();
    println!("在管理群发送：新会话/接入 群名称");
    println!("Clash 会自动接入新的 ClaudeCli，发送消息 Claude 会用流式卡片回复");
}

fn log_command_received(name: &str) {
    println!("收到建群指令：{name}");
}

fn log_message_received(chat: &LarkChat, message: &LarkMessage) {
    println!(
        "收到消息：{} / {}",
        chat.name,
        message_summary(&message.text)
    );
}

fn message_summary(text: &str) -> String {
    let summary = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = summary.chars();
    let clipped = chars.by_ref().take(25).collect::<String>();
    if chars.next().is_some() {
        format!("{clipped}...")
    } else {
        clipped
    }
}

fn log_recovering(message: &str) {
    eprintln!("{message}");
}

fn log_error_action(title: &str, reason: &str, action: &str) {
    eprintln!("{title}");
    eprintln!("原因：{reason}");
    eprintln!("处理：{action}");
}

#[cfg(test)]
mod tests {
    use super::message_summary;

    #[test]
    fn message_summary_truncates_after_25_chars() {
        assert_eq!(message_summary("短消息"), "短消息");
        assert_eq!(
            message_summary("12345678901234567890123456"),
            "1234567890123456789012345..."
        );
    }

    #[test]
    fn message_summary_normalizes_whitespace() {
        assert_eq!(message_summary("  hello\nworld  "), "hello world");
    }
}
