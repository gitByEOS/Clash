use std::env;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct ConfigSlot {
    pub idx: usize,
    pub config: ClashConfig,
}

#[derive(Debug, Clone)]
pub struct ClashConfig {
    pub base_url: String,
    pub auth_token_encrypted: String,
    pub command: String,
    pub models: Vec<String>,
    pub name: Option<String>,
}

#[derive(Debug)]
pub enum ConfigError {
    NotFound,
    ParseError(String),
    IoError(std::io::Error),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::NotFound => write!(f, "未找到配置文件"),
            ConfigError::ParseError(msg) => write!(f, "配置解析错误: {}", msg),
            ConfigError::IoError(e) => write!(f, "IO 错误: {}", e),
        }
    }
}

/// Resolve config file path: $XDG_CONFIG_HOME/clash/auth or $HOME/.config/clash/auth
pub fn config_dir() -> PathBuf {
    env::var("XDG_CONFIG_HOME")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = env::var("HOME").unwrap_or_else(|_| "/root".to_string());
            PathBuf::from(home).join(".config")
        })
        .join("clash")
}

/// Resolve config file path: idx 0 -> auth, idx n -> authn
pub fn config_path_for_idx(idx: usize) -> PathBuf {
    let file_name = if idx == 0 {
        "auth".to_string()
    } else {
        format!("auth{idx}")
    };
    config_dir().join(file_name)
}

/// System prompt file path: ~/.config/clash/system-prompt
pub fn system_prompt_path() -> PathBuf {
    config_dir().join("system-prompt")
}

/// Ensure system-prompt file exists, create if not
pub fn ensure_system_prompt_file() {
    let path = system_prompt_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let existing = fs::read_to_string(&path).unwrap_or_default();
    let managed = crate::prompts::managed_system_prompt();
    let next = merge_managed_system_prompt(&existing, &managed);
    if next != existing {
        let _ = fs::write(&path, next);
    }
}

fn merge_managed_system_prompt(existing: &str, managed: &str) -> String {
    if let Some(tag_start) = existing.find("<clash-system-prompt") {
        if let Some(close_start_rel) = existing[tag_start..].find("</clash-system-prompt>") {
            let close_start = tag_start + close_start_rel;
            let close_end = close_start + "</clash-system-prompt>".len();
            let before = &existing[..tag_start];
            let after = &existing[close_end..];
            return format!("{before}{managed}{after}");
        }
    }
    managed.to_string()
}

/// Read system prompt from file, create default if not exists
pub fn read_system_prompt() -> Option<String> {
    ensure_system_prompt_file();
    let path = system_prompt_path();
    let content = fs::read_to_string(&path).unwrap_or_default();
    let trimmed = content.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Normalize models string: split by comma, trim, filter empty
pub fn normalize_models(input: &str) -> Vec<String> {
    input
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

pub fn read_config_raw_for_idx(idx: usize) -> Result<ClashConfig, ConfigError> {
    let path = config_path_for_idx(idx);
    let content = fs::read_to_string(&path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => ConfigError::NotFound,
        _ => ConfigError::IoError(e),
    })?;

    Ok(parse_config_content(&content))
}

pub fn read_config_for_idx(idx: usize) -> Result<ClashConfig, ConfigError> {
    let cfg = read_config_raw_for_idx(idx)?;
    if cfg.base_url.is_empty() || cfg.auth_token_encrypted.is_empty() {
        return Err(ConfigError::ParseError(
            "缺少 BASE_URL 或 AUTH_TOKEN".to_string(),
        ));
    }

    Ok(cfg)
}

fn parse_config_content(content: &str) -> ClashConfig {
    let mut base_url = String::new();
    let mut auth_token_encrypted = String::new();
    let mut command = String::new();
    let mut models = Vec::new();
    let mut name: Option<String> = None;
    let mut in_models = false;

    for line in content.lines() {
        if line.starts_with('#') {
            continue;
        }

        if in_models {
            if line == "MODELS" {
                in_models = false;
                continue;
            }
            if !line.is_empty() {
                models.push(line.to_string());
            }
            continue;
        }

        if let Some((key, value)) = line.split_once('=') {
            match key {
                "BASE_URL" => base_url = value.to_string(),
                "AUTH_TOKEN" => auth_token_encrypted = value.to_string(),
                "COMMAND" => command = value.to_string(),
                "NAME" => name = Some(value.to_string()).filter(|s| !s.is_empty()),
                "MODELS" if value == "<<MODELS" => {
                    in_models = true;
                }
                _ => {}
            }
        }
    }

    if command.is_empty() {
        command = "clash".to_string();
    }

    ClashConfig {
        base_url,
        auth_token_encrypted,
        command,
        models,
        name,
    }
}

pub fn write_config_for_idx(idx: usize, cfg: &ClashConfig) -> Result<(), ConfigError> {
    let path = config_path_for_idx(idx);

    // Create parent directory
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(ConfigError::IoError)?;
    }

    let now = chrono_like_timestamp();

    let mut content = String::new();
    content.push_str("# Clash 配置文件\n");
    content.push_str(&format!("# 生成时间: {}\n", now));
    content.push_str(&format!("BASE_URL={}\n", cfg.base_url));
    content.push_str(&format!("AUTH_TOKEN={}\n", cfg.auth_token_encrypted));
    if let Some(name) = &cfg.name {
        content.push_str(&format!("NAME={}\n", name));
    }
    content.push_str(&format!("COMMAND={}\n", cfg.command));
    content.push_str("MODELS=<<MODELS\n");
    for model in &cfg.models {
        content.push_str(model);
        content.push('\n');
    }
    content.push_str("MODELS\n");

    fs::write(&path, content).map_err(ConfigError::IoError)?;

    // chmod 600
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&path)
            .map_err(ConfigError::IoError)?
            .permissions();
        perms.set_mode(0o600);
        fs::set_permissions(&path, perms).map_err(ConfigError::IoError)?;
    }

    Ok(())
}

fn parse_config_idx(file_name: &str) -> Option<usize> {
    if file_name == "auth" {
        return Some(0);
    }
    let suffix = file_name.strip_prefix("auth")?;
    if suffix.is_empty() || !suffix.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    suffix.parse::<usize>().ok()
}

pub fn read_config_slots() -> Result<Vec<ConfigSlot>, ConfigError> {
    let dir = config_dir();
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(ConfigError::IoError(e)),
    };

    let mut indices = Vec::new();
    for entry in entries {
        let entry = entry.map_err(ConfigError::IoError)?;
        let Some(file_name) = entry.file_name().to_str().map(|s| s.to_string()) else {
            continue;
        };
        if let Some(idx) = parse_config_idx(&file_name) {
            indices.push(idx);
        }
    }
    indices.sort_unstable();
    indices.dedup();

    let mut slots = Vec::new();
    for idx in indices {
        if let Ok(config) = read_config_for_idx(idx) {
            if !config.models.is_empty() {
                slots.push(ConfigSlot { idx, config });
            }
        }
    }

    Ok(slots)
}

pub fn delete_all_configs() -> Result<(), ConfigError> {
    let dir = config_dir();
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(ConfigError::IoError(e)),
    };

    for entry in entries {
        let entry = entry.map_err(ConfigError::IoError)?;
        let Some(file_name) = entry.file_name().to_str().map(|s| s.to_string()) else {
            continue;
        };
        if parse_config_idx(&file_name).is_some() {
            fs::remove_file(entry.path()).map_err(ConfigError::IoError)?;
        }
    }

    Ok(())
}

#[allow(unreachable_code)]
fn chrono_like_timestamp() -> String {
    #[cfg(unix)]
    {
        let now = std::time::SystemTime::now();
        let secs = now
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as libc::time_t;
        let mut tm = std::mem::MaybeUninit::<libc::tm>::zeroed();
        if unsafe { libc::localtime_r(&secs, tm.as_mut_ptr()) }.is_null() {
            return utc_timestamp();
        }
        let tm = unsafe { tm.assume_init() };
        return format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec,
        );
    }
    utc_timestamp()
}

fn utc_timestamp() -> String {
    let total_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let (hours, minutes, seconds) = {
        let h = total_secs / 3600 % 24;
        let m = total_secs / 60 % 60;
        let s = total_secs % 60;
        (h, m, s)
    };

    // days since 1970-01-01
    let mut days = total_secs / 86400;
    let mut year = 1970u64;

    while days >= 365 {
        let leap = is_leap_year(year);
        days -= if leap { 366 } else { 365 };
        year += 1;
    }

    let leap = is_leap_year(year);
    let month_days: [u64; 12] = if leap {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let (month, day) = month_days
        .iter()
        .enumerate()
        .fold((1, days + 1), |(m, d), (i, md)| {
            if d > *md {
                (i as u64 + 2, d - *md)
            } else {
                (m, d)
            }
        });

    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        year, month, day, hours, minutes, seconds
    )
}

fn is_leap_year(year: u64) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_models() {
        let models = normalize_models("model-a, model-b ,model-c");
        assert_eq!(models, vec!["model-a", "model-b", "model-c"]);
    }

    #[test]
    fn test_normalize_models_empty() {
        let models = normalize_models("");
        assert!(models.is_empty());
    }

    #[test]
    fn test_normalize_models_single() {
        let models = normalize_models("claude-sonnet-4-20250514");
        assert_eq!(models, vec!["claude-sonnet-4-20250514"]);
    }

    #[test]
    fn test_config_path_default() {
        let path = config_path_for_idx(0);
        assert!(path.ends_with("clash/auth"));
    }

    #[test]
    fn merge_managed_system_prompt_overwrites_when_missing() {
        let merged = merge_managed_system_prompt(
            "用户内容",
            "<clash-system-prompt version=\"x\">\n内置\n</clash-system-prompt>\n",
        );
        assert_eq!(
            merged,
            "<clash-system-prompt version=\"x\">\n内置\n</clash-system-prompt>\n"
        );
    }

    #[test]
    fn merge_managed_system_prompt_replaces_tagged_block() {
        let existing = "前\n<clash-system-prompt version=\"old\">\n旧\n</clash-system-prompt>\n后";
        let managed = "<clash-system-prompt version=\"new\">\n新\n</clash-system-prompt>\n";
        let merged = merge_managed_system_prompt(existing, managed);
        assert!(merged.contains("version=\"new\""));
        assert!(!merged.contains("\n旧\n"));
        assert!(merged.starts_with("前\n"));
        assert!(merged.ends_with("后"));
    }
}
