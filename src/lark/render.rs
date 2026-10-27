use serde_json::Value;
use std::collections::VecDeque;

/// 进度区的一条活动，按出现顺序渲染
enum Activity {
    Thinking(String),
    Tool { key: String, text: String },
}

enum SnapshotGrow {
    Unchanged,
    Replace(String),
    NewRound,
}

/// 本轮折叠结果：思考变长是 Patch，新工具/新一轮思考是 Commit，答案增长也 PATCH 同一张卡
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fold {
    Unchanged,
    Patch,
    Commit,
    Answer,
}

impl Fold {
    pub fn merge(self, other: Self) -> Self {
        use Fold::{Answer, Commit, Patch, Unchanged};
        match (self, other) {
            (Commit, _) | (_, Commit) => Commit,
            (Patch, _) | (_, Patch) => Patch,
            (Answer, _) | (_, Answer) => Answer,
            (Unchanged, Unchanged) => Unchanged,
        }
    }

    pub fn breaks_pump(self) -> bool {
        matches!(self, Fold::Commit)
    }
}

/// 已提交的飞书动作；PatchProgress 在队列里最多一条
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboundOp {
    PatchProgress,
    CloseProgress,
}

pub fn queue_fold(ops: &mut VecDeque<OutboundOp>, fold: Fold) {
    if matches!(fold, Fold::Patch | Fold::Commit | Fold::Answer)
        && !ops.iter().any(|op| *op == OutboundOp::PatchProgress)
    {
        ops.push_front(OutboundOp::PatchProgress);
    }
}

pub fn queue_result(ops: &mut VecDeque<OutboundOp>) {
    ops.push_back(OutboundOp::CloseProgress);
}

pub fn queue_stop(ops: &mut VecDeque<OutboundOp>) {
    ops.retain(|op| *op == OutboundOp::PatchProgress);
    ops.push_back(OutboundOp::CloseProgress);
}

struct ToolLine {
    names: &'static [&'static str],
    verb: &'static str,
    keys: &'static [&'static str],
    fallback: &'static str,
}

const TOOL_LINES: &[ToolLine] = &[
    ToolLine {
        names: &["Read"],
        verb: "读取",
        keys: &["file_path", "path", "notebook_path"],
        fallback: "文件",
    },
    ToolLine {
        names: &["Write"],
        verb: "写入",
        keys: &["file_path", "path"],
        fallback: "文件",
    },
    ToolLine {
        names: &["Edit", "MultiEdit", "NotebookEdit"],
        verb: "修改",
        keys: &["file_path", "path", "notebook_path"],
        fallback: "文件",
    },
    ToolLine {
        names: &["Glob", "Grep"],
        verb: "检索",
        keys: &["pattern"],
        fallback: "检索",
    },
    ToolLine {
        names: &["WebFetch", "WebSearch"],
        verb: "访问",
        keys: &["url", "query"],
        fallback: "网络请求",
    },
    ToolLine {
        names: &["Task"],
        verb: "委派",
        keys: &["description"],
        fallback: "子任务",
    },
];

pub struct ClaudeRenderState {
    activities: Vec<Activity>,
    answer: String,
}

impl ClaudeRenderState {
    pub fn new() -> Self {
        Self {
            activities: Vec::new(),
            answer: String::new(),
        }
    }

    pub fn apply_event(&mut self, event: &Value) -> Fold {
        // 工具元数据消息（isMeta=true）不写入卡片，避免撑爆内容上限
        if event.get("isMeta").and_then(Value::as_bool) == Some(true) {
            return Fold::Unchanged;
        }
        let inner = if event.get("type").and_then(Value::as_str) == Some("stream_event") {
            event.get("event").unwrap_or(event)
        } else {
            event
        };
        match inner.get("type").and_then(Value::as_str) {
            Some("content_block_delta") => self.apply_content_delta(inner),
            Some("content_block_start") => self.apply_content_start(inner),
            _ => self.apply_message_blocks(inner),
        }
    }

    fn apply_message_blocks(&mut self, event: &Value) -> Fold {
        let Some(content) = event.pointer("/message/content").and_then(Value::as_array) else {
            return Fold::Unchanged;
        };

        let mut fold = Fold::Unchanged;
        for block in content {
            let next = match block.get("type").and_then(Value::as_str) {
                Some("text") => block
                    .get("text")
                    .and_then(Value::as_str)
                    .map(|text| self.append_answer(text))
                    .unwrap_or(Fold::Unchanged),
                Some("thinking") => self.append_thinking(thinking_text(block)),
                Some("tool_use") => self.apply_tool_block(block),
                _ => Fold::Unchanged,
            };
            fold = fold.merge(next);
        }
        fold
    }

    fn apply_content_delta(&mut self, event: &Value) -> Fold {
        let Some(delta) = event.get("delta") else {
            return Fold::Unchanged;
        };
        match delta.get("type").and_then(Value::as_str) {
            Some("thinking_delta") => {
                let text = delta
                    .get("thinking")
                    .or_else(|| delta.get("text"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                self.append_thinking_delta(text)
            }
            Some("text_delta") => {
                let text = delta.get("text").and_then(Value::as_str).unwrap_or("");
                self.append_answer(text)
            }
            _ => Fold::Unchanged,
        }
    }

    fn apply_content_start(&mut self, event: &Value) -> Fold {
        let Some(block) = event.get("content_block") else {
            return Fold::Unchanged;
        };
        match block.get("type").and_then(Value::as_str) {
            Some("thinking") => self.append_thinking(thinking_text(block)),
            Some("tool_use") => self.apply_tool_block(block),
            Some("text") => block
                .get("text")
                .and_then(Value::as_str)
                .map(|text| self.append_answer(text))
                .unwrap_or(Fold::Unchanged),
            _ => Fold::Unchanged,
        }
    }

    pub fn set_answer_if_empty(&mut self, text: &str) {
        if self.answer.trim().is_empty() {
            let _ = self.append_answer(text);
        }
    }

    pub fn render(&self) -> String {
        let mut out = String::new();
        if !self.activities.is_empty() {
            out.push_str("**进度**\n");
            out.push_str(&self.render_progress());
        }
        if !self.answer.trim().is_empty() {
            if !out.is_empty() {
                out.push_str("\n\n---\n\n");
            }
            out.push_str(self.answer.trim());
        }
        if out.trim().is_empty() {
            "- 正在处理…".to_string()
        } else {
            out
        }
    }

    pub fn render_stream(&self) -> String {
        if self.activities.is_empty() {
            "- 正在处理…".to_string()
        } else {
            format!("**进度**\n{}", self.render_progress())
        }
    }

    pub fn has_answer(&self) -> bool {
        !self.answer.trim().is_empty()
    }

    pub fn patch_body(&self) -> String {
        if self.has_answer() {
            self.render()
        } else {
            self.render_stream()
        }
    }

    fn render_progress(&self) -> String {
        self.activities
            .iter()
            .map(|activity| match activity {
                Activity::Thinking(text) => format!("- 💭 {text}"),
                Activity::Tool { text, .. } => text.clone(),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// 同轮思考就地增长，新一轮思考另起一行，旧快照回放忽略
    fn append_thinking(&mut self, text: &str) -> Fold {
        let text = text.trim();
        if text.is_empty() {
            return Fold::Unchanged;
        }
        if let Some(Activity::Thinking(last)) = self.activities.last_mut() {
            match grow_thinking_snapshot(last, text) {
                SnapshotGrow::Unchanged => return Fold::Unchanged,
                SnapshotGrow::Replace(next) => {
                    *last = next;
                    return Fold::Patch;
                }
                SnapshotGrow::NewRound => {}
            }
        }
        self.activities.push(Activity::Thinking(text.to_string()));
        Fold::Commit
    }

    fn append_thinking_delta(&mut self, delta: &str) -> Fold {
        if delta.is_empty() {
            return Fold::Unchanged;
        }
        if let Some(Activity::Thinking(last)) = self.activities.last_mut() {
            last.push_str(delta);
            return Fold::Patch;
        }
        let text = delta.trim();
        if text.is_empty() {
            return Fold::Unchanged;
        }
        self.activities.push(Activity::Thinking(text.to_string()));
        Fold::Patch
    }

    fn apply_tool_block(&mut self, block: &Value) -> Fold {
        if !tool_has_render_input(block) {
            return Fold::Unchanged;
        }
        self.append_tool(tool_use_key(block), render_tool_use(block))
    }

    fn append_tool(&mut self, key: String, text: String) -> Fold {
        if text.trim().is_empty() {
            return Fold::Unchanged;
        }
        if let Some(existing) = self
            .activities
            .iter_mut()
            .find_map(|activity| match activity {
                Activity::Tool {
                    key: existing_key,
                    text,
                } if *existing_key == key => Some(text),
                _ => None,
            })
        {
            if text == *existing {
                return Fold::Unchanged;
            }
            if text.starts_with(existing.as_str()) {
                *existing = text;
                return Fold::Patch;
            }
            return Fold::Unchanged;
        }
        self.activities.push(Activity::Tool { key, text });
        Fold::Commit
    }

    fn append_answer(&mut self, text: &str) -> Fold {
        let Some((next_answer, _)) = stream_delta(&self.answer, text) else {
            return Fold::Unchanged;
        };
        self.answer = next_answer;
        Fold::Answer
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

/// 思考只认前缀快照增长与回滚，不把无关两段拼成一行
fn grow_thinking_snapshot(current: &str, next: &str) -> SnapshotGrow {
    if current.starts_with(next) {
        SnapshotGrow::Unchanged
    } else if next.starts_with(current) {
        SnapshotGrow::Replace(next.to_string())
    } else {
        SnapshotGrow::NewRound
    }
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

fn tool_has_render_input(block: &Value) -> bool {
    let name = block.get("name").and_then(Value::as_str).unwrap_or("");
    let input = block.get("input").unwrap_or(&Value::Null);
    if name == "Bash" {
        return has_nonempty_str(input, &["command"]);
    }
    if let Some(line) = TOOL_LINES.iter().find(|line| line.names.contains(&name)) {
        return has_nonempty_str(input, line.keys);
    }
    true
}

fn has_nonempty_str(input: &Value, keys: &[&str]) -> bool {
    keys.iter().any(|key| {
        input
            .get(*key)
            .and_then(Value::as_str)
            .is_some_and(|value| !value.is_empty())
    })
}

/// 工具调用语义化为单行进度，避免把原始 input JSON 倾倒进卡片
fn render_tool_use(block: &Value) -> String {
    let name = block.get("name").and_then(Value::as_str).unwrap_or("工具");
    let input = block.get("input").unwrap_or(&Value::Null);
    if name == "Bash" {
        let command = pick_field(input, &["command"], "执行命令");
        return format!("- {}", markdown_code(&format!("$ {command}")));
    }
    if let Some(line) = TOOL_LINES.iter().find(|line| line.names.contains(&name)) {
        let value = pick_field(input, line.keys, line.fallback);
        return format!("- {} {}", line.verb, markdown_code(value));
    }
    format!("- {}", markdown_code(name))
}

fn pick_field<'a>(input: &'a Value, keys: &[&str], fallback: &'a str) -> &'a str {
    keys.iter()
        .find_map(|key| input.get(*key).and_then(Value::as_str))
        .unwrap_or(fallback)
}

/// 用足够长的反引号包裹，内容含反引号时不会截断
fn markdown_code(text: &str) -> String {
    let longest_run = longest_backtick_run(text);
    let delimiter = "`".repeat(longest_run + 1);
    format!("{delimiter}{text}{delimiter}")
}

fn longest_backtick_run(text: &str) -> usize {
    let mut longest = 0;
    let mut current = 0;
    for ch in text.chars() {
        if ch == '`' {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 0;
        }
    }
    longest
}

fn thinking_text(block: &Value) -> &str {
    block
        .get("thinking")
        .or_else(|| block.get("text"))
        .and_then(Value::as_str)
        .unwrap_or("")
}

fn compact_json(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_string())
}

/// 最新主会话请求的用量，缺失字段与显式零值分开保存
#[derive(Debug, Default, Clone, Copy)]
pub struct TokenUsage {
    input: Option<u64>,
    cache_creation: Option<u64>,
    cache_read: Option<u64>,
    output: Option<u64>,
}

impl TokenUsage {
    pub fn observe(&mut self, event: &Value) {
        if event
            .get("parent_tool_use_id")
            .is_some_and(|parent| !parent.is_null())
        {
            return;
        }
        // result 是回合累计消耗，不能覆盖最新请求的上下文占用
        let (usage, is_snapshot) = match event.get("type").and_then(Value::as_str) {
            Some("assistant") => (event.pointer("/message/usage"), true),
            Some("stream_event") => match event.pointer("/event/type").and_then(Value::as_str) {
                Some("message_start") => (event.pointer("/event/message/usage"), true),
                Some("message_delta") => (event.pointer("/event/usage"), false),
                _ => return,
            },
            _ => return,
        };
        if is_snapshot {
            self.reset();
        }
        let Some(usage) = usage.filter(|value| value.is_object()) else {
            return;
        };
        for (field, key) in [
            (&mut self.input, "input_tokens"),
            (&mut self.cache_creation, "cache_creation_input_tokens"),
            (&mut self.cache_read, "cache_read_input_tokens"),
            (&mut self.output, "output_tokens"),
        ] {
            if let Some(value) = usage.get(key).and_then(Value::as_u64) {
                *field = Some(value);
            }
        }
    }

    pub fn total(&self) -> Option<u64> {
        [self.input, self.cache_creation, self.cache_read, self.output]
            .into_iter()
            .flatten()
            .reduce(u64::saturating_add)
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

pub const DEFAULT_CONTEXT_TOKENS: u64 = 200_000;

pub fn format_elapsed(elapsed: std::time::Duration) -> String {
    let secs = elapsed.as_secs();
    let hours = secs / 3600;
    let mins = (secs % 3600) / 60;
    let remain = secs % 60;
    if hours > 0 {
        format!("{hours}h{mins}m")
    } else if mins > 0 {
        format!("{mins}m")
    } else {
        format!("{remain}s")
    }
}

pub fn format_context_percent(used: Option<u64>, max_ctx: Option<u64>) -> String {
    match (used, max_ctx) {
        (Some(used), Some(max_ctx)) if max_ctx > 0 => {
            format!("{:.2}%", (used as f64) * 100.0 / max_ctx as f64)
        }
        _ => "--".to_string(),
    }
}

pub fn format_session_footer(model: &str, ctx: &str, elapsed: &str, dir: &str) -> String {
    format!("model: {model} · ctx: {ctx} · ⏱ {elapsed} · dir: {dir}")
}
