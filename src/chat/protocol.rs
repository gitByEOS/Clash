use serde::{Deserialize, Serialize};

pub const DEFAULT_WATCH_TIMEOUT_SECS: u64 = 30 * 60;
pub const DEFAULT_LEASE_SECS: u64 = 60;
pub const MAX_MESSAGE_LINE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatMessage {
    pub id: String,
    pub ts: u64,
    pub from: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentState {
    pub name: String,
    pub last_active_ts: u64,
    pub lease_until_ts: u64,
    #[serde(default)]
    pub cursor_offset: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

pub fn infer_target(text: &str) -> Option<String> {
    let mentions = mentions(text);
    if mentions
        .iter()
        .any(|mention| mention.eq_ignore_ascii_case("all"))
    {
        return Some("all".to_string());
    }
    mentions.into_iter().next()
}

pub fn should_handle(message: &ChatMessage, name: &str) -> bool {
    if message.from == name {
        return false;
    }
    if let Some(to) = &message.to {
        return to.eq_ignore_ascii_case("all") || to.eq_ignore_ascii_case(name);
    }
    mentions_target(&message.text, name) || mentions_target(&message.text, "all")
}

pub fn build_wake_prompt(message: &ChatMessage, name: &str) -> String {
    let payload = serde_json::to_string_pretty(message).unwrap_or_else(|_| "{}".to_string());
    [
        "聊天室有新消息，请读取并执行。",
        "",
        "消息：",
        &payload,
        "",
        "规则：",
        &format!("- 若消息发给 @{name} 或 @all，按任务说明执行"),
        "- 完成后用 `clash chat send` 回写进度",
    ]
    .join("\n")
}

fn mentions_target(text: &str, target: &str) -> bool {
    mentions(text)
        .into_iter()
        .any(|mention| mention.eq_ignore_ascii_case(target))
}

fn mentions(text: &str) -> Vec<String> {
    let chars = text.char_indices().collect::<Vec<_>>();
    let mut result = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let (at_idx, ch) = chars[i];
        if ch != '@' {
            i += 1;
            continue;
        }

        let start = at_idx + ch.len_utf8();
        let mut end = start;
        i += 1;
        while i < chars.len() {
            let (idx, c) = chars[i];
            if !is_mention_char(c) {
                break;
            }
            end = idx + c.len_utf8();
            i += 1;
        }

        if end > start {
            result.push(text[start..end].to_string());
        }
    }
    result
}

fn is_mention_char(ch: char) -> bool {
    ch.is_alphanumeric() || matches!(ch, '_' | '-' | '.')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(from: &str, to: Option<&str>, text: &str) -> ChatMessage {
        ChatMessage {
            id: "1".to_string(),
            ts: 1,
            from: from.to_string(),
            to: to.map(str::to_string),
            text: text.to_string(),
            status: None,
        }
    }

    #[test]
    fn mention_matches_complete_name() {
        assert!(should_handle(&message("A", None, "@bob 看这里"), "bob"));
        assert!(!should_handle(&message("A", None, "@bob2 看这里"), "bob"));
    }

    #[test]
    fn mention_all_matches_everyone_except_sender() {
        assert!(should_handle(&message("A", None, "@all 同步"), "B"));
        assert!(!should_handle(&message("A", None, "@all 同步"), "A"));
    }

    #[test]
    fn explicit_target_wins() {
        assert!(should_handle(&message("A", Some("B"), "hello"), "B"));
        assert!(!should_handle(&message("A", Some("C"), "@B hello"), "B"));
    }

    #[test]
    fn infer_target_prefers_all() {
        assert_eq!(infer_target("@B @all"), Some("all".to_string()));
        assert_eq!(infer_target("ping @B"), Some("B".to_string()));
        assert_eq!(infer_target("ping"), None);
    }
}
