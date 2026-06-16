use std::sync::mpsc::Sender;

pub const DEFAULT_PREFIX: &str = "Clash-";
pub const MANAGER_CHAT_NAME: &str = "Clash-GroupManager";
pub const DEFAULT_POLL_SECS: u64 = 15;
pub const EVENT_RECONNECT_SECS: u64 = 3;
pub const CLAUDE_TURN_TIMEOUT_SECS: u64 = 600;
pub const DEFAULT_CARD_UPDATE_THROTTLE_MS: u64 = 120;

#[derive(Debug, Clone)]
pub struct LarkOptions {
    pub prefix: String,
    pub poll_secs: u64,
    pub once: bool,
}

#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub base_url: String,
    pub auth_token: String,
    pub model: String,
    pub system_prompt_file: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LarkChat {
    pub chat_id: String,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct LarkMessage {
    pub message_id: String,
    pub chat_id: String,
    pub text: String,
}

pub struct ChatWorker {
    pub tx: Sender<LarkMessage>,
}
