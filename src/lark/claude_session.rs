use crate::claude;
use crate::lark::lark_cli::{drain_stderr, log_verbose};
use crate::lark::types::{AgentConfig, LarkChat};
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
    interrupt_seq: u64,
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
            agent.effort.clone(),
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
        claude::apply_claude_env(
            &mut command,
            &claude::ClaudeEnvSpec {
                base_url: &agent.base_url,
                auth_token: &agent.auth_token,
                model: &agent.model,
                max_ctx: agent.max_ctx,
            },
        );

        let mut child = command
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

        Ok(Self {
            child,
            stdin,
            rx,
            interrupt_seq: 0,
        })
    }

    pub fn send_prompt(&mut self, prompt: &str) -> Result<(), String> {
        let input = json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": prompt
            },
            "parent_tool_use_id": null
        });
        self.write_line(&input)
    }

    /// 中断当前回合，不拆掉会话进程
    pub fn interrupt(&mut self) -> Result<(), String> {
        self.interrupt_seq += 1;
        let input = json!({
            "type": "control_request",
            "request_id": format!("stop-{}", self.interrupt_seq),
            "request": { "subtype": "interrupt" }
        });
        self.write_line(&input)
    }

    pub fn recv_event(&mut self, timeout: Duration) -> Result<Option<Value>, String> {
        match self.rx.recv_timeout(timeout) {
            Ok(event) => Ok(Some(event)),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err("claude 输出通道已关闭".to_string()),
        }
    }

    pub fn try_recv_event(&mut self) -> Result<Option<Value>, String> {
        match self.rx.try_recv() {
            Ok(event) => Ok(Some(event)),
            Err(mpsc::TryRecvError::Empty) => Ok(None),
            Err(mpsc::TryRecvError::Disconnected) => Err("claude 输出通道已关闭".to_string()),
        }
    }

    fn write_line(&mut self, value: &Value) -> Result<(), String> {
        writeln!(self.stdin, "{value}").map_err(|e| format!("写入 claude 失败: {e}"))?;
        self.stdin
            .flush()
            .map_err(|e| format!("刷新 claude stdin 失败: {e}"))
    }
}
