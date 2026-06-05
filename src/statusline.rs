use serde::Deserialize;
use std::env;
use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;

// ── Statusline JSON input structure ─────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(crate) struct StatuslineInput {
    model: Option<StatuslineModel>,
    context_window: Option<StatuslineContextWindow>,
    cost: Option<StatuslineCost>,
}

#[derive(Debug, Deserialize)]
struct StatuslineModel {
    display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StatuslineContextWindow {
    context_window_size: Option<u64>,
    current_usage: Option<StatuslineUsage>,
}

#[derive(Debug, Deserialize)]
struct StatuslineUsage {
    input_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
    cache_read_input_tokens: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct StatuslineCost {
    total_duration_ms: Option<u64>,
}

// ── statusline config ───────────────────────────────────────────────────

pub(crate) fn claude_settings_path() -> PathBuf {
    env::var("CLAUDE_CONFIG_DIR")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = env::var("HOME").unwrap_or_else(|_| "/root".to_string());
            PathBuf::from(home).join(".claude")
        })
        .join("settings.json")
}

fn has_valid_statusline_config() -> bool {
    let path = claude_settings_path();
    if !path.exists() {
        return false;
    }
    let content = fs::read_to_string(&path).unwrap_or_default();
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
        if let Some(obj) = json.as_object() {
            if let Some(sl) = obj.get("statusLine") {
                if let Some(sl_obj) = sl.as_object() {
                    let has_type = sl_obj.get("type").and_then(|v| v.as_str()).is_some();
                    let has_command = sl_obj.get("command").and_then(|v| v.as_str()).is_some();
                    return has_type && has_command;
                }
                return false;
            }
        }
    }
    false
}

pub(crate) fn ensure_statusline_config() {
    if has_valid_statusline_config() {
        return;
    }

    let path = claude_settings_path();
    let sl_config = serde_json::json!({
        "statusLine": {
            "type": "command",
            "command": "clash statusline"
        }
    });

    if path.exists() {
        let existing = fs::read_to_string(&path).unwrap_or_default();
        if let Ok(mut json) = serde_json::from_str::<serde_json::Value>(&existing) {
            if let Some(obj) = json.as_object_mut() {
                if let Some(sl_obj) = sl_config.as_object() {
                    for (k, v) in sl_obj {
                        obj.insert(k.clone(), v.clone());
                    }
                }
            }
            let merged = serde_json::to_string_pretty(&json).unwrap_or_default();
            fs::write(&path, merged).ok();
        }
    } else {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).ok();
        }
        let content = serde_json::to_string_pretty(&sl_config).unwrap_or_default();
        fs::write(&path, content).ok();
    }
}

// ── formatting helpers ──────────────────────────────────────────────────

fn format_context_size(size: u64) -> String {
    if size >= 1_000_000 {
        format!("{}m", size / 1_000_000)
    } else if size >= 1_000 {
        format!("{}k", size / 1_000)
    } else {
        size.to_string()
    }
}

fn color_for_pct(pct: u64) -> &'static str {
    if pct >= 90 {
        "\x1b[1;31m" // red
    } else if pct >= 70 {
        "\x1b[1;33m" // yellow
    } else if pct >= 50 {
        "\x1b[1;35m" // magenta
    } else {
        "\x1b[1;32m" // green
    }
}

fn build_bar(pct: u64, width: usize) -> String {
    let filled = ((pct as usize).min(100) * width) / 100;
    let empty = width - filled;
    let bar_color = color_for_pct(pct);
    let dim = "\x1b[2m";
    let reset = "\x1b[0m";
    format!(
        "{}{}{}{}{}",
        bar_color,
        "█".repeat(filled),
        dim,
        "░".repeat(empty),
        reset
    )
}

fn remove_size_marker(name: &str) -> String {
    let mut result = name.to_string();
    let mut i = 0;
    let chars = name.chars().collect::<Vec<_>>();
    while i < chars.len() {
        if chars[i] == '[' {
            let mut j = i + 1;
            let mut is_size = true;
            while j < chars.len() && chars[j] != ']' {
                let c = chars[j];
                if !(c.is_ascii_digit() || c == 'k' || c == 'm') {
                    is_size = false;
                }
                j += 1;
            }
            if j < chars.len() && chars[j] == ']' && is_size {
                let marker = &name[i..=j];
                result = result.replace(marker, "");
            }
        }
        i += 1;
    }
    result
}

fn format_duration(secs: i64) -> String {
    if secs < 0 {
        return "0s".to_string();
    }
    if secs >= 3600 {
        format!("{}h{}m", secs / 3600, (secs % 3600) / 60)
    } else if secs >= 60 {
        format!("{}m", secs / 60)
    } else {
        format!("{}s", secs)
    }
}

// ── main statusline output ──────────────────────────────────────────────

pub(crate) fn do_statusline() {
    let input: String = {
        let mut buf = String::new();
        io::stdin().read_to_string(&mut buf).unwrap_or_default();
        buf
    };

    if input.is_empty() {
        println!("\x1b[1;38;2;255;176;85mClash\x1b[0m");
        return;
    }

    let parsed: StatuslineInput = serde_json::from_str(&input).unwrap_or(StatuslineInput {
        model: None,
        context_window: None,
        cost: None,
    });

    let model_name_raw = parsed
        .model
        .and_then(|m| m.display_name)
        .unwrap_or_else(|| "Clash".to_string());

    let model_name = remove_size_marker(&model_name_raw);

    let ctx = parsed.context_window.unwrap_or(StatuslineContextWindow {
        context_window_size: Some(200_000),
        current_usage: None,
    });

    let size = ctx.context_window_size.unwrap_or(200_000);
    let usage = ctx.current_usage.unwrap_or(StatuslineUsage {
        input_tokens: Some(0),
        cache_creation_input_tokens: Some(0),
        cache_read_input_tokens: Some(0),
    });

    let current = usage.input_tokens.unwrap_or(0)
        + usage.cache_creation_input_tokens.unwrap_or(0)
        + usage.cache_read_input_tokens.unwrap_or(0);

    let pct = current
        .saturating_mul(100)
        .checked_div(size)
        .unwrap_or(0)
        .min(100);

    let session_duration = parsed
        .cost
        .and_then(|c| c.total_duration_ms)
        .map(|ms| format_duration(ms as i64 / 1000));

    let blue = "\x1b[1;36m";
    let orange = "\x1b[1;38;2;255;176;85m";
    let dim = "\x1b[2m";
    let reset = "\x1b[0m";

    let bar = build_bar(pct, 10);
    let pct_color = color_for_pct(pct);
    let size_str = format_context_size(size);

    let duration_str = session_duration
        .map(|d| format!(" {}⏱ {}{}", dim, reset, d))
        .unwrap_or_default();

    println!(
        "{}[{}]{} {} {}{}%{} - {}{} \x1b[90m|{} {}Clash{}{}",
        blue, model_name, reset,
        bar,
        pct_color, pct, reset,
        blue, size_str, reset,
        orange, reset,
        duration_str
    );
}