use crate::lark::lark_cli::{assert_lark_api_ok, run_lark_cli_json};
use crate::lark::render::truncate_chars;
use serde_json::{json, Value};
use std::env;

const BODY_ELEMENT_ID: &str = "body_md";
const TEXT_MAX: usize = 100_000;

pub struct CardStream {
    card_id: String,
    sequence: u64,
    closed: bool,
    text: String,
}

impl Drop for CardStream {
    fn drop(&mut self) {
        if !self.closed {
            let _ = self.close("已中断");
        }
    }
}

impl CardStream {
    pub fn open(reply_to: &str, initial_text: &str) -> Result<Self, String> {
        let card = build_streaming_card(initial_text, true);
        let card_data = json!({
            "type": "card_json",
            "data": card.to_string()
        })
        .to_string();
        let created = run_lark_cli_json(&[
            "api",
            "POST",
            "/open-apis/cardkit/v1/cards",
            "--data",
            &card_data,
            "--as",
            "bot",
            "--json",
        ])?;
        assert_lark_api_ok(&created, "cardkit.card.create")?;
        let card_id = created
            .pointer("/data/card_id")
            .and_then(Value::as_str)
            .ok_or_else(|| "cardkit.card.create 未返回 card_id".to_string())?
            .to_string();

        let content = json!({
            "type": "card",
            "data": { "card_id": &card_id }
        })
        .to_string();
        let sent = run_lark_cli_json(&[
            "im",
            "+messages-reply",
            "--message-id",
            reply_to,
            "--msg-type",
            "interactive",
            "--content",
            &content,
            "--as",
            "bot",
            "--json",
        ])?;
        assert_lark_api_ok(&sent, "im.send.card")?;

        Ok(Self {
            card_id,
            sequence: 1,
            closed: false,
            text: String::new(),
        })
    }

    pub fn update(&mut self, text: &str) -> Result<(), String> {
        let Some(content) = accept_monotonic_text(&mut self.text, text) else {
            return Ok(());
        };
        self.sequence += 1;
        let path = format!(
            "/open-apis/cardkit/v1/cards/{}/elements/{}/content",
            self.card_id, BODY_ELEMENT_ID
        );
        let data = json!({
            "content": content,
            "sequence": self.sequence
        })
        .to_string();
        let res = run_lark_cli_json(&[
            "api", "PUT", &path, "--data", &data, "--as", "bot", "--json",
        ])?;
        assert_lark_api_ok(&res, "cardkit.element.content")
    }

    pub fn close(&mut self, text: &str) -> Result<(), String> {
        if self.closed {
            return Ok(());
        }
        self.sequence += 1;
        let path = format!("/open-apis/cardkit/v1/cards/{}", self.card_id);
        let card = build_streaming_card(text, false);
        let data = json!({
            "card": {
                "type": "card_json",
                "data": card.to_string()
            },
            "sequence": self.sequence
        })
        .to_string();
        let res = run_lark_cli_json(&[
            "api", "PUT", &path, "--data", &data, "--as", "bot", "--json",
        ])?;
        assert_lark_api_ok(&res, "cardkit.card.update")?;
        self.closed = true;
        Ok(())
    }
}

fn build_streaming_card(text: &str, streaming: bool) -> Value {
    let body = prepare_card_content(text);
    let mut config = json!({
        "streaming_mode": streaming,
        "summary": { "content": strip_markdown_for_summary(&body) }
    });
    if streaming {
        let speed_ms = card_print_frequency_ms();
        config["streaming_config"] = json!({
            "print_frequency_ms": {
                "default": speed_ms,
                "android": speed_ms,
                "ios": speed_ms,
                "pc": speed_ms
            },
            "print_step": { "default": 1, "android": 1, "ios": 1, "pc": 1 },
            "print_strategy": "delay"
        });
    }

    json!({
        "schema": "2.0",
        "config": config,
        "body": {
            "elements": [{
                "tag": "markdown",
                "content": body,
                "element_id": BODY_ELEMENT_ID
            }]
        }
    })
}

fn card_print_frequency_ms() -> u64 {
    env::var("CLASH_LARK_CARD_PRINT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(30)
}

fn prepare_card_content(text: &str) -> String {
    let raw = truncate_chars(text.trim(), TEXT_MAX);
    if raw.is_empty() {
        "生成中...".to_string()
    } else {
        normalize_code_blocks(&raw)
    }
}

fn normalize_code_blocks(text: &str) -> String {
    text.lines()
        .map(|line| {
            let trimmed = line.trim();
            if trimmed.starts_with("```") {
                trimmed.to_string()
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn strip_markdown_for_summary(text: &str) -> String {
    let cleaned = text
        .chars()
        .filter(|c| !matches!(c, '*' | '_' | '~' | '#' | '`'))
        .collect::<String>();
    let summary = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let summary = truncate_chars(summary.trim(), 80);
    if summary.is_empty() {
        "生成中".to_string()
    } else {
        summary
    }
}

fn accept_monotonic_text(current: &mut String, text: &str) -> Option<String> {
    let next = prepare_card_content(text);
    if !current.is_empty() && !next.starts_with(current.as_str()) {
        return None;
    }
    if next == *current {
        return None;
    }
    *current = next.clone();
    Some(next)
}
