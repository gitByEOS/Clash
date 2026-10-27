use crate::lark::types::{LarkOptions, DEFAULT_POLL_SECS, DEFAULT_PREFIX};
use std::path::{Path, PathBuf};

pub fn parse_lark_args(args: &[String]) -> Result<LarkOptions, String> {
    let mut prefix = DEFAULT_PREFIX.to_string();
    let mut poll_secs = DEFAULT_POLL_SECS;
    let mut once = false;
    let mut model_index = None;
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "--prefix" => {
                i += 1;
                let value = args.get(i).ok_or_else(|| "--prefix 缺少值".to_string())?;
                prefix = value.trim().to_string();
                if prefix.is_empty() {
                    return Err("--prefix 不能为空".to_string());
                }
            }
            "--poll-secs" => {
                i += 1;
                let value = args
                    .get(i)
                    .ok_or_else(|| "--poll-secs 缺少值".to_string())?;
                poll_secs = value
                    .parse::<u64>()
                    .map_err(|_| "--poll-secs 必须是正整数".to_string())?;
                if poll_secs == 0 {
                    return Err("--poll-secs 必须大于 0".to_string());
                }
            }
            "--once" => once = true,
            "-m" => {
                i += 1;
                let value = args.get(i).ok_or_else(|| "-m 缺少值".to_string())?;
                model_index = Some(
                    value
                        .parse::<usize>()
                        .map_err(|_| "-m 必须是正整数".to_string())?,
                );
            }
            other => return Err(format!("未知参数: {other}")),
        }
        i += 1;
    }

    Ok(LarkOptions {
        prefix,
        poll_secs,
        once,
        model_index,
    })
}

pub fn parse_create_session_command(text: &str) -> Option<String> {
    let input = text.trim();
    for keyword in ["新建群组", "新会话", "接入"] {
        if let Some(rest) = input.strip_prefix(keyword) {
            let name = trim_command_separator(rest).to_string();
            if !name.is_empty() {
                return Some(name);
            }
        }
    }
    None
}

fn trim_command_separator(text: &str) -> &str {
    text.trim_start_matches(|c: char| {
        c == ':' || c == '：' || c == ',' || c == '，' || c.is_whitespace()
    })
    .trim()
}

/// 会话群斜杠命令。整句匹配，避免误伤正文里的 /stop
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionCommand {
    Stop,
}

pub fn parse_session_command(text: &str) -> Option<SessionCommand> {
    match text.trim().to_ascii_lowercase().as_str() {
        "/stop" => Some(SessionCommand::Stop),
        _ => None,
    }
}

pub fn resolve_working_dir(dir: Option<&str>) -> Result<Option<PathBuf>, String> {
    let Some(raw) = dir.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let path = Path::new(raw);
    if !path.is_dir() {
        return Err(format!("DIR 不是目录: {raw}"));
    }
    Ok(Some(path.to_path_buf()))
}

pub fn normalize_session_chat_name(raw_name: &str, prefix: &str) -> String {
    let name = raw_name.trim();
    if name.starts_with(prefix) {
        name.to_string()
    } else {
        format!("{prefix}{name}")
    }
}
