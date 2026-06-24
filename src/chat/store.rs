use crate::chat::protocol::{AgentState, ChatMessage, DEFAULT_LEASE_SECS, MAX_MESSAGE_LINE_BYTES};
use crate::config;
use fs2::FileExt;
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub struct ChatStore {
    root: PathBuf,
}

struct RoomLock {
    _file: File,
}

#[derive(Debug)]
pub struct MessageRecord {
    pub end_offset: u64,
    pub message: ChatMessage,
}

impl ChatStore {
    pub fn with_root(root: PathBuf) -> Self {
        Self { root }
    }

    #[cfg(test)]
    fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn append_message(&self, room: &str, message: &ChatMessage) -> Result<(), String> {
        validate_segment(room, "room")?;
        validate_segment(&message.from, "name")?;
        if let Some(to) = &message.to {
            if !to.eq_ignore_ascii_case("all") {
                validate_segment(to, "to")?;
            }
        }

        let dir = self.room_dir(room)?;
        fs::create_dir_all(&dir).map_err(|e| format!("无法创建房间目录: {e}"))?;
        let _lock = self.acquire_room_lock(room)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.messages_path(room)?)
            .map_err(|e| format!("无法打开消息日志: {e}"))?;
        let line = serde_json::to_string(message).map_err(|e| format!("消息序列化失败: {e}"))?;
        if line.len() + 1 > MAX_MESSAGE_LINE_BYTES {
            return Err(format!(
                "消息过长，单行不能超过 {MAX_MESSAGE_LINE_BYTES} 字节"
            ));
        }
        writeln!(file, "{line}").map_err(|e| format!("写入消息失败: {e}"))?;
        file.flush().map_err(|e| format!("刷新消息失败: {e}"))?;
        Ok(())
    }

    pub fn read_records_from(&self, room: &str, offset: u64) -> Result<Vec<MessageRecord>, String> {
        validate_segment(room, "room")?;
        let path = self.messages_path(room)?;
        let mut file = match File::open(&path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(format!("无法读取消息日志: {e}")),
        };

        let len = file
            .metadata()
            .map_err(|e| format!("无法读取消息日志元信息: {e}"))?
            .len();
        let start = offset.min(len);
        file.seek(SeekFrom::Start(start))
            .map_err(|e| format!("无法定位消息日志: {e}"))?;

        let mut reader = BufReader::new(file);
        let mut current = start;
        let mut records = Vec::new();
        while let Some((line, bytes, is_too_long)) = read_jsonl_line(&mut reader)? {
            current += bytes as u64;
            if is_too_long {
                return Err(format!("消息日志单行超过 {MAX_MESSAGE_LINE_BYTES} 字节"));
            }
            let Ok(line) = std::str::from_utf8(&line) else {
                continue;
            };
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(message) = serde_json::from_str::<ChatMessage>(trimmed) {
                records.push(MessageRecord {
                    end_offset: current,
                    message,
                });
            }
        }
        Ok(records)
    }

    pub fn history(&self, room: &str, limit: usize) -> Result<Vec<ChatMessage>, String> {
        let records = self.read_records_from(room, 0)?;
        let mut messages = records
            .into_iter()
            .map(|record| record.message)
            .collect::<Vec<_>>();
        if messages.len() > limit {
            messages = messages.split_off(messages.len() - limit);
        }
        Ok(messages)
    }

    pub fn read_cursor(&self, room: &str, name: &str) -> Result<u64, String> {
        validate_segment(room, "room")?;
        validate_segment(name, "name")?;
        Ok(self
            .read_agent_state(room, name)?
            .map(|state| state.cursor_offset)
            .unwrap_or(0))
    }

    pub fn write_cursor(&self, room: &str, name: &str, offset: u64) -> Result<(), String> {
        validate_segment(room, "room")?;
        validate_segment(name, "name")?;
        let mut state = self
            .read_agent_state(room, name)?
            .unwrap_or_else(|| empty_agent_state(name));
        state.cursor_offset = offset;
        self.write_agent_state(room, name, &state)
    }

    pub fn refresh_lease(
        &self,
        room: &str,
        name: &str,
        status: Option<&str>,
    ) -> Result<AgentState, String> {
        validate_segment(room, "room")?;
        validate_segment(name, "name")?;
        let mut state = self
            .read_agent_state(room, name)?
            .unwrap_or_else(|| empty_agent_state(name));
        let now = unix_secs();
        state.last_active_ts = now;
        state.lease_until_ts = now + DEFAULT_LEASE_SECS;
        state.status = status.map(str::to_string);
        self.write_agent_state(room, name, &state)?;
        Ok(state)
    }

    pub fn read_agent_state(&self, room: &str, name: &str) -> Result<Option<AgentState>, String> {
        validate_segment(room, "room")?;
        validate_segment(name, "name")?;
        self.migrate_agents_if_needed(room)?;
        let path = self.agent_path(room, name)?;
        let content = match fs::read_to_string(path) {
            Ok(content) => content,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("无法读取 Agent 状态: {e}")),
        };
        serde_json::from_str::<AgentState>(&content)
            .map(Some)
            .map_err(|e| format!("Agent 状态无效: {e}"))
    }

    pub fn is_agent_online(&self, room: &str, name: &str) -> Result<bool, String> {
        Ok(self
            .read_agent_state(room, name)?
            .map(|state| state.lease_until_ts >= unix_secs())
            .unwrap_or(false))
    }

    pub fn list_agent_names(&self, room: &str) -> Result<Vec<String>, String> {
        validate_segment(room, "room")?;
        self.migrate_agents_if_needed(room)?;
        let dir = self.agents_dir(room)?;
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(format!("无法读取 Agent 目录: {e}")),
        };
        let mut names = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| format!("无法读取 Agent 文件: {e}"))?;
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            if let Some(name) = path.file_stem().and_then(|value| value.to_str()) {
                names.push(name.to_string());
            }
        }
        names.sort();
        Ok(names)
    }

    fn write_agent_state(&self, room: &str, name: &str, state: &AgentState) -> Result<(), String> {
        let content =
            serde_json::to_string_pretty(state).map_err(|e| format!("Agent 序列化失败: {e}"))?;
        write_atomic(&self.agent_path(room, name)?, &(content + "\n"))
    }

    fn migrate_agents_if_needed(&self, room: &str) -> Result<(), String> {
        let legacy_path = self.legacy_agents_path(room)?;
        if !legacy_path.exists() || !self.is_agents_dir_empty(room)? {
            return Ok(());
        }

        let _lock = self.acquire_room_lock(room)?;
        if !legacy_path.exists() || !self.is_agents_dir_empty(room)? {
            return Ok(());
        }

        let content =
            fs::read_to_string(&legacy_path).map_err(|e| format!("无法读取 agents.json: {e}"))?;
        let agents = serde_json::from_str::<BTreeMap<String, AgentState>>(&content)
            .map_err(|e| format!("agents.json 无效: {e}"))?;
        for (name, state) in agents {
            validate_segment(&name, "name")?;
            self.write_agent_state(room, &name, &state)?;
        }
        fs::remove_file(&legacy_path).map_err(|e| format!("无法删除旧 agents.json: {e}"))?;
        Ok(())
    }

    fn is_agents_dir_empty(&self, room: &str) -> Result<bool, String> {
        let dir = self.agents_dir(room)?;
        let mut entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(true),
            Err(e) => return Err(format!("无法读取 Agent 目录: {e}")),
        };
        match entries.next() {
            Some(Ok(_)) => Ok(false),
            Some(Err(e)) => Err(format!("无法读取 Agent 文件: {e}")),
            None => Ok(true),
        }
    }

    fn acquire_room_lock(&self, room: &str) -> Result<RoomLock, String> {
        let dir = self.room_dir(room)?;
        fs::create_dir_all(&dir).map_err(|e| format!("无法创建房间目录: {e}"))?;
        let path = dir.join(".append.lock");
        let deadline = Instant::now() + Duration::from_secs(5);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|e| format!("无法打开消息写入锁: {e}"))?;
        loop {
            match file.try_lock_exclusive() {
                Ok(()) => return Ok(RoomLock { _file: file }),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return Err("等待消息写入锁超时".to_string());
                    }
                    thread::sleep(Duration::from_millis(20));
                }
                Err(e) => return Err(format!("无法获取消息写入锁: {e}")),
            }
        }
    }

    fn room_dir(&self, room: &str) -> Result<PathBuf, String> {
        validate_segment(room, "room")?;
        Ok(self.root.join(room))
    }

    pub fn messages_path(&self, room: &str) -> Result<PathBuf, String> {
        Ok(self.room_dir(room)?.join("messages.jsonl"))
    }

    pub fn messages_dir(&self, room: &str) -> Result<PathBuf, String> {
        self.room_dir(room)
    }

    fn agents_dir(&self, room: &str) -> Result<PathBuf, String> {
        Ok(self.room_dir(room)?.join("agents"))
    }

    fn agent_path(&self, room: &str, name: &str) -> Result<PathBuf, String> {
        validate_segment(name, "name")?;
        Ok(self.agents_dir(room)?.join(format!("{name}.json")))
    }

    fn legacy_agents_path(&self, room: &str) -> Result<PathBuf, String> {
        Ok(self.room_dir(room)?.join("agents.json"))
    }
}

pub fn default_rooms_root() -> PathBuf {
    config::config_dir().join("rooms")
}

pub fn resolve_rooms_root(path: Option<&str>) -> Result<PathBuf, String> {
    let Some(raw) = path.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(default_rooms_root());
    };
    if let Some(path) = raw.strip_prefix("file://") {
        return Ok(PathBuf::from(path));
    }
    if let Some((scheme, value)) = raw.split_once("://") {
        validate_segment(scheme, "scheme")?;
        let name = value
            .trim_matches('/')
            .replace(['/', '\\', ':'], "-")
            .trim()
            .to_string();
        if name.is_empty() {
            return Err("--path URI 缺少名称".to_string());
        }
        return Ok(config::config_dir()
            .join("rooms")
            .join(format!("{scheme}-{name}")));
    }
    Ok(PathBuf::from(raw))
}

pub fn unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
fn unix_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

fn validate_segment(value: &str, label: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("{label} 不能为空"));
    }
    if value == "."
        || value == ".."
        || value.contains('/')
        || value.contains('\\')
        || value.contains('\0')
    {
        return Err(format!("{label} 不能包含路径分隔符"));
    }
    Ok(())
}

fn empty_agent_state(name: &str) -> AgentState {
    AgentState {
        name: name.to_string(),
        last_active_ts: 0,
        lease_until_ts: 0,
        cursor_offset: 0,
        status: None,
    }
}

fn write_atomic(path: &Path, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("无法创建目录: {e}"))?;
    }
    let tmp = path.with_extension(format!("tmp-{}-{}", std::process::id(), unix_millis()));
    fs::write(&tmp, content).map_err(|e| format!("无法写入临时文件: {e}"))?;
    fs::rename(&tmp, path).map_err(|e| format!("无法替换文件: {e}"))?;
    Ok(())
}

fn read_jsonl_line<R: BufRead>(reader: &mut R) -> Result<Option<(Vec<u8>, usize, bool)>, String> {
    let mut line = Vec::new();
    let mut bytes = 0;
    let mut is_too_long = false;

    loop {
        let buffer = reader
            .fill_buf()
            .map_err(|e| format!("读取消息失败: {e}"))?;
        if buffer.is_empty() {
            return if bytes == 0 {
                Ok(None)
            } else {
                Ok(Some((line, bytes, is_too_long)))
            };
        }

        let newline = buffer.iter().position(|byte| *byte == b'\n');
        let consume = newline.map_or(buffer.len(), |idx| idx + 1);
        if !is_too_long {
            if line.len() + consume <= MAX_MESSAGE_LINE_BYTES {
                line.extend_from_slice(&buffer[..consume]);
            } else {
                line.clear();
                is_too_long = true;
            }
        }
        bytes += consume;
        reader.consume(consume);

        if newline.is_some() {
            return Ok(Some((line, bytes, is_too_long)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::protocol::ChatMessage;

    fn temp_store() -> (ChatStore, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "clash-chat-test-{}-{}",
            std::process::id(),
            unix_nanos()
        ));
        (ChatStore::new(root.clone()), root)
    }

    fn message(id: &str, from: &str, text: &str) -> ChatMessage {
        ChatMessage {
            id: id.to_string(),
            ts: 1,
            from: from.to_string(),
            to: None,
            text: text.to_string(),
            status: None,
        }
    }

    #[test]
    fn appends_jsonl_and_skips_bad_lines() {
        let (store, root) = temp_store();
        store
            .append_message("r1", &message("1", "A", "@B hi"))
            .unwrap();
        let path = root.join("r1").join("messages.jsonl");
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(file, "bad json").unwrap();
        store
            .append_message("r1", &message("2", "B", "ok"))
            .unwrap();

        let records = store.read_records_from("r1", 0).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].message.id, "1");
        assert_eq!(records[1].message.id, "2");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_oversized_message_line() {
        let (store, root) = temp_store();
        let err = store
            .append_message(
                "r1",
                &message("1", "A", &"x".repeat(MAX_MESSAGE_LINE_BYTES)),
            )
            .unwrap_err();
        assert!(err.contains("消息过长"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_oversized_jsonl_line() {
        let (store, root) = temp_store();
        let dir = root.join("r1");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("messages.jsonl");
        let mut file = File::create(&path).unwrap();
        writeln!(file, "{}", "x".repeat(MAX_MESSAGE_LINE_BYTES + 1)).unwrap();
        writeln!(
            file,
            "{}",
            serde_json::to_string(&message("1", "A", "ok")).unwrap()
        )
        .unwrap();

        let err = store.read_records_from("r1", 0).unwrap_err();
        assert!(err.contains("消息日志单行超过"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cursor_skips_read_messages() {
        let (store, root) = temp_store();
        store
            .append_message("r1", &message("1", "A", "@B hi"))
            .unwrap();
        let first = store.read_records_from("r1", 0).unwrap();
        store.write_cursor("r1", "B", first[0].end_offset).unwrap();
        store
            .append_message("r1", &message("2", "A", "@B again"))
            .unwrap();
        let records = store
            .read_records_from("r1", store.read_cursor("r1", "B").unwrap())
            .unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].message.id, "2");
        let agents_json =
            fs::read_to_string(root.join("r1").join("agents").join("B.json")).unwrap();
        assert!(agents_json.contains("\"cursor_offset\""));
        assert!(!root.join("r1").join("agents.json").exists());
        assert!(!root.join("r1").join("cursors").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn lease_marks_agent_online() {
        let (store, root) = temp_store();
        store.refresh_lease("r1", "A", Some("idle")).unwrap();
        assert!(store.is_agent_online("r1", "A").unwrap());
        let state = store.read_agent_state("r1", "A").unwrap().unwrap();
        assert!(state.lease_until_ts >= state.last_active_ts);
        assert_eq!(state.status.as_deref(), Some("idle"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn expired_lease_marks_agent_offline() {
        let (store, root) = temp_store();
        let state = AgentState {
            name: "A".to_string(),
            last_active_ts: 1,
            lease_until_ts: 1,
            cursor_offset: 0,
            status: None,
        };
        store.write_agent_state("r1", "A", &state).unwrap();
        assert!(!store.is_agent_online("r1", "A").unwrap());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn migrates_legacy_agents_json_to_agent_files() {
        let (store, root) = temp_store();
        let dir = root.join("r1");
        fs::create_dir_all(&dir).unwrap();
        let state = AgentState {
            name: "A".to_string(),
            last_active_ts: 1,
            lease_until_ts: 2,
            cursor_offset: 3,
            status: Some("idle".to_string()),
        };
        let mut agents = BTreeMap::new();
        agents.insert("A".to_string(), state.clone());
        fs::write(
            dir.join("agents.json"),
            serde_json::to_string_pretty(&agents).unwrap(),
        )
        .unwrap();

        assert_eq!(store.read_agent_state("r1", "A").unwrap(), Some(state));
        assert!(root.join("r1").join("agents").join("A.json").exists());
        assert!(!root.join("r1").join("agents.json").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn list_agent_names_reads_agent_files() {
        let (store, root) = temp_store();
        store.refresh_lease("r1", "B", None).unwrap();
        store.refresh_lease("r1", "A", None).unwrap();

        assert_eq!(
            store.list_agent_names("r1").unwrap(),
            vec!["A".to_string(), "B".to_string()]
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn resolves_path_namespaces() {
        assert_eq!(
            resolve_rooms_root(Some("/tmp/clash-rooms")).unwrap(),
            PathBuf::from("/tmp/clash-rooms")
        );
        assert_eq!(
            resolve_rooms_root(Some("file:///tmp/clash-rooms")).unwrap(),
            PathBuf::from("/tmp/clash-rooms")
        );
        assert!(resolve_rooms_root(Some("share://team/a"))
            .unwrap()
            .ends_with("rooms/share-team-a"));
        assert!(resolve_rooms_root(Some("smb://team:a"))
            .unwrap()
            .ends_with("rooms/smb-team-a"));
    }
}
