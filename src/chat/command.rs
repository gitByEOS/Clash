use crate::chat::protocol::{
    build_wake_prompt, infer_target, should_handle, ChatMessage, DEFAULT_WATCH_TIMEOUT_SECS,
};
use crate::chat::store::{resolve_rooms_root, unix_millis, unix_secs, ChatStore};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde_json::json;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

const WATCH_LEASE_TICK_SECS: u64 = 15;

pub fn do_chat(args: &[String]) -> Result<(), ()> {
    let result = match args.first().map(String::as_str) {
        Some("send") => do_send(&args[1..]),
        Some("watch") => do_watch(&args[1..]),
        Some("history") => do_history(&args[1..]),
        _ => Err(chat_usage().to_string()),
    };
    if let Err(err) = result {
        println!("{}", error_json(&err));
    }
    Ok(())
}

fn do_send(args: &[String]) -> Result<(), String> {
    let parsed = parse_send_args(args)?;
    let store = chat_store(parsed.path.as_deref())?;
    let to = parsed.to.or_else(|| infer_target(&parsed.text));
    if let Some(target) = &to {
        if !target.eq_ignore_ascii_case("all") && !store.is_agent_online(&parsed.room, target)? {
            let known_agents = store.list_agent_names(&parsed.room)?;
            let known = if known_agents.is_empty() {
                String::new()
            } else {
                format!("，已知 Agent: {}", known_agents.join(", "))
            };
            return Err(format!("目标 @{target} 不在线{known}"));
        }
    }

    store.refresh_lease(&parsed.room, &parsed.name, parsed.status.as_deref())?;
    let message = ChatMessage {
        id: new_message_id(),
        ts: unix_secs(),
        from: parsed.name,
        to,
        text: parsed.text,
        status: parsed.status,
    };
    store.append_message(&parsed.room, &message)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&message).unwrap_or_else(|_| "{}".to_string())
    );
    Ok(())
}

fn do_watch(args: &[String]) -> Result<(), String> {
    let parsed = parse_watch_args(args)?;
    let store = chat_store(parsed.path.as_deref())?;
    let deadline = Instant::now() + Duration::from_secs(parsed.timeout_secs);
    store.refresh_lease(&parsed.room, &parsed.name, parsed.status.as_deref())?;

    if let Some(prompt) = check_watch_messages(&store, &parsed)? {
        return finish_watch(&store, &parsed, prompt);
    }

    match create_message_watcher(&store, &parsed.room) {
        Ok((watcher, watcher_rx)) => {
            event_watch_loop(&store, &parsed, deadline, watcher, watcher_rx)
        }
        Err(_) => poll_watch_loop(&store, &parsed, deadline),
    }
}

fn event_watch_loop(
    store: &ChatStore,
    parsed: &WatchArgs,
    deadline: Instant,
    _watcher: RecommendedWatcher,
    watcher_rx: Receiver<notify::Result<Event>>,
) -> Result<(), String> {
    let messages_path = store.messages_path(&parsed.room)?;
    loop {
        if Instant::now() >= deadline {
            return Err("等待超时".to_string());
        }

        let remaining = deadline.saturating_duration_since(Instant::now());
        let wait = remaining.min(Duration::from_secs(WATCH_LEASE_TICK_SECS));
        match watcher_rx.recv_timeout(wait) {
            Ok(Ok(event)) => {
                if !is_messages_event(&event, &messages_path) {
                    continue;
                }
                if let Some(prompt) = check_watch_messages(store, parsed)? {
                    return finish_watch(store, parsed, prompt);
                }
                store.refresh_lease(&parsed.room, &parsed.name, parsed.status.as_deref())?;
            }
            Ok(Err(_)) => {
                if let Some(prompt) = check_watch_messages(store, parsed)? {
                    return finish_watch(store, parsed, prompt);
                }
                store.refresh_lease(&parsed.room, &parsed.name, parsed.status.as_deref())?;
            }
            Err(RecvTimeoutError::Timeout) => {
                store.refresh_lease(&parsed.room, &parsed.name, parsed.status.as_deref())?;
            }
            Err(RecvTimeoutError::Disconnected) => return poll_watch_loop(store, parsed, deadline),
        }
    }
}

fn poll_watch_loop(store: &ChatStore, parsed: &WatchArgs, deadline: Instant) -> Result<(), String> {
    loop {
        if Instant::now() >= deadline {
            return Err("等待超时".to_string());
        }

        if let Some(prompt) = check_watch_messages(store, parsed)? {
            return finish_watch(store, parsed, prompt);
        }

        store.refresh_lease(&parsed.room, &parsed.name, parsed.status.as_deref())?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        thread::sleep(remaining.min(Duration::from_millis(parsed.poll_ms)));
    }
}

fn check_watch_messages(store: &ChatStore, parsed: &WatchArgs) -> Result<Option<String>, String> {
    if let Some(expect) = &parsed.expect {
        if !store.is_agent_online(&parsed.room, expect)? {
            return Err(format!("目标 @{expect} 不在线"));
        }
    }

    let cursor = store.read_cursor(&parsed.room, &parsed.name)?;
    let records = store.read_records_from(&parsed.room, cursor)?;
    let mut latest_offset = cursor;
    for record in records {
        latest_offset = record.end_offset;
        if should_handle(&record.message, &parsed.name) {
            store.write_cursor(&parsed.room, &parsed.name, record.end_offset)?;
            return Ok(Some(build_wake_prompt(&record.message, &parsed.name)));
        }
    }
    if latest_offset != cursor {
        store.write_cursor(&parsed.room, &parsed.name, latest_offset)?;
    }
    Ok(None)
}

fn finish_watch(store: &ChatStore, parsed: &WatchArgs, prompt: String) -> Result<(), String> {
    println!("{prompt}");
    store.refresh_lease(&parsed.room, &parsed.name, parsed.status.as_deref())?;
    Ok(())
}

fn create_message_watcher(
    store: &ChatStore,
    room: &str,
) -> Result<(RecommendedWatcher, Receiver<notify::Result<Event>>), String> {
    let (watcher_tx, watcher_rx) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |result| {
        let _ = watcher_tx.send(result);
    })
    .map_err(|e| format!("无法初始化文件事件监听: {e}"))?;
    watcher
        .watch(&store.messages_dir(room)?, RecursiveMode::NonRecursive)
        .map_err(|e| format!("无法监听消息目录: {e}"))?;
    Ok((watcher, watcher_rx))
}

fn is_messages_event(event: &Event, messages_path: &Path) -> bool {
    if !matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_)) {
        return false;
    }
    event.paths.iter().any(|path| {
        path == messages_path
            || path.file_name().and_then(|value| value.to_str()) == Some("messages.jsonl")
    })
}

fn do_history(args: &[String]) -> Result<(), String> {
    let parsed = parse_history_args(args)?;
    let store = chat_store(parsed.path.as_deref())?;
    let messages = store.history(&parsed.room, parsed.limit)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!(messages)).unwrap_or_else(|_| "[]".to_string())
    );
    Ok(())
}

#[derive(Debug)]
struct SendArgs {
    room: String,
    path: Option<String>,
    name: String,
    text: String,
    to: Option<String>,
    status: Option<String>,
}

#[derive(Debug)]
struct WatchArgs {
    room: String,
    path: Option<String>,
    name: String,
    expect: Option<String>,
    status: Option<String>,
    timeout_secs: u64,
    poll_ms: u64,
}

#[derive(Debug)]
struct HistoryArgs {
    room: String,
    path: Option<String>,
    limit: usize,
}

fn parse_send_args(args: &[String]) -> Result<SendArgs, String> {
    let mut room = None;
    let mut path = None;
    let mut name = None;
    let mut text = None;
    let mut to = None;
    let mut status = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--room" => room = Some(next_value(args, &mut i, "--room")?),
            "--path" => path = Some(next_value(args, &mut i, "--path")?),
            "--name" => name = Some(next_value(args, &mut i, "--name")?),
            "--text" => text = Some(next_value(args, &mut i, "--text")?),
            "--to" => to = Some(next_value(args, &mut i, "--to")?),
            "--status" => status = Some(next_value(args, &mut i, "--status")?),
            _ => return Err(chat_usage().to_string()),
        }
        i += 1;
    }

    Ok(SendArgs {
        room: required_room(room)?,
        path,
        name: required_name(name)?,
        text: required_text(text)?,
        to,
        status,
    })
}

fn parse_watch_args(args: &[String]) -> Result<WatchArgs, String> {
    let mut room = None;
    let mut path = None;
    let mut name = None;
    let mut expect = None;
    let mut status = None;
    let mut timeout_secs = DEFAULT_WATCH_TIMEOUT_SECS;
    let mut poll_ms = 500;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--room" => room = Some(next_value(args, &mut i, "--room")?),
            "--path" => path = Some(next_value(args, &mut i, "--path")?),
            "--name" => name = Some(next_value(args, &mut i, "--name")?),
            "--expect" => expect = Some(next_value(args, &mut i, "--expect")?),
            "--status" => status = Some(next_value(args, &mut i, "--status")?),
            "--timeout" => {
                timeout_secs = next_value(args, &mut i, "--timeout")?
                    .parse::<u64>()
                    .map_err(|_| "--timeout 必须是秒数".to_string())?
            }
            "--poll-ms" => {
                poll_ms = next_value(args, &mut i, "--poll-ms")?
                    .parse::<u64>()
                    .map_err(|_| "--poll-ms 必须是毫秒数".to_string())?
            }
            _ => return Err(chat_usage().to_string()),
        }
        i += 1;
    }

    Ok(WatchArgs {
        room: required_room(room)?,
        path,
        name: required_name(name)?,
        expect,
        status,
        timeout_secs,
        poll_ms,
    })
}

fn parse_history_args(args: &[String]) -> Result<HistoryArgs, String> {
    let mut room = None;
    let mut path = None;
    let mut limit = 20;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--room" => room = Some(next_value(args, &mut i, "--room")?),
            "--path" => path = Some(next_value(args, &mut i, "--path")?),
            "--limit" => {
                limit = next_value(args, &mut i, "--limit")?
                    .parse::<usize>()
                    .map_err(|_| "--limit 必须是正整数".to_string())?
            }
            _ => return Err(chat_usage().to_string()),
        }
        i += 1;
    }
    Ok(HistoryArgs {
        room: required_room(room)?,
        path,
        limit,
    })
}

fn next_value(args: &[String], i: &mut usize, flag: &str) -> Result<String, String> {
    *i += 1;
    args.get(*i)
        .cloned()
        .ok_or_else(|| format!("{flag} 缺少值"))
}

fn required_room(value: Option<String>) -> Result<String, String> {
    Ok(value
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(default_room_name))
}

fn required_name(value: Option<String>) -> Result<String, String> {
    value
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "--name 必填".to_string())
}

fn required_text(value: Option<String>) -> Result<String, String> {
    value
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "--text 不能为空".to_string())
}

fn new_message_id() -> String {
    format!("{}-{}", unix_millis(), std::process::id())
}

fn chat_store(path: Option<&str>) -> Result<ChatStore, String> {
    Ok(ChatStore::with_root(resolve_rooms_root(path)?))
}

fn error_json(message: &str) -> String {
    serde_json::to_string(&json!({ "error": message }))
        .unwrap_or_else(|_| "{\"error\":\"未知错误\"}".to_string())
}

fn default_room_name() -> String {
    let (year, month, day) = today_ymd();
    format!("room-{year:04}-{month:02}-{day:02}")
}

#[allow(unreachable_code)]
fn today_ymd() -> (u64, u64, u64) {
    #[cfg(unix)]
    {
        let secs = unix_secs() as libc::time_t;
        let mut tm = std::mem::MaybeUninit::<libc::tm>::zeroed();
        if !unsafe { libc::localtime_r(&secs, tm.as_mut_ptr()) }.is_null() {
            let tm = unsafe { tm.assume_init() };
            return (
                (tm.tm_year + 1900) as u64,
                (tm.tm_mon + 1) as u64,
                tm.tm_mday as u64,
            );
        }
    }
    unix_days_to_ymd(unix_secs() / 86400)
}

fn unix_days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    let mut year = 1970;
    while days >= year_days(year) {
        days -= year_days(year);
        year += 1;
    }
    let month_days = if is_leap_year(year) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    for (idx, month_day) in month_days.iter().enumerate() {
        if days < *month_day {
            return (year, idx as u64 + 1, days + 1);
        }
        days -= month_day;
    }
    (year, 12, 31)
}

fn year_days(year: u64) -> u64 {
    if is_leap_year(year) {
        366
    } else {
        365
    }
}

fn is_leap_year(year: u64) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

fn chat_usage() -> &'static str {
    "用法:
  clash chat send --name <名字> --text <消息> [--room <房间>] [--path <路径|URI>] [--to <名字|all>] [--status <状态>]
  clash chat watch --name <名字> [--room <房间>] [--path <路径|URI>] [--expect <名字>] [--timeout <秒>] [--poll-ms <毫秒>]
  clash chat history [--room <房间>] [--path <路径|URI>] [--limit <数量>]"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_room_uses_date_prefix() {
        assert!(default_room_name().starts_with("room-"));
        assert_eq!(default_room_name().len(), "room-2026-06-23".len());
    }

    #[test]
    fn unix_days_to_ymd_handles_leap_day() {
        assert_eq!(unix_days_to_ymd(0), (1970, 1, 1));
        assert_eq!(unix_days_to_ymd(19_782), (2024, 2, 29));
    }

    #[test]
    fn parse_send_args_accepts_path() {
        let args = vec![
            "--room".to_string(),
            "room-x".to_string(),
            "--path".to_string(),
            "share://team".to_string(),
            "--name".to_string(),
            "A".to_string(),
            "--text".to_string(),
            "@B hi".to_string(),
        ];
        let parsed = parse_send_args(&args).unwrap();
        assert_eq!(parsed.room, "room-x");
        assert_eq!(parsed.path.as_deref(), Some("share://team"));
    }

    #[test]
    fn parse_send_args_rejects_empty_text() {
        let args = vec![
            "--name".to_string(),
            "A".to_string(),
            "--text".to_string(),
            "  ".to_string(),
        ];
        assert!(parse_send_args(&args).is_err());
    }

    #[test]
    fn default_watch_timeout_is_30_minutes() {
        let args = vec!["--name".to_string(), "A".to_string()];
        let parsed = parse_watch_args(&args).unwrap();
        assert_eq!(parsed.timeout_secs, 30 * 60);
    }

    #[test]
    fn error_json_returns_error_object() {
        assert_eq!(error_json("消息过长"), "{\"error\":\"消息过长\"}");
    }
}
