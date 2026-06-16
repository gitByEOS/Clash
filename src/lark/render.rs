use serde_json::Value;
use std::collections::HashSet;

pub struct ClaudeRenderState {
    output: String,
    answer: String,
    seen_tools: HashSet<String>,
    is_tool_section_started: bool,
    is_answer_started: bool,
}

impl ClaudeRenderState {
    pub fn new() -> Self {
        Self {
            output: String::new(),
            answer: String::new(),
            seen_tools: HashSet::new(),
            is_tool_section_started: false,
            is_answer_started: false,
        }
    }

    pub fn apply_event(&mut self, event: &Value) -> bool {
        let Some(content) = event.pointer("/message/content").and_then(Value::as_array) else {
            return false;
        };

        let mut changed = false;
        for block in content {
            match block.get("type").and_then(Value::as_str) {
                Some("text") => {
                    if let Some(text) = block.get("text").and_then(Value::as_str) {
                        changed |= self.append_answer(text);
                    }
                }
                Some("thinking") => {
                    changed |=
                        self.append_tool_once("thinking".to_string(), render_thinking(block));
                }
                Some("tool_use") => {
                    changed |= self.append_tool_once(tool_use_key(block), render_tool_use(block));
                }
                Some("tool_result") => {}
                _ => {}
            }
        }
        changed
    }

    pub fn set_answer_if_empty(&mut self, text: &str) {
        if self.answer.trim().is_empty() {
            let _ = self.append_answer(text);
        }
    }

    pub fn render(&self) -> String {
        if self.output.trim().is_empty() {
            "生成中...".to_string()
        } else {
            self.output.trim().to_string()
        }
    }

    fn append_tool_once(&mut self, key: String, text: String) -> bool {
        if text.trim().is_empty() || !self.seen_tools.insert(key) {
            return false;
        }
        if !self.is_tool_section_started {
            self.output.push_str("**工具调用**\n\n");
            self.is_tool_section_started = true;
        } else {
            self.output.push_str("\n\n");
        }
        self.output.push_str(&text);
        true
    }

    fn append_answer(&mut self, text: &str) -> bool {
        let Some((next_answer, delta)) = stream_delta(&self.answer, text) else {
            return false;
        };
        if !self.is_answer_started {
            if !self.output.trim().is_empty() {
                self.output.push_str("\n\n---\n\n");
            }
            self.is_answer_started = true;
        }
        self.output.push_str(&delta);
        self.answer = next_answer;
        true
    }
}

pub fn result_text(value: &Value) -> Option<String> {
    value
        .get("result")
        .and_then(Value::as_str)
        .map(str::to_string)
}

pub fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    text.chars().take(max.saturating_sub(3)).collect::<String>() + "..."
}

fn stream_delta(current: &str, next: &str) -> Option<(String, String)> {
    if next.is_empty() || current.ends_with(next) {
        None
    } else if let Some(delta) = next.strip_prefix(current) {
        if delta.is_empty() {
            None
        } else {
            Some((next.to_string(), delta.to_string()))
        }
    } else {
        Some((format!("{current}{next}"), next.to_string()))
    }
}

fn tool_use_key(block: &Value) -> String {
    block
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| {
            let name = block.get("name").and_then(Value::as_str).unwrap_or("tool");
            format!(
                "tool_use:{name}:{}",
                compact_json(block.get("input").unwrap_or(&Value::Null))
            )
        })
}

fn render_tool_use(block: &Value) -> String {
    let name = block.get("name").and_then(Value::as_str).unwrap_or("tool");
    let input = block.get("input").unwrap_or(&Value::Null);
    let input = truncate_chars(&compact_json(input), 1200);
    format!("- `{name}`\n```json\n{input}\n```")
}

fn render_thinking(block: &Value) -> String {
    let text = block
        .get("thinking")
        .or_else(|| block.get("text"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let text = truncate_after_chars(text, 50);
    format!("- `Thinking`\n```markdown\n{text}\n```")
}

fn compact_json(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_string())
}

fn truncate_after_chars(text: &str, max: usize) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = normalized.chars();
    let clipped = chars.by_ref().take(max).collect::<String>();
    if chars.next().is_some() {
        format!("{clipped}...")
    } else {
        clipped
    }
}
