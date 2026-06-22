use crate::claude;
use crate::lark::lark_cli::{drain_stderr, log_verbose};
use crate::lark::render::{result_text, ClaudeRenderState};
use crate::lark::types::{AgentConfig, LarkChat, CLAUDE_TURN_TIMEOUT_SECS};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::Duration;

pub struct ClaudeSession {
    child: Child,
    stdin: std::process::ChildStdin,
    rx: Receiver<Value>,
}

impl Drop for ClaudeSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl ClaudeSession {
    pub fn spawn(chat: &LarkChat, agent: &AgentConfig) -> Result<Self, String> {
        let claude_path = claude::find_claude_binary().map_err(|_| "未找到 claude".to_string())?;
        let mut args = vec![
            "--print".to_string(),
            "--permission-mode".to_string(),
            "bypassPermissions".to_string(),
            "--effort".to_string(),
            "max".to_string(),
            "--model".to_string(),
            agent.model.clone(),
            "--input-format".to_string(),
            "stream-json".to_string(),
            "--output-format".to_string(),
            "stream-json".to_string(),
            "--verbose".to_string(),
            "--include-partial-messages".to_string(),
            "--setting-sources".to_string(),
            "user,project,local".to_string(),
            "--name".to_string(),
            chat.name.clone(),
        ];
        if let Some(path) = &agent.system_prompt_file {
            args.push("--append-system-prompt-file".to_string());
            args.push(path.clone());
        }

        let mut command = Command::new(&claude_path);
        command.args(&args);

        let mut child = command
            .env("ANTHROPIC_BASE_URL", &agent.base_url)
            .env("ANTHROPIC_AUTH_TOKEN", &agent.auth_token)
            .env("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1")
            .env("CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS", "1")
            .env("CLAUDE_CODE_ATTRIBUTION_HEADER", "0")
            .env("CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS", "1")
            .env("CLAUDE_CODE_SUBAGENT_MODEL", &agent.model)
            .env("ANTHROPIC_MODEL", &agent.model)
            .env("ANTHROPIC_SMALL_FAST_MODEL", &agent.model)
            .env("ANTHROPIC_DEFAULT_SONNET_MODEL", &agent.model)
            .env("ANTHROPIC_DEFAULT_OPUS_MODEL", &agent.model)
            .env("ANTHROPIC_DEFAULT_HAIKU_MODEL", &agent.model)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("无法启动 claude: {e}"))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "claude stdin 不可用".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "claude stdout 不可用".to_string())?;
        if let Some(stderr) = child.stderr.take() {
            drain_stderr(format!("claude[{}]", chat.name), stderr);
        }

        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines().map_while(Result::ok) {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                match serde_json::from_str::<Value>(trimmed) {
                    Ok(value) => {
                        if tx.send(value).is_err() {
                            break;
                        }
                    }
                    Err(err) => log_verbose(&format!("claude JSON 解析失败：{err}")),
                }
            }
        });

        Ok(Self { child, stdin, rx })
    }

    pub fn ask_stream<F>(&mut self, prompt: &str, mut on_text: F) -> Result<String, String>
    where
        F: FnMut(&str) -> Result<(), String>,
    {
        let input = json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": prompt
            },
            "parent_tool_use_id": null
        });
        writeln!(self.stdin, "{input}").map_err(|e| format!("写入 claude 失败: {e}"))?;
        self.stdin
            .flush()
            .map_err(|e| format!("刷新 claude stdin 失败: {e}"))?;

        let mut state = ClaudeRenderState::new();
        loop {
            let event = match self
                .rx
                .recv_timeout(Duration::from_secs(CLAUDE_TURN_TIMEOUT_SECS))
            {
                Ok(event) => event,
                Err(RecvTimeoutError::Timeout) => {
                    return Err(format!("Claude 超过 {CLAUDE_TURN_TIMEOUT_SECS} 秒未返回"));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err("claude 输出通道已关闭".to_string());
                }
            };
            match event.get("type").and_then(Value::as_str) {
                Some("assistant") | Some("user") => {
                    if state.apply_event(&event) {
                        let rendered = state.render();
                        on_text(&rendered)?;
                    }
                }
                Some("result") => {
                    if let Some(result) = result_text(&event).filter(|s| !s.trim().is_empty()) {
                        state.set_answer_if_empty(&result);
                    }
                    return Ok(state.render());
                }
                _ => {}
            }
        }
    }
}
