use std::sync::mpsc::Sender;

pub const DEFAULT_PREFIX: &str = "Clash-";
pub const MANAGER_CHAT_NAME: &str = "Clash-GroupManager";
pub const DEFAULT_POLL_SECS: u64 = 15;
pub const EVENT_RECONNECT_SECS: u64 = 3;

#[derive(Debug, Clone)]
pub struct LarkOptions {
    pub prefix: String,
    pub poll_secs: u64,
    pub once: bool,
    /// 模型序号（1-based），对齐 clash 交互列表显示的数字
    pub model_index: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub base_url: String,
    pub auth_token: String,
    pub model: String,
    pub effort: String,
    pub max_ctx: Option<u64>,
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
