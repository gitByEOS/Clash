use crate::cli::{print_cyan, print_green, print_red, print_yellow};
use crate::config;
use crate::tui;
use std::env;
use std::process;

const UPDATE_CHECK_INTERVAL_SECS: i64 = 3 * 24 * 60 * 60; // 3 days

fn app_config_path() -> std::path::PathBuf {
    config::config_dir().join("app_config")
}

fn read_app_config() -> (Option<i64>, Option<String>) {
    let path = app_config_path();
    if !path.exists() {
        return (None, None);
    }

    let content = std::fs::read_to_string(&path).unwrap_or_default();
    let mut last_ts: Option<i64> = None;
    let mut ignored: Option<String> = None;

    for line in content.lines() {
        if let Some((key, value)) = line.split_once('=') {
            match key.trim() {
                "last_check_ts" => last_ts = value.trim().parse().ok(),
                "ignored_version" => ignored = Some(value.trim().to_string()),
                _ => {}
            }
        }
    }

    (last_ts, ignored)
}

fn write_app_config(last_ts: i64, ignored: Option<&str>) {
    let path = app_config_path();
    let content = format!(
        "last_check_ts={}\nignored_version={}\n",
        last_ts,
        ignored.unwrap_or("")
    );
    let _ = std::fs::write(&path, content);
}

fn get_current_ts() -> i64 {
    let output = process::Command::new("date")
        .arg("+%s")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "0".to_string());
    output.parse().unwrap_or(0)
}

fn check_claude_outdated() -> Option<String> {
    let output = process::Command::new("npm")
        .args(["-g", "outdated", "@anthropic-ai/claude-code"])
        .output();

    match output {
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            for line in stdout.lines().skip(1) {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 3 && parts[0] == "@anthropic-ai/claude-code" {
                    return Some(parts[2].to_string());
                }
            }
            None
        }
        Err(_) => None,
    }
}

fn get_claude_version() -> Option<String> {
    let output = process::Command::new("claude")
        .arg("--version")
        .output();

    match output {
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            // 输出格式: "2.1.166 (Claude Code)" 或 "v2.1.166"
            for line in stdout.lines() {
                // 尝试匹配 v 开头的版本
                if let Some(v) = line.split_whitespace()
                    .find(|s| s.starts_with('v'))
                {
                    return Some(v[1..].to_string()); // 去掉 v
                }
                // 尝试匹配纯数字版本 (如 "2.1.166")
                if let Some(v) = line.split_whitespace()
                    .find(|s| s.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false))
                {
                    return Some(v.to_string());
                }
            }
            None
        }
        Err(_) => None,
    }
}

fn prompt_update_choice(current: &str, latest: &str) -> Option<bool> {
    // Some(true) = 更新, Some(false) = 忽略, None = 取消
    print_cyan(&format!("Claude Code 有新版本: {} -> {}", current, latest));

    let options = vec![
        format!("更新到 {}", latest),
        "忽略此版本".to_string(),
    ];

    let title = "Claude Code 更新";
    let selected = tui::select_item(&options, title);

    match selected {
        Some(s) if s.starts_with("更新") => Some(true),
        Some(s) if s == "忽略此版本" => Some(false),
        _ => None,
    }
}

fn run_npm_update() -> bool {
    print_cyan("正在更新 Claude Code...");
    let status = process::Command::new("npm")
        .args(["-g", "update", "@anthropic-ai/claude-code", "--silent"])
        .status();

    match status {
        Ok(s) if s.success() => {
            print_green("Claude Code 更新成功");
            true
        }
        Ok(_) => {
            print_red("更新失败，请手动执行: npm -g update @anthropic-ai/claude-code");
            false
        }
        Err(_) => {
            print_red("无法执行 npm update");
            false
        }
    }
}

pub fn maybe_check_update() {
    let current_ts = get_current_ts();
    let (last_ts, ignored_version) = read_app_config();

    let need_check = match last_ts {
        None => true,
        Some(last) => current_ts - last >= UPDATE_CHECK_INTERVAL_SECS,
    };

    if !need_check {
        return;
    }

    write_app_config(current_ts, ignored_version.as_deref());

    let latest = check_claude_outdated();
    if latest.is_none() {
        return;
    }

    let latest = latest.unwrap();

    if ignored_version.as_deref() == Some(latest.as_str()) {
        print_yellow(&format!("Claude Code {} 已被忽略，跳过更新检查", latest));
        return;
    }

    let current = get_claude_version().unwrap_or_else(|| "未知".to_string());

    match prompt_update_choice(&current, &latest) {
        Some(true) => {
            if run_npm_update() {
                write_app_config(current_ts, None);
            }
        }
        Some(false) => {
            print_yellow(&format!("已忽略 Claude Code {}", latest));
            write_app_config(current_ts, Some(&latest));
        }
        None => {
            // 用户取消，不做任何操作
        }
    }
}

fn run_npm_install() -> Result<(), ()> {
    print_cyan("未找到 claude，正在安装 @anthropic-ai/claude-code@latest ...");

    let status = if cfg!(windows) {
        process::Command::new("cmd")
            .args(["/C", "npm install -g @anthropic-ai/claude-code@latest --silent"])
            .status()
    } else {
        process::Command::new("sh")
            .arg("-c")
            .arg("npm install -g @anthropic-ai/claude-code@latest --silent")
            .status()
    };

    match status {
        Ok(s) if s.success() => {
            print_green("claude 安装成功");
            Ok(())
        }
        Ok(_) => {
            print_red("npm install 失败，请手动执行: npm install -g @anthropic-ai/claude-code@latest");
            Err(())
        }
        Err(_) => {
            print_red("未找到 npm，请先安装 Node.js");
            Err(())
        }
    }
}

pub fn find_claude_binary() -> Result<String, ()> {
    let path_sep = if cfg!(windows) { ';' } else { ':' };

    if let Ok(path_env) = env::var("PATH") {
        for dir in path_env.split(path_sep) {
            let candidate = if cfg!(windows) {
                format!("{}\\claude.cmd", dir)
            } else {
                format!("{}/claude", dir)
            };
            if std::path::Path::new(&candidate).exists() {
                return Ok(candidate);
            }
        }
    }

    run_npm_install()?;

    // 重新查找
    if let Ok(path_env) = env::var("PATH") {
        for dir in path_env.split(path_sep) {
            let candidate = if cfg!(windows) {
                format!("{}\\claude.cmd", dir)
            } else {
                format!("{}/claude", dir)
            };
            if std::path::Path::new(&candidate).exists() {
                return Ok(candidate);
            }
        }
    }

    print_red("安装成功但仍未找到 claude，请确认 npm 全局路径在 PATH 中");
    Err(())
}