use std::env;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct ClashConfig {
    pub base_url: String,
    pub auth_token_encrypted: String,
    pub command: String,
    pub models: Vec<String>,
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
pub fn config_path() -> PathBuf {
    let config_dir = env::var("XDG_CONFIG_HOME")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = env::var("HOME").unwrap_or_else(|_| "/root".to_string());
            PathBuf::from(home).join(".config")
        });
    config_dir.join("clash").join("auth")
}

/// Normalize models string: split by comma, trim, filter empty
pub fn normalize_models(input: &str) -> Vec<String> {
    input
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// 读取配置文件，允许字段不完整
pub fn read_config_raw() -> Result<ClashConfig, ConfigError> {
    let path = config_path();
    let content = fs::read_to_string(&path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => ConfigError::NotFound,
        _ => ConfigError::IoError(e),
    })?;

    Ok(parse_config_content(&content))
}

/// Parse config file
pub fn read_config() -> Result<ClashConfig, ConfigError> {
    let cfg = read_config_raw()?;
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
                "MODELS" => {
                    if value == "<<MODELS" {
                        in_models = true;
                    }
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
    }
}

/// Write config file
pub fn write_config(cfg: &ClashConfig) -> Result<(), ConfigError> {
    let path = config_path();

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

/// 删除配置文件
pub fn delete_config() -> Result<(), ConfigError> {
    let path = config_path();
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(ConfigError::IoError(e)),
    }
}

#[allow(unreachable_code)]
fn chrono_like_timestamp() -> String {
    #[cfg(unix)]
    {
        let now = std::time::SystemTime::now();
        let secs = now.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as libc::time_t;
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
            if d > *md { (i as u64 + 2, d - *md) } else { (m, d) }
        });

    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", year, month, day, hours, minutes, seconds)
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
        let path = config_path();
        assert!(path.ends_with("clash/auth"));
    }
}
