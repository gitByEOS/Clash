use crate::lark::lark_cli::{assert_lark_api_ok, log_verbose, run_lark_cli_json};
use crate::lark::render::truncate_chars;
use serde_json::{json, Value};
use std::env;

const BODY_ELEMENT_ID: &str = "body_md";
const FOOTER_ELEMENT_ID: &str = "footer_md";
/// 单卡内容上限：超过即翻页新开一张卡，避免飞书 cardkit 渲染超长内容崩溃
const PAGE_SIZE: usize = 10_000;
/// 全量内容上限：超出后截断，作为兜底防极端输出无限增长
const TEXT_MAX: usize = 50_000;

pub struct CardStream {
    card_id: String,
    /// 翻页时新卡回复的目标消息 id，沿用首轮消息保持串接
    reply_to: String,
    sequence: u64,
    closed: bool,
    /// 全量 normalized 文本，单调前缀检查的基准
    text: String,
    /// 当前页在全量中的起始字符偏移，翻页时推进
    page_start: usize,
}

impl Drop for CardStream {
    fn drop(&mut self) {
        if !self.closed {
            let _ = self.close("已停止", "");
        }
    }
}

impl CardStream {
    pub fn open(reply_to: &str, initial_text: &str, footer: &str) -> Result<Self, String> {
        let card_id = create_and_reply(reply_to, initial_text, footer, true)?;
        Ok(Self {
            card_id,
            reply_to: reply_to.to_string(),
            sequence: 1,
            closed: false,
            text: String::new(),
            page_start: 0,
        })
    }

    pub fn reply_to(&self) -> &str {
        &self.reply_to
    }

    /// 写入 body 与 footer；全量超过 page_start+PAGE_SIZE 时翻页：旧卡封顶关闭，新卡承接后续
    pub fn update(&mut self, text: &str, footer: &str) -> Result<(), String> {
        if !accept_monotonic_text(&mut self.text, text) {
            log_verbose(&format!("update 跳过，current 长度={}", self.text.chars().count()));
            return Ok(());
        }
        let (body, need_split) = page_content(&self.text, self.page_start);
        if need_split {
            log_verbose(&format!(
                "翻页：page_start={}->{} total={}",
                self.page_start,
                self.page_start + PAGE_SIZE,
                self.text.chars().count()
            ));
            self.close(&body, footer)?;
            self.page_start += PAGE_SIZE;
            let (initial, _) = page_content(&self.text, self.page_start);
            self.card_id = create_and_reply(&self.reply_to, &initial, footer, true)?;
            self.sequence = 1;
            self.closed = false;
            return Ok(());
        }
        log_verbose(&format!("update 写入，body 长度={}", body.chars().count()));
        self.put_element(BODY_ELEMENT_ID, &body)?;
        self.put_element(FOOTER_ELEMENT_ID, &prepare_footer_content(footer))
    }

    pub fn close(&mut self, text: &str, footer: &str) -> Result<(), String> {
        if self.closed {
            return Ok(());
        }
        self.sequence += 1;
        let path = format!("/open-apis/cardkit/v1/cards/{}", self.card_id);
        let card = build_streaming_card(text, footer, false);
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

    fn put_element(&mut self, element_id: &str, content: &str) -> Result<(), String> {
        self.sequence += 1;
        let path = format!(
            "/open-apis/cardkit/v1/cards/{}/elements/{element_id}/content",
            self.card_id
        );
        let data = json!({
            "content": content,
            "sequence": self.sequence
        })
        .to_string();
        let res = run_lark_cli_json(&[
            "api", "PUT", &path, "--data", &data, "--as", "bot", "--json",
        ])?;
        assert_lark_api_ok(&res, "cardkit.element.content")?;
        log_verbose(&format!("put_element {element_id} ok, 长度={}", content.chars().count()));
        Ok(())
    }
}

fn create_and_reply(
    reply_to: &str,
    text: &str,
    footer: &str,
    streaming: bool,
) -> Result<String, String> {
    let card = build_streaming_card(text, footer, streaming);
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
    log_verbose(&format!("create_and_reply 创建卡片 card_id={card_id} reply_to={reply_to} streaming={streaming}"));

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
    log_verbose(&format!("create_and_reply reply 发送成功 card_id={card_id} sent={}", sent.to_string().chars().take(200).collect::<String>()));
    Ok(card_id)
}

fn build_streaming_card(text: &str, footer: &str, streaming: bool) -> Value {
    let body = prepare_card_content(text);
    let footer = prepare_footer_content(footer);
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
            "elements": [
                {
                    "tag": "markdown",
                    "content": body,
                    "element_id": BODY_ELEMENT_ID
                },
                {
                    "tag": "markdown",
                    "content": footer,
                    "text_size": "notation",
                    "element_id": FOOTER_ELEMENT_ID
                }
            ]
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
        let normalized = normalize_code_blocks(&raw);
        ensure_code_fences_closed(&normalized)
    }
}

/// ``` 围栏数量为奇数时补一个闭合围栏，避免飞书 cardkit 渲染不闭合代码块崩溃
fn ensure_code_fences_closed(text: &str) -> String {
    let fence_count = text.lines().filter(|line| line.trim().starts_with("```")).count();
    if fence_count % 2 == 1 {
        format!("{text}\n```")
    } else {
        text.to_string()
    }
}

fn prepare_footer_content(text: &str) -> String {
    let raw = truncate_chars(text.trim(), 200);
    if raw.is_empty() {
        " ".to_string()
    } else {
        raw
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

/// 计算当前页内容切片与是否需要翻页（纯逻辑，便于单测）
fn current_page(text: &str, page_start: usize) -> (String, bool) {
    let total = text.chars().count();
    let need_split = total > page_start + PAGE_SIZE;
    let page: String = text.chars().skip(page_start).take(PAGE_SIZE).collect();
    (page, need_split)
}

/// 取当前页内容并确保 ``` 围栏配对，同时返回是否需要翻页：
/// - 开头补开始围栏：若 page_start 落在上一页未闭合的 code block 内
/// - 尾部补闭合围栏：若切片切在 code block 内部
fn page_content(text: &str, page_start: usize) -> (String, bool) {
    let (page, need_split) = current_page(text, page_start);
    let before: String = text.chars().take(page_start).collect();
    let in_code = before
        .lines()
        .filter(|line| line.trim().starts_with("```"))
        .count()
        % 2
        == 1;
    let content = if in_code {
        format!("```\n{page}")
    } else {
        page
    };
    (ensure_code_fences_closed(&content), need_split)
}

/// 接受单调增长的文本：normalized 与 current 不同则更新 current 返回 true，相同或空返回 false
/// 闭合围栏由 page_content/prepare_card_content 负责，这里只判等防重复写入
fn accept_monotonic_text(current: &mut String, text: &str) -> bool {
    let raw = truncate_chars(text.trim(), TEXT_MAX);
    if raw.is_empty() {
        return false;
    }
    let normalized = normalize_code_blocks(&raw);
    // render() 严格单调增长（thinking/tool 就地扩展、answer 累积），不会真回滚；
    // 只防内容未变化时重复写入，不防中间行增长（那会误判回滚卡死卡片）
    if normalized == *current {
        return false;
    }
    *current = normalized;
    true
}
