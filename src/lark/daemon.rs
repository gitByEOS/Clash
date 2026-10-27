use crate::config::{self, RunModelChoice};
use crate::crypto;
use crate::lark::card::CardStream;
use crate::lark::claude_session::ClaudeSession;
use crate::lark::command::{
    normalize_session_chat_name, parse_create_session_command, parse_lark_args,
    parse_session_command, resolve_working_dir, SessionCommand,
};
use crate::lark::lark_cli::{
    create_lark_chat, discover_session_chats, ensure_manager_chat, find_chat_by_name, log_verbose,
    send_lark_reply, send_lark_text, spawn_lark_event_consumer,
};
use crate::lark::render::{
    format_context_percent, format_elapsed, format_session_footer, queue_fold, queue_result,
    queue_stop, result_text, ClaudeRenderState, Fold, OutboundOp, TokenUsage, DEFAULT_CONTEXT_TOKENS,
};
use crate::lark::types::{AgentConfig, ChatWorker, LarkChat, LarkMessage, LarkOptions};
use crate::model::{context_size_marker, remove_size_marker};
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::env;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

pub fn do_lark(args: &[String]) -> Result<(), ()> {
    let options = parse_lark_args(args).map_err(|msg| {
        log_error_action(
            "参数错误",
            &msg,
            "用法：clash lark [-m <模型编号>] [--prefix Clash-] [--poll-secs 15] [--once]",
        );
    })?;

    if let Err(msg) = enter_working_dir() {
        log_error_action("无法进入工作目录", &msg, "检查 DIR 是否指向存在的目录");
        return Err(());
    }

    let agent = load_agent(&options).map_err(|msg| {
        log_error_action("无法选择模型", &msg, "编号见 clash 启动时的模型列表");
    })?;

    run_lark_daemon(options, agent).map_err(|msg| {
        log_error_action(
            "无法启动飞书监听",
            &msg,
            "请先安装并完成 lark-cli auth login",
        );
    })
}

fn enter_working_dir() -> Result<(), String> {
    let Some(path) = resolve_working_dir(env::var("DIR").ok().as_deref())? else {
        return Ok(());
    };
    env::set_current_dir(&path).map_err(|err| format!("无法进入 {}: {err}", path.display()))
}

/// 解析 -m 序号，缺省取首项；越界返回全部可选编号
fn pick_run_choice(
    choices: &[RunModelChoice],
    index: Option<usize>,
) -> Result<&RunModelChoice, String> {
    let labels: Vec<&str> = choices.iter().map(|choice| choice.label.as_str()).collect();
    let index = config::resolve_choice_index(&labels, index)?;
    Ok(&choices[index])
}

fn load_agent(options: &LarkOptions) -> Result<AgentConfig, String> {
    let choices = config::collect_run_choices()
        .map_err(|_| "未找到 Clash 配置，请先运行 clash config".to_string())?;
    let choice = pick_run_choice(&choices, options.model_index).map_err(|err| {
        if choices.is_empty() {
            "未找到 Clash 配置，请先运行 clash config".to_string()
        } else {
            err
        }
    })?;

    let max_ctx = context_size_marker(&choice.model);
    let model = remove_size_marker(&choice.model);
    let auth_token = crypto::decrypt_token(&choice.config.auth_token_encrypted)
        .map_err(|_| "无法解密 API Key".to_string())?;

    Ok(AgentConfig {
        base_url: choice.config.base_url.clone(),
        auth_token,
        model,
        effort: choice.config.effort.clone(),
        max_ctx,
        system_prompt_file: config::read_system_prompt()
            .map(|_| config::system_prompt_path().to_string_lossy().to_string()),
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

    let mut status = SessionStatus::new(&agent);
    let mut turn: Option<ActiveTurn> = None;
    let mut pending = VecDeque::new();

    loop {
        if turn.is_some() {
            match rx.try_recv() {
                Ok(message) => {
                    if let Err(err) = handle_chat_message(
                        &chat,
                        &mut session,
                        &mut status,
                        &mut turn,
                        &mut pending,
                        message,
                    ) {
                        log_error_action("会话处理失败", &err, "将继续监听后续消息");
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => return,
            }

            let Some(active) = turn.as_mut() else {
                continue;
            };
            match session.recv_event(Duration::from_millis(50)) {
                Ok(Some(event)) => {
                    match pump_turn_events(&mut session, active, &mut status, event) {
                        Ok(TurnProgress::Finished) => {
                            let _finished = turn.take().expect("回合仍在");
                        }
                        Ok(TurnProgress::Busy) => {}
                        Err(err) => {
                            fail_turn(&mut turn, &status, &format!("处理已中断: {err}"));
                        }
                    }
                }
                Ok(None) => {
                    if let Err(err) = drain_outbound(active, &status) {
                        log_verbose(&format!("刷新卡片失败：{err}"));
                    }
                }
                Err(err) => {
                    fail_turn(&mut turn, &status, &format!("Claude 进程异常退出: {err}"));
                }
            }
            continue;
        }

        if let Some(message) = pending.pop_front() {
            if let Err(err) = handle_chat_message(
                &chat,
                &mut session,
                &mut status,
                &mut turn,
                &mut pending,
                message,
            ) {
                log_error_action("会话处理失败", &err, "将继续监听后续消息");
            }
            continue;
        }

        match rx.recv() {
            Ok(message) => {
                if let Err(err) = handle_chat_message(
                    &chat,
                    &mut session,
                    &mut status,
                    &mut turn,
                    &mut pending,
                    message,
                ) {
                    log_error_action("会话处理失败", &err, "将继续监听后续消息");
                }
            }
            Err(_) => return,
        }
    }
}

struct SessionStatus {
    model: String,
    max_ctx: u64,
    usage: TokenUsage,
    started: Instant,
    dir: String,
}

impl SessionStatus {
    fn new(agent: &AgentConfig) -> Self {
        Self {
            model: agent.model.clone(),
            max_ctx: agent.max_ctx.unwrap_or(DEFAULT_CONTEXT_TOKENS),
            usage: TokenUsage::default(),
            started: Instant::now(),
            dir: session_dir_name(),
        }
    }

    fn observe(&mut self, event: &Value) {
        let before = self.usage.total();
        self.usage.observe(event);
        let after = self.usage.total();
        if after.is_some() && before != after {
            log_verbose(&format!("ctx observe: {before:?} -> {after:?} usage={:?}", self.usage));
        }
    }

    fn reset(&mut self) {
        self.usage.reset();
    }

    fn footer(&self) -> String {
        let total = self.usage.total();
        let percent = format_context_percent(total, Some(self.max_ctx));
        log_verbose(&format!("ctx: total={total:?} max_ctx={} -> {percent}", self.max_ctx));
        format_session_footer(
            &self.model,
            &percent,
            &format_elapsed(self.started.elapsed()),
            &self.dir,
        )
    }
}

struct ActiveTurn {
    card: CardStream,
    state: ClaudeRenderState,
    outbound: VecDeque<OutboundOp>,
    close_note: Option<String>,
}

enum TurnProgress {
    Busy,
    Finished,
}

fn session_dir_name() -> String {
    env::current_dir()
        .ok()
        .and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "-".to_string())
}

fn handle_chat_message(
    chat: &LarkChat,
    session: &mut ClaudeSession,
    status: &mut SessionStatus,
    turn: &mut Option<ActiveTurn>,
    pending: &mut VecDeque<LarkMessage>,
    message: LarkMessage,
) -> Result<(), String> {
    if parse_session_command(&message.text) == Some(SessionCommand::Stop) {
        return handle_stop(session, status, turn, &message);
    }
    if turn.is_some() {
        pending.push_back(message);
        return Ok(());
    }
    start_turn(chat, session, status, turn, message)
}

fn handle_stop(
    session: &mut ClaudeSession,
    status: &mut SessionStatus,
    turn: &mut Option<ActiveTurn>,
    message: &LarkMessage,
) -> Result<(), String> {
    let Some(mut active) = turn.take() else {
        let _ = send_lark_reply(&message.message_id, "当前没有进行中的任务");
        return Ok(());
    };

    session.interrupt()?;
    drain_until_result(session, status, &mut active, Duration::from_secs(8));
    abort_turn(&mut active, status, "已停止");
    if let Err(err) = send_lark_reply(&message.message_id, "已停止") {
        log_verbose(&format!("中断回复发送失败：{err}"));
    }
    Ok(())
}

fn drain_until_result(
    session: &mut ClaudeSession,
    status: &mut SessionStatus,
    turn: &mut ActiveTurn,
    budget: Duration,
) {
    let deadline = Instant::now() + budget;
    while Instant::now() < deadline {
        match session.recv_event(Duration::from_millis(50)) {
            Ok(Some(event)) => {
                status.observe(&event);
                let _ = fold_turn_event(turn, &event);
                if event.get("type").and_then(Value::as_str) == Some("result") {
                    return;
                }
            }
            Ok(None) => {}
            Err(_) => return,
        }
    }
}

fn start_turn(
    chat: &LarkChat,
    session: &mut ClaudeSession,
    status: &mut SessionStatus,
    turn: &mut Option<ActiveTurn>,
    message: LarkMessage,
) -> Result<(), String> {
    log_message_received(chat, &message);
    status.reset();
    let mut card =
        match CardStream::open(&message.message_id, "收到，开始处理...", &status.footer()) {
            Ok(card) => card,
            Err(err) => {
                log_error_action(
                    "无法创建流式卡片",
                    &err,
                    "请检查飞书应用 cardkit 和消息权限后重试",
                );
                let _ = send_lark_reply(&message.message_id, &format!("流式卡片创建失败: {err}"));
                return Ok(());
            }
        };
    if let Err(err) = session.send_prompt(&message.text) {
        let _ = card.close(&format!("处理已中断: {err}"), &status.footer());
        return Err(err);
    }
    *turn = Some(ActiveTurn {
        card,
        state: ClaudeRenderState::new(),
        outbound: VecDeque::new(),
        close_note: None,
    });
    Ok(())
}

fn pump_turn_events(
    session: &mut ClaudeSession,
    turn: &mut ActiveTurn,
    status: &mut SessionStatus,
    first: Value,
) -> Result<TurnProgress, String> {
    let mut step = apply_turn_event(turn, status, &first);
    while matches!(step, PumpStep::Busy) {
        match session.try_recv_event()? {
            Some(event) => step = apply_turn_event(turn, status, &event),
            None => break,
        }
    }
    drain_if_needed(turn, status, step)?;
    Ok(match step {
        PumpStep::Finished => TurnProgress::Finished,
        _ => TurnProgress::Busy,
    })
}

#[derive(Clone, Copy)]
enum PumpStep {
    Busy,
    Commit,
    Finished,
}

fn fold_turn_event(turn: &mut ActiveTurn, event: &Value) -> PumpStep {
    if event.get("type").and_then(Value::as_str) == Some("result") {
        if let Some(result) = result_text(event).filter(|s| !s.trim().is_empty()) {
            turn.state.set_answer_if_empty(&result);
        }
        return PumpStep::Finished;
    }
    let fold = turn.state.apply_event(event);
    if !matches!(fold, Fold::Unchanged) {
        log_verbose(&format!("fold={fold:?}"));
    }
    queue_fold(&mut turn.outbound, fold);
    if fold.breaks_pump() {
        return PumpStep::Commit;
    }
    PumpStep::Busy
}

fn apply_turn_event(turn: &mut ActiveTurn, status: &mut SessionStatus, event: &Value) -> PumpStep {
    status.observe(event);
    let step = fold_turn_event(turn, event);
    if matches!(step, PumpStep::Finished) {
        queue_result(&mut turn.outbound);
    }
    step
}

fn drain_if_needed(
    turn: &mut ActiveTurn,
    status: &SessionStatus,
    step: PumpStep,
) -> Result<(), String> {
    if matches!(step, PumpStep::Commit | PumpStep::Finished) || turn.state.has_answer() {
        drain_outbound(turn, status)
    } else {
        Ok(())
    }
}

fn drain_outbound(turn: &mut ActiveTurn, status: &SessionStatus) -> Result<(), String> {
    while let Some(op) = turn.outbound.pop_front() {
        log_verbose(&format!("op={op:?}"));
        let footer = status.footer();
        match op {
            OutboundOp::PatchProgress => {
                turn.card.update(&turn.state.patch_body(), &footer)?;
            }
            OutboundOp::CloseProgress => {
                turn.card.close(&final_close_body(turn), &footer)?;
            }
        }
    }
    Ok(())
}

fn final_close_body(turn: &ActiveTurn) -> String {
    if turn.close_note.is_some() {
        return progress_close_body(turn);
    }
    let body = turn.state.render();
    if body.trim().is_empty() || body == "- 正在处理…" {
        "Claude 没有返回内容".to_string()
    } else {
        body
    }
}

fn progress_close_body(turn: &ActiveTurn) -> String {
    let rendered = turn.state.render_stream();
    match &turn.close_note {
        Some(note) if rendered.trim().is_empty() || rendered == "- 正在处理…" => note.clone(),
        Some(note) => format!("{rendered}\n\n{note}"),
        None => rendered,
    }
}

fn abort_turn(turn: &mut ActiveTurn, status: &SessionStatus, note: &str) {
    queue_stop(&mut turn.outbound);
    turn.close_note = Some(note.to_string());
    if let Err(err) = drain_outbound(turn, status) {
        log_verbose(&format!("关闭卡片失败：{err}"));
        let _ = turn.card.close(note, &status.footer());
    }
}

/// 被动中断统一入口：关闭卡片 + 回复首轮消息 + 留痕发送失败
fn fail_turn(turn: &mut Option<ActiveTurn>, status: &SessionStatus, note: &str) {
    let Some(mut failed) = turn.take() else {
        return;
    };
    let reply_to = failed.card.reply_to().to_string();
    abort_turn(&mut failed, status, note);
    if let Err(err) = send_lark_reply(&reply_to, note) {
        log_verbose(&format!("中断回复发送失败：{err}"));
    }
}

fn log_ready(manager: &LarkChat, options: &LarkOptions, agent: &AgentConfig, chat_count: usize) {
    println!("Lark 已启动");
    println!("管理群：{}", manager.name);
    println!("会话前缀：{}", options.prefix);
    println!("已发现会话：{chat_count} 个");
    println!("使用模型：{}", agent.model);
    println!("工作目录：{}", session_dir_name());
    println!();
    println!("在管理群发送：新会话/接入 群名称");
    println!("会话群发送 /stop 可中断当前回合");
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
