use crate::lark::types::{LarkOptions, DEFAULT_POLL_SECS, DEFAULT_PREFIX};

pub fn parse_lark_args(args: &[String]) -> Result<LarkOptions, String> {
    let mut prefix = DEFAULT_PREFIX.to_string();
    let mut poll_secs = DEFAULT_POLL_SECS;
    let mut once = false;
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
            other => return Err(format!("未知参数: {other}")),
        }
        i += 1;
    }

    Ok(LarkOptions {
        prefix,
        poll_secs,
        once,
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

pub fn normalize_session_chat_name(raw_name: &str, prefix: &str) -> String {
    let name = raw_name.trim();
    if name.starts_with(prefix) {
        name.to_string()
    } else {
        format!("{prefix}{name}")
    }
}
