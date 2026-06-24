use serde_json::Value;
use std::env;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct ClaudeSession {
    pub id: String,
    pub title: String,
    pub model: Option<String>,
    pub project: String,
    pub cwd: Option<String>,
    pub updated_at: String,
    pub path: PathBuf,
    pub preview: Vec<String>,
    pub search_text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionScope {
    CurrentProject,
    AllProjects,
}

pub fn load_sessions(scope: SessionScope) -> Result<Vec<ClaudeSession>, String> {
    let projects_root = claude_projects_dir();
    let current_dir = env::current_dir().map_err(|err| format!("无法读取当前目录: {err}"))?;
    load_sessions_from(&projects_root, &current_dir, scope)
}

pub fn load_sessions_from(
    projects_root: &Path,
    current_dir: &Path,
    scope: SessionScope,
) -> Result<Vec<ClaudeSession>, String> {
    if !projects_root.exists() {
        return Ok(Vec::new());
    }

    let project_dirs = match scope {
        SessionScope::CurrentProject => vec![projects_root.join(project_dir_name(current_dir))],
        SessionScope::AllProjects => list_project_dirs(projects_root)?,
    };

    let mut sessions = Vec::new();
    for project_dir in project_dirs {
        sessions.extend(read_project_sessions(&project_dir)?);
    }
    sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(sessions)
}

pub fn project_dir_name(path: &Path) -> String {
    path.to_string_lossy().replace('/', "-")
}

fn claude_projects_dir() -> PathBuf {
    let home = env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    PathBuf::from(home).join(".claude").join("projects")
}

fn list_project_dirs(projects_root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut dirs = Vec::new();
    let entries = fs::read_dir(projects_root).map_err(|err| {
        format!(
            "无法读取 Claude 历史目录 {}: {err}",
            projects_root.display()
        )
    })?;

    for entry in entries {
        let entry = entry.map_err(|err| format!("无法读取 Claude 历史项: {err}"))?;
        let file_type = entry
            .file_type()
            .map_err(|err| format!("无法读取 Claude 历史项类型: {err}"))?;
        if file_type.is_dir() {
            dirs.push(entry.path());
        }
    }
    Ok(dirs)
}

fn read_project_sessions(project_dir: &Path) -> Result<Vec<ClaudeSession>, String> {
    if !project_dir.exists() {
        return Ok(Vec::new());
    }

    let mut sessions = Vec::new();
    let entries = fs::read_dir(project_dir)
        .map_err(|err| format!("无法读取 Claude 项目历史 {}: {err}", project_dir.display()))?;

    for entry in entries {
        let entry = entry.map_err(|err| format!("无法读取 Claude 会话项: {err}"))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|err| format!("无法读取 Claude 会话项类型: {err}"))?;
        if file_type.is_file() && path.extension().and_then(|value| value.to_str()) == Some("jsonl")
        {
            if let Some(session) = read_session_file(&path, project_dir)? {
                sessions.push(session);
            }
        }
    }

    Ok(sessions)
}

fn read_session_file(path: &Path, project_dir: &Path) -> Result<Option<ClaudeSession>, String> {
    let file = File::open(path)
        .map_err(|err| format!("无法读取 Claude 会话 {}: {err}", path.display()))?;
    let reader = BufReader::new(file);
    let mut state = SessionDraft::new(project_dir, path);

    for line in reader.lines() {
        let line = line.map_err(|err| format!("读取 Claude 会话失败 {}: {err}", path.display()))?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        state.push_value(&value);
    }

    Ok(state.build())
}

struct SessionDraft {
    id: Option<String>,
    custom_title: Option<String>,
    ai_title: Option<String>,
    model: Option<String>,
    cwd: Option<String>,
    updated_at: Option<String>,
    project: String,
    path: PathBuf,
    messages: Vec<String>,
    has_assistant_text: bool,
}

impl SessionDraft {
    fn new(project_dir: &Path, path: &Path) -> Self {
        let project = project_dir
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("unknown")
            .to_string();
        Self {
            id: None,
            custom_title: None,
            ai_title: None,
            model: None,
            cwd: None,
            updated_at: file_mtime_key(path),
            project,
            path: path.to_path_buf(),
            messages: Vec::new(),
            has_assistant_text: false,
        }
    }

    fn push_value(&mut self, value: &Value) {
        if let Some(session_id) = value.get("sessionId").and_then(Value::as_str) {
            self.id.get_or_insert_with(|| session_id.to_string());
        }
        if let Some(timestamp) = value.get("timestamp").and_then(Value::as_str) {
            self.updated_at = Some(timestamp.to_string());
        }
        if let Some(cwd) = value.get("cwd").and_then(Value::as_str) {
            self.cwd.get_or_insert_with(|| cwd.to_string());
        }
        if let Some(model) = value
            .get("message")
            .and_then(|message| message.get("model"))
            .and_then(Value::as_str)
        {
            self.model = Some(model.to_string());
        }

        match value.get("type").and_then(Value::as_str) {
            Some("custom-title") => {
                if let Some(title) = value.get("customTitle").and_then(Value::as_str) {
                    self.custom_title = Some(clean_title(title));
                }
            }
            Some("ai-title") => {
                if let Some(title) = value.get("aiTitle").and_then(Value::as_str) {
                    self.ai_title = Some(clean_title(title));
                }
            }
            Some("user") => self.push_message(value, false),
            Some("assistant") => self.push_message(value, true),
            _ => {}
        }
    }

    fn push_message(&mut self, value: &Value, is_assistant: bool) {
        let Some(message) = value.get("message") else {
            return;
        };
        let Some(content) = message.get("content") else {
            return;
        };

        for text in extract_text_content(content) {
            let text = clean_block(&text);
            if !text.is_empty() {
                if is_assistant {
                    self.has_assistant_text = true;
                }
                self.messages.push(text);
            }
        }
    }

    fn build(self) -> Option<ClaudeSession> {
        if !self.has_assistant_text {
            return None;
        }
        let id = self.id.or_else(|| file_stem(&self.path))?;
        let title = self
            .custom_title
            .or(self.ai_title)
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| id.clone());
        let updated_at = self.updated_at.unwrap_or_default();
        let preview = self.messages;
        let search_text = preview.join("\n");

        Some(ClaudeSession {
            id,
            title,
            model: self.model,
            project: self.project,
            cwd: self.cwd,
            updated_at,
            path: self.path,
            preview,
            search_text,
        })
    }
}

fn extract_text_content(content: &Value) -> Vec<String> {
    if let Some(text) = content.as_str() {
        return vec![text.to_string()];
    }

    let Some(items) = content.as_array() else {
        return Vec::new();
    };

    items
        .iter()
        .filter_map(|item| match item.get("type").and_then(Value::as_str) {
            Some("text") => item.get("text").and_then(Value::as_str).map(str::to_string),
            _ => None,
        })
        .collect()
}

fn clean_title(text: &str) -> String {
    clean_block(text).chars().take(80).collect()
}

fn clean_block(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn file_stem(path: &Path) -> Option<String> {
    path.file_stem()
        .and_then(|value| value.to_str())
        .map(str::to_string)
}

fn file_mtime_key(path: &Path) -> Option<String> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    Some(system_time_key(modified))
}

fn system_time_key(time: SystemTime) -> String {
    let secs = time
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{secs:020}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn encodes_current_path_like_claude_project_dir() {
        assert_eq!(
            project_dir_name(Path::new("/Users/bole/dev/Clash")),
            "-Users-bole-dev-Clash"
        );
    }

    #[test]
    fn extracts_title_and_visible_history_text() {
        let root = temp_root("extracts_title");
        let current = Path::new("/tmp/clash");
        let project = root.join(project_dir_name(current));
        fs::create_dir_all(&project).unwrap();
        write_jsonl(
            &project.join("session-1.jsonl"),
            &[
                r#"{"type":"mode","sessionId":"session-1"}"#,
                r#"{"type":"user","message":{"role":"user","content":"请实现 resume"},"timestamp":"2026-06-24T01:00:00.000Z","cwd":"/tmp/clash","sessionId":"session-1"}"#,
                r#"{"type":"assistant","message":{"role":"assistant","model":"qwen3.6-plus","content":[{"type":"thinking","thinking":"hidden"},{"type":"text","text":"已经完成搜索高亮"}]},"timestamp":"2026-06-24T01:01:00.000Z","sessionId":"session-1"}"#,
                r#"{"type":"ai-title","aiTitle":"Resume 选择器","sessionId":"session-1"}"#,
                r#"{"type":"custom-title","customTitle":"我的 Resume","sessionId":"session-1"}"#,
            ],
        );

        let sessions = load_sessions_from(&root, current, SessionScope::CurrentProject).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "session-1");
        assert_eq!(sessions[0].title, "我的 Resume");
        assert_eq!(sessions[0].model.as_deref(), Some("qwen3.6-plus"));
        assert!(sessions[0].search_text.contains("请实现 resume"));
        assert!(sessions[0].search_text.contains("已经完成搜索高亮"));
        assert!(!sessions[0].search_text.contains("hidden"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn skips_attachments_snapshots_and_subagents() {
        let root = temp_root("skips_noise");
        let current = Path::new("/tmp/clash");
        let project = root.join(project_dir_name(current));
        fs::create_dir_all(project.join("subagents")).unwrap();
        write_jsonl(
            &project.join("session-1.jsonl"),
            &[
                r#"{"type":"file-history-snapshot","sessionId":"session-1","snapshot":{"x":"resume"}} "#,
                r#"{"type":"attachment","attachment":{"type":"skill_listing","content":"污染搜索"},"sessionId":"session-1"}"#,
                r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"工具输出"},{"type":"text","text":"真正正文"}]},"sessionId":"session-1"}"#,
                r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"助手正文"}]},"sessionId":"session-1"}"#,
            ],
        );
        write_jsonl(
            &project.join("subagents").join("agent.jsonl"),
            &[r#"{"type":"user","message":{"content":"子代理正文"},"sessionId":"agent"}"#],
        );

        let sessions = load_sessions_from(&root, current, SessionScope::CurrentProject).unwrap();
        assert_eq!(sessions.len(), 1);
        assert!(sessions[0].search_text.contains("真正正文"));
        assert!(!sessions[0].search_text.contains("污染搜索"));
        assert!(!sessions[0].search_text.contains("工具输出"));
        assert!(!sessions[0].search_text.contains("子代理正文"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn filters_current_project_or_all_projects() {
        let root = temp_root("scope");
        let current = Path::new("/tmp/clash");
        let other = Path::new("/tmp/other");
        let current_project = root.join(project_dir_name(current));
        let other_project = root.join(project_dir_name(other));
        fs::create_dir_all(&current_project).unwrap();
        fs::create_dir_all(&other_project).unwrap();
        write_jsonl(
            &current_project.join("current.jsonl"),
            &[
                r#"{"type":"user","message":{"content":"当前项目"},"sessionId":"current"}"#,
                r#"{"type":"assistant","message":{"content":"当前助手回复"},"sessionId":"current"}"#,
            ],
        );
        write_jsonl(
            &other_project.join("other.jsonl"),
            &[
                r#"{"type":"user","message":{"content":"其他项目"},"sessionId":"other"}"#,
                r#"{"type":"assistant","message":{"content":"其他助手回复"},"sessionId":"other"}"#,
            ],
        );

        let current_sessions =
            load_sessions_from(&root, current, SessionScope::CurrentProject).unwrap();
        let all_sessions = load_sessions_from(&root, current, SessionScope::AllProjects).unwrap();
        assert_eq!(current_sessions.len(), 1);
        assert_eq!(current_sessions[0].id, "current");
        assert_eq!(all_sessions.len(), 2);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn filters_sessions_without_assistant_text() {
        let root = temp_root("empty_session");
        let current = Path::new("/tmp/clash");
        let project = root.join(project_dir_name(current));
        fs::create_dir_all(&project).unwrap();
        write_jsonl(
            &project.join("empty.jsonl"),
            &[r#"{"type":"user","message":{"content":"只有用户消息"},"sessionId":"empty"}"#],
        );
        write_jsonl(
            &project.join("ok.jsonl"),
            &[
                r#"{"type":"user","message":{"content":"用户消息"},"sessionId":"ok"}"#,
                r#"{"type":"assistant","message":{"content":[{"type":"text","text":"助手回复"}]},"sessionId":"ok"}"#,
            ],
        );

        let sessions = load_sessions_from(&root, current, SessionScope::CurrentProject).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "ok");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn falls_back_to_full_session_id_without_title() {
        let root = temp_root("full_id");
        let current = Path::new("/tmp/clash");
        let project = root.join(project_dir_name(current));
        fs::create_dir_all(&project).unwrap();
        write_jsonl(
            &project.join("2e4ec99f-26c6-4619-a01a-307cdaee2841.jsonl"),
            &[
                r#"{"type":"user","message":{"content":"用户消息"},"sessionId":"2e4ec99f-26c6-4619-a01a-307cdaee2841"}"#,
                r#"{"type":"assistant","message":{"content":"助手回复"},"sessionId":"2e4ec99f-26c6-4619-a01a-307cdaee2841"}"#,
            ],
        );

        let sessions = load_sessions_from(&root, current, SessionScope::CurrentProject).unwrap();
        assert_eq!(sessions[0].title, "2e4ec99f-26c6-4619-a01a-307cdaee2841");
        let _ = fs::remove_dir_all(root);
    }

    fn write_jsonl(path: &Path, lines: &[&str]) {
        let mut file = File::create(path).unwrap();
        for line in lines {
            writeln!(file, "{line}").unwrap();
        }
    }

    fn temp_root(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        env::temp_dir().join(format!("clash-{name}-{nanos}"))
    }
}
