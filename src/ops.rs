use crate::api_test;
use crate::claude;
use crate::claude_history::{self, SessionScope};
use crate::cli::{print_cyan, print_green, print_red, print_yellow, ConfigSetArgs};
use crate::config::{self, ClashConfig, ConfigSlot};
use crate::crypto;
use crate::hooks;
use crate::model::{context_size_marker, remove_size_marker};
use crate::prompt_capture;
use crate::statusline;
use crate::tui;
use std::env;
use std::fs::{self, File};
use std::io::Write;
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{self, Child, Stdio};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Clone)]
pub struct RunModelChoice {
    pub label: String,
    pub model: String,
    pub config: ClashConfig,
}

pub fn account_label(slot: &ConfigSlot) -> String {
    if let Some(name) = &slot.config.name {
        name.clone()
    } else {
        format!("{}st", slot.idx + 1)
    }
}

// ── version / update ────────────────────────────────────────

fn raw_base_url(default: &str) -> String {
    env::var("CLASH_INSTALL_BASE_URL").unwrap_or_else(|_| default.to_string())
}

fn fetch_text(url: &str) -> Result<String, String> {
    let output = process::Command::new("curl")
        .arg("-fsSL")
        .arg(url)
        .output()
        .map_err(|e| format!("无法执行 curl: {e}"))?;

    if !output.status.success() {
        return Err(format!("下载失败: {url}"));
    }

    String::from_utf8(output.stdout).map_err(|_| "远端内容不是 UTF-8".to_string())
}

fn latest_version_from_cargo_toml(content: &str) -> Option<String> {
    content.lines().find_map(|line| {
        let line = line.trim();
        let (_, value) = line.split_once('=')?;
        if line.starts_with("version") {
            Some(format!("v{}", value.trim().trim_matches('"')))
        } else {
            None
        }
    })
}

pub fn do_version(app_version: &str) {
    println!("{}", app_version);
}

pub fn do_update(
    app_version: &str,
    default_base_url: &str,
    print_red_fn: fn(&str),
    print_green_fn: fn(&str),
    print_cyan_fn: fn(&str),
) -> Result<(), ()> {
    let base_url = raw_base_url(default_base_url);
    let cargo_toml_url = format!("{base_url}/Cargo.toml");
    let cargo_toml = fetch_text(&cargo_toml_url).map_err(|err| {
        print_red_fn(&err);
    })?;
    let latest = latest_version_from_cargo_toml(&cargo_toml).ok_or_else(|| {
        print_red_fn("无法从 Cargo.toml 读取最新版本");
    })?;

    if latest == app_version {
        print_green_fn(&format!("已是最新版本: {}", app_version));
        return Ok(());
    }

    print_cyan_fn(&format!("发现新版本: {} -> {}", app_version, latest));
    let install_url = format!("{base_url}/install.sh");
    let status = process::Command::new("bash")
        .arg("-c")
        .arg(format!(
            "curl -fsSL '{}' | bash",
            install_url.replace('\'', "'\\''")
        ))
        .status()
        .map_err(|e| {
            print_red_fn(&format!("无法执行安装脚本: {e}"));
        })?;

    if status.success() {
        Ok(())
    } else {
        print_red_fn("更新失败");
        Err(())
    }
}

// ── config ─────────────────────────────────────────────────

fn save_config(
    idx: usize,
    base_url: String,
    auth_token: String,
    models: Vec<String>,
    name: Option<String>,
) -> Result<(), ()> {
    let cfg = ClashConfig {
        base_url,
        auth_token_encrypted: crypto::encrypt_token(&auth_token).map_err(|_| ())?,
        command: "clash".to_string(),
        models,
        name,
    };

    config::write_config_for_idx(idx, &cfg).map_err(|_| ())?;
    let config_path = config::config_path_for_idx(idx);
    print_green(&format!("配置已保存到 {}", config_path.display()));
    print_green("API Key 已加密存储");
    auto_test_after_config(idx)
}

fn do_configure_interactive_for_idx(idx: usize) -> Result<(), ()> {
    print_cyan("Clash 配置向导（以 DeepSeek 为例）");

    let mut buf = String::new();
    print!("API 地址 (如 https://api.deepseek.com/anthropic)\n> ");
    std::io::stdout().flush().unwrap();
    std::io::stdin().read_line(&mut buf).unwrap();
    let base_url = buf.trim().to_string();
    if base_url.is_empty() {
        print_red("地址不能为空");
        return Err(());
    }

    buf.clear();
    print!("API Key (如 sk-c9cbf*******cd7a)\n> ");
    std::io::stdout().flush().unwrap();
    std::io::stdin().read_line(&mut buf).unwrap();
    let auth_token = buf.trim().to_string();
    if auth_token.is_empty() {
        print_red("Key 不能为空");
        return Err(());
    }

    let mut model_list = Vec::new();
    while model_list.is_empty() {
        buf.clear();
        print!("模型列表 (如 deepseek-v4-pro[1m], deepseek-v4-flash)\n> ");
        std::io::stdout().flush().unwrap();
        std::io::stdin().read_line(&mut buf).unwrap();
        model_list = config::normalize_models(buf.trim());
        if model_list.is_empty() {
            print_red("模型列表不能为空");
        }
    }

    save_config(idx, base_url, auth_token, model_list, None)
}

fn load_config_for_update(idx: usize) -> Result<ClashConfig, ()> {
    match config::read_config_raw_for_idx(idx) {
        Ok(cfg) => Ok(cfg),
        Err(config::ConfigError::NotFound) => Ok(ClashConfig {
            base_url: String::new(),
            auth_token_encrypted: String::new(),
            command: "clash".to_string(),
            models: vec![],
            name: None,
        }),
        Err(_) => Err(()),
    }
}

pub fn do_config(
    args: &[String],
    _print_red: fn(&str),
    _print_green: fn(&str),
    _print_yellow: fn(&str),
    _print_cyan: fn(&str),
    parse_fn: fn(&[String]) -> Result<ConfigSetArgs, ()>,
) -> Result<(), ()> {
    statusline::ensure_statusline_config();
    config::ensure_system_prompt_file();

    if args.is_empty() {
        return do_config_show(0);
    }

    let parsed = parse_fn(args)?;
    if parsed.base_url.is_none() && parsed.auth_key.is_none() && parsed.models.is_none() {
        return match do_config_show(parsed.idx) {
            Ok(()) => Ok(()),
            Err(()) => do_configure_interactive_for_idx(parsed.idx),
        };
    }

    let mut cfg = load_config_for_update(parsed.idx)?;

    if let Some(base_url) = parsed.base_url {
        cfg.base_url = base_url;
    }
    if let Some(auth_key) = parsed.auth_key {
        cfg.auth_token_encrypted = crypto::encrypt_token(&auth_key).map_err(|_| ())?;
    }
    if let Some(models_raw) = parsed.models {
        let models = config::normalize_models(&models_raw);
        if models.is_empty() {
            print_red("模型列表不能为空");
            return Err(());
        }
        cfg.models = models;
    }

    config::write_config_for_idx(parsed.idx, &cfg).map_err(|_| ())?;
    let config_path = config::config_path_for_idx(parsed.idx);
    print_green(&format!("配置已保存到 {}", config_path.display()));
    if !cfg.auth_token_encrypted.is_empty() {
        print_green("API Key 已加密存储");
    }
    auto_test_after_config(parsed.idx)
}

fn do_config_show(idx: usize) -> Result<(), ()> {
    let cfg = config::read_config_raw_for_idx(idx).map_err(|_| {
        print_yellow("未配置，请运行 clash 进行初始化");
    })?;

    if cfg.base_url.is_empty() && cfg.auth_token_encrypted.is_empty() && cfg.models.is_empty() {
        print_yellow("未配置，请运行 clash 进行初始化");
        return Err(());
    }

    print_cyan(&format!("=== 当前配置 idx={} ===", idx));
    if cfg.base_url.is_empty() {
        println!("BASE_URL=");
    } else {
        println!("BASE_URL={}", cfg.base_url);
    }

    if cfg.auth_token_encrypted.is_empty() {
        println!("AUTH_TOKEN=");
    } else {
        let decrypted = crypto::decrypt_token(&cfg.auth_token_encrypted).unwrap_or_default();
        if decrypted.len() >= 10 {
            let prefix = &decrypted[..5];
            let suffix = &decrypted[decrypted.len() - 5..];
            println!("AUTH_TOKEN={}****{} (AES-256 加密存储)", prefix, suffix);
        } else {
            println!("AUTH_TOKEN=**** (AES-256 加密存储)");
        }
    }

    println!("COMMAND={}", cfg.command);
    println!("MODELS=<<MODELS");
    for model in &cfg.models {
        println!("{}", model);
    }
    println!("MODELS");
    Ok(())
}

pub fn do_reset(_print_red: fn(&str), _print_green: fn(&str)) -> Result<(), ()> {
    config::delete_all_configs().map_err(|_| ())?;
    print_green(&format!(
        "已删除全部配置 {}",
        config::config_dir().display()
    ));
    Ok(())
}

// ── hooks ───────────────────────────────────────────────────

pub fn do_hooks() -> Result<(), ()> {
    hooks::do_hooks()
}

// ── prompts ───────────────────────────────────────────────────

pub fn do_prompts(args: &[String], _print_red: fn(&str), _print_green: fn(&str)) -> Result<(), ()> {
    let output = prompt_capture::parse_prompt_output(args).map_err(|msg| {
        print_red(&msg);
    })?;
    let capture = prompt_capture::capture_claude_prompt(print_red)?;
    match output {
        prompt_capture::PromptOutput::HtmlOpen => {
            let path = prompt_capture::write_html_report(&capture).map_err(|msg| {
                print_red(&msg);
            })?;
            prompt_capture::open_html_report(&path).map_err(|msg| {
                print_red(&msg);
            })?;
            print_green(&format!("HTML 报告已打开: {}", path.display()));
        }
        _ => prompt_capture::print_capture(&capture, output),
    }
    Ok(())
}

// ── debug ───────────────────────────────────────────────────

const DEBUG_DEFAULT_HOST: &str = "localhost";
const DEBUG_DEFAULT_PORT: u16 = 11435;
const MOCK_OLLAMA_CHECK_INTERVAL_SECS: u64 = 7 * 24 * 60 * 60;

struct DebugArgs {
    idx: Option<usize>,
    model: Option<String>,
    host: String,
    port: u16,
    claude_args: Vec<String>,
}

fn debug_usage() -> &'static str {
    "用法: clash debug [--idx <编号>] [--model <模型>] [--host <地址>] [--port <端口>] [-- <claude 参数>]"
}

fn parse_debug_args(args: &[String]) -> Result<DebugArgs, String> {
    let mut idx = None;
    let mut model = None;
    let mut host = DEBUG_DEFAULT_HOST.to_string();
    let mut port = DEBUG_DEFAULT_PORT;
    let mut claude_args = Vec::new();
    let mut i = 0;

    while i < args.len() {
        let arg = &args[i];
        if arg == "--" {
            claude_args.extend(args[i + 1..].iter().cloned());
            break;
        }
        let value = args
            .get(i + 1)
            .ok_or_else(|| format!("{arg} 缺少值"))?
            .clone();
        match arg.as_str() {
            "--idx" => {
                idx = Some(
                    value
                        .parse::<usize>()
                        .map_err(|_| "--idx 必须是 0 或正整数".to_string())?,
                )
            }
            "--model" => model = Some(value),
            "--host" => host = value,
            "--port" => {
                port = value
                    .parse::<u16>()
                    .map_err(|_| "--port 必须是 1-65535".to_string())?
            }
            _ => return Err(debug_usage().to_string()),
        }
        i += 2;
    }

    Ok(DebugArgs {
        idx,
        model,
        host,
        port,
        claude_args,
    })
}

// ── rename ───────────────────────────────────────────────────

pub fn do_rename(
    _print_red: fn(&str),
    _print_green: fn(&str),
    _print_yellow: fn(&str),
    _print_cyan: fn(&str),
) -> Result<(), ()> {
    let slots = config::read_config_slots().map_err(|_| ())?;
    if slots.is_empty() {
        print_yellow("未找到任何配置账户");
        return Err(());
    }

    let labels: Vec<String> = slots
        .iter()
        .map(|slot| {
            let current_name = account_label(slot);
            let models_count = slot.config.models.len();
            format!("{}  ({} 个模型)", current_name, models_count)
        })
        .collect();

    let selected_label = tui::select_item(&labels, "选择账户").ok_or(())?;

    let slot = slots
        .iter()
        .find(|slot| {
            let current_name = account_label(slot);
            let models_count = slot.config.models.len();
            format!("{}  ({} 个模型)", current_name, models_count) == selected_label
        })
        .ok_or(())?;

    let current_name = account_label(slot);
    print_cyan(&format!("当前名称: {}", current_name));

    let mut buf = String::new();
    print!("输入新名称: ");
    std::io::stdout().flush().unwrap();
    std::io::stdin().read_line(&mut buf).unwrap();
    let new_name = buf.trim().to_string();
    let name_opt = if new_name.is_empty() {
        None
    } else {
        Some(new_name)
    };

    let mut cfg = slot.config.clone();
    cfg.name = name_opt;

    config::write_config_for_idx(slot.idx, &cfg).map_err(|_| ())?;

    let new_label = cfg
        .name
        .clone()
        .unwrap_or_else(|| format!("{}st", slot.idx + 1));
    print_green(&format!("账户已重命名为: {}", new_label));

    Ok(())
}

// ── test ───────────────────────────────────────────────────

fn should_skip_auto_test() -> bool {
    matches!(
        env::var("CLASH_SKIP_AUTO_TEST").as_deref(),
        Ok("1") | Ok("true") | Ok("yes") | Ok("TRUE") | Ok("YES")
    )
}

fn flush_stdout() {
    let _ = std::io::Write::flush(&mut std::io::stdout());
}

fn print_probe_item(item: &api_test::ModelProbeResult) {
    if item.ok {
        print_green(&format!("  {} 通过", item.model));
    } else {
        print_red(&format!(
            "  {} 失败: {}",
            item.model,
            item.detail.as_deref().unwrap_or("未知错误")
        ));
    }
    flush_stdout();
}

/// 逐个模型做连通测试，每完成一个立即输出
fn run_model_probes(ctx: &api_test::TestContext) -> bool {
    let mut failed = 0usize;
    for model in &ctx.models {
        print_cyan(&format!("  连通测试 {model} ..."));
        flush_stdout();
        let item = api_test::probe_one(ctx, model);
        print_probe_item(&item);
        if !item.ok {
            failed += 1;
        }
    }

    if failed > 0 {
        print_red(&format!(
            "{}/{} 个模型连通测试失败",
            failed,
            ctx.models.len()
        ));
        flush_stdout();
        return false;
    }

    print_green(&format!("全部通过（{} 个模型）", ctx.models.len()));
    flush_stdout();
    true
}

/// 配置写入后自动做连通测试；不完整或 CLASH_SKIP_AUTO_TEST=1 时跳过
fn auto_test_after_config(idx: usize) -> Result<(), ()> {
    if should_skip_auto_test() {
        return Ok(());
    }

    let cfg = match config::read_config_raw_for_idx(idx) {
        Ok(c) => c,
        Err(_) => return Ok(()),
    };
    if cfg.base_url.is_empty() || cfg.auth_token_encrypted.is_empty() || cfg.models.is_empty() {
        print_yellow("配置不完整，跳过连通性测试");
        return Ok(());
    }

    print_cyan("正在进行 Anthropic 兼容 API 连通测试（curl POST /v1/messages）...");
    let opts = api_test::TestOptions {
        idx: Some(idx),
        base_url: None,
        auth_key: None,
        model: None,
    };

    let ctx = api_test::prepare_for_idx(idx, &opts).map_err(|err| {
        print_red(&err);
    })?;
    if run_model_probes(&ctx) {
        Ok(())
    } else {
        Err(())
    }
}

pub fn do_test(
    args: &[String],
    _print_red: fn(&str),
    _print_green: fn(&str),
    _print_yellow: fn(&str),
    _print_cyan: fn(&str),
) -> Result<(), ()> {
    statusline::ensure_statusline_config();

    let opts = api_test::parse_test_args(args).map_err(|_| {
        print_red("用法: clash test [--idx <编号>] [--url <地址>] [--key <Key>] [--model <模型>]");
    })?;

    let slots = config::read_config_slots().map_err(|_| ())?;
    if slots.is_empty() {
        print_yellow("未找到任何配置账户");
        return Err(());
    }

    let test_indices: Vec<usize> = match opts.idx {
        Some(idx) => {
            if slots.iter().any(|s| s.idx == idx) {
                vec![idx]
            } else {
                print_red(&format!("账户 idx={} 不存在", idx));
                return Err(());
            }
        }
        None => slots.iter().map(|s| s.idx).collect(),
    };

    let mut total_failed = 0usize;
    let mut total_passed = 0usize;

    for &idx in &test_indices {
        print_cyan(&format!(
            "=== 测试账户 [{}] ===",
            slots
                .iter()
                .find(|s| s.idx == idx)
                .map(account_label)
                .unwrap_or_else(|| format!("{}st", idx + 1))
        ));
        flush_stdout();

        let ctx = api_test::prepare_for_idx(idx, &opts).map_err(|err| {
            print_red(&err);
        })?;

        let failed = !run_model_probes(&ctx);
        if failed {
            total_failed += ctx.models.len();
        } else {
            total_passed += ctx.models.len();
        }
    }

    if test_indices.len() > 1 {
        print_cyan(&format!(
            "=== 总结: {} 通过, {} 失败 ===",
            total_passed, total_failed
        ));
    }

    if total_failed > 0 {
        Err(())
    } else {
        Ok(())
    }
}

// ── select and run ─────────────────────────────────────────────────

fn collect_run_choices() -> Result<Vec<RunModelChoice>, ()> {
    let slots = config::read_config_slots().map_err(|_| ())?;
    let is_multi_account = slots.len() > 1;
    let mut choices = Vec::new();

    for slot in slots {
        for model in &slot.config.models {
            let label = if is_multi_account {
                format!("[{}]  {}", account_label(&slot), model)
            } else {
                model.clone()
            };
            choices.push(RunModelChoice {
                label,
                model: model.clone(),
                config: slot.config.clone(),
            });
        }
    }

    Ok(choices)
}

fn load_run_choices() -> Result<Vec<RunModelChoice>, ()> {
    let choices = collect_run_choices()?;
    if !choices.is_empty() {
        return Ok(choices);
    }

    print_yellow("未找到配置，请先配置厂商地址和 API Key");
    do_configure_interactive_for_idx(0)?;

    let choices = collect_run_choices()?;
    if choices.is_empty() {
        print_red("配置不完整，请重新配置");
        do_configure_interactive_for_idx(0)?;
        return collect_run_choices();
    }

    Ok(choices)
}

fn select_choice_from_list(choices: Vec<RunModelChoice>) -> Result<RunModelChoice, ()> {
    let labels = choices
        .iter()
        .map(|choice| choice.label.clone())
        .collect::<Vec<_>>();
    let selected_label = tui::select_model(&labels).ok_or(())?;
    choices
        .into_iter()
        .find(|choice| choice.label == selected_label)
        .ok_or(())
}

fn select_run_choice_with_model_hint(model_hint: Option<&str>) -> Result<RunModelChoice, ()> {
    let choices = load_run_choices()?;
    let Some(model_hint) = model_hint else {
        return select_choice_from_list(choices);
    };

    let normalized_model = remove_size_marker(model_hint);
    let matches = choices
        .iter()
        .filter(|choice| remove_size_marker(&choice.model) == normalized_model)
        .cloned()
        .collect::<Vec<_>>();

    match matches.len() {
        1 => Ok(matches.into_iter().next().unwrap()),
        _ => select_choice_from_list(choices),
    }
}

fn select_debug_choice(args: &DebugArgs) -> Result<RunModelChoice, ()> {
    if let Some(idx) = args.idx {
        let config = config::read_config_for_idx(idx).map_err(|err| {
            print_red(&format!("读取账户 idx={idx} 失败: {err}"));
        })?;
        if config.models.is_empty() {
            print_red(&format!("账户 idx={idx} 未配置模型"));
            return Err(());
        }

        let model = match &args.model {
            Some(model) => model.clone(),
            None if config.models.len() == 1 => config.models[0].clone(),
            None => tui::select_model(&config.models).ok_or(())?,
        };
        let label = format!("[{}]  {}", idx, model);
        return Ok(RunModelChoice {
            label,
            model,
            config,
        });
    }

    let choices = load_run_choices()?;
    if let Some(model) = &args.model {
        let normalized_model = remove_size_marker(model);
        let matches = choices
            .into_iter()
            .filter(|choice| remove_size_marker(&choice.model) == normalized_model)
            .collect::<Vec<_>>();
        return match matches.len() {
            0 => {
                print_red(&format!("未找到模型: {model}"));
                Err(())
            }
            1 => Ok(matches.into_iter().next().unwrap()),
            _ => select_choice_from_list(matches),
        };
    }

    select_choice_from_list(choices)
}

fn set_claude_env(base_url: &str, auth_token: &str, model: &str, max_ctx: Option<u64>) {
    env::set_var("ANTHROPIC_BASE_URL", base_url);
    env::set_var("ANTHROPIC_AUTH_TOKEN", auth_token);
    env::set_var("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1");
    env::set_var("CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS", "1");
    env::set_var("CLAUDE_CODE_ATTRIBUTION_HEADER", "0");
    env::set_var("CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS", "1");
    env::set_var("CLAUDE_CODE_ALWAYS_ENABLE_EFFORT", "1");
    env::set_var("ENABLE_TOOL_SEARCH", "false");
    env::set_var("CLAUDE_CODE_SUBAGENT_MODEL", model);
    env::set_var("ANTHROPIC_MODEL", model);
    env::set_var("ANTHROPIC_SMALL_FAST_MODEL", model);
    env::set_var("ANTHROPIC_DEFAULT_SONNET_MODEL", model);
    env::set_var("ANTHROPIC_DEFAULT_OPUS_MODEL", model);
    env::set_var("ANTHROPIC_DEFAULT_HAIKU_MODEL", model);
    if let Some(max_ctx) = max_ctx {
        env::set_var("CLAUDE_CODE_MAX_CONTEXT_TOKENS", max_ctx.to_string());
    } else {
        env::remove_var("CLAUDE_CODE_MAX_CONTEXT_TOKENS");
    }
}

fn claude_args(model: &str, extra_args: &[String]) -> Vec<String> {
    let system_prompt = config::read_system_prompt();
    let mut cmd_args: Vec<String> = vec![
        "--permission-mode".to_string(),
        "bypassPermissions".to_string(),
        "--effort".to_string(),
        "max".to_string(),
        "--model".to_string(),
        model.to_string(),
    ];

    if system_prompt.is_some() {
        cmd_args.push("--append-system-prompt-file".to_string());
        cmd_args.push(config::system_prompt_path().to_string_lossy().to_string());
    }

    cmd_args.extend(extra_args.iter().cloned());
    cmd_args
}

fn debug_dir() -> PathBuf {
    config::config_dir().join("debug")
}

fn debug_log_path() -> PathBuf {
    debug_dir().join("latest.log")
}

fn debug_app_config_path() -> PathBuf {
    debug_dir().join("app_config")
}

fn unix_now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn read_mock_ollama_last_check_ts() -> Option<u64> {
    let content = fs::read_to_string(debug_app_config_path()).ok()?;
    content.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        if key.trim() == "mock_ollama_last_check_ts" {
            value.trim().parse::<u64>().ok()
        } else {
            None
        }
    })
}

fn write_mock_ollama_last_check_ts(ts: u64) {
    let dir = debug_dir();
    let _ = fs::create_dir_all(&dir);
    let content = format!("mock_ollama_last_check_ts={ts}\n");
    let _ = fs::write(debug_app_config_path(), content);
}

fn is_mock_ollama_installed() -> bool {
    process::Command::new("mock-ollama")
        .arg("-h")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

fn install_mock_ollama() -> Result<(), ()> {
    print_cyan("正在安装 mock-ollama@latest ...");
    let status = process::Command::new("npm")
        .args(["install", "-g", "mock-ollama@latest"])
        .status()
        .map_err(|err| {
            print_red(&format!("无法执行 npm: {err}"));
        })?;
    if status.success() {
        print_green("mock-ollama 安装完成");
        Ok(())
    } else {
        print_red("mock-ollama 安装失败，请手动执行: npm install -g mock-ollama@latest");
        Err(())
    }
}

fn maybe_install_mock_ollama() -> Result<(), ()> {
    if is_mock_ollama_installed() {
        return Ok(());
    }
    install_mock_ollama()
}

fn maybe_update_mock_ollama() {
    let now = unix_now_secs();
    if read_mock_ollama_last_check_ts()
        .map(|last| now.saturating_sub(last) < MOCK_OLLAMA_CHECK_INTERVAL_SECS)
        .unwrap_or(false)
    {
        return;
    }
    write_mock_ollama_last_check_ts(now);

    let output = match process::Command::new("npm")
        .args(["-g", "outdated", "mock-ollama", "--json"])
        .output()
    {
        Ok(output) => output,
        Err(err) => {
            print_yellow(&format!("mock-ollama 更新检查失败: {err}"));
            return;
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() && stdout.trim().is_empty() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr.trim();
        if !detail.is_empty() {
            print_yellow(&format!("mock-ollama 更新检查失败: {detail}"));
        }
        return;
    }
    if stdout.trim().is_empty() || !stdout.contains("mock-ollama") {
        return;
    }
    if install_mock_ollama().is_err() {
        print_yellow("继续使用当前 mock-ollama");
    }
}

fn prepare_debug_log_file() -> Result<File, ()> {
    let dir = debug_dir();
    fs::create_dir_all(&dir).map_err(|err| {
        print_red(&format!("无法创建 debug 目录: {err}"));
    })?;
    File::create(debug_log_path()).map_err(|err| {
        print_red(&format!("无法创建 debug 日志: {err}"));
    })
}

fn spawn_mock_ollama(args: &DebugArgs, base_url: &str, auth_token: &str) -> Result<Child, ()> {
    let log_file = prepare_debug_log_file()?;
    let stderr = log_file.try_clone().map_err(|err| {
        print_red(&format!("无法复制 debug 日志句柄: {err}"));
    })?;

    process::Command::new("mock-ollama")
        .arg("--url")
        .arg(base_url)
        .arg("--apikey")
        .arg(auth_token)
        .arg("--host")
        .arg(&args.host)
        .arg("--port")
        .arg(args.port.to_string())
        .arg("--api-style")
        .arg("anthropic")
        .arg("--open")
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(stderr))
        .spawn()
        .map_err(|err| {
            print_red(&format!("无法启动 mock-ollama: {err}"));
        })
}

fn wait_for_mock_ollama(child: &mut Child, host: &str, port: u16) -> Result<(), ()> {
    let addr = format!("{host}:{port}");
    for _ in 0..100 {
        if let Ok(Some(status)) = child.try_wait() {
            print_red(&format!("mock-ollama 已退出: {status}"));
            print_yellow(&format!("查看日志: {}", debug_log_path().display()));
            return Err(());
        }
        if TcpStream::connect(&addr).is_ok() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }
    print_red(&format!("mock-ollama 未在 10 秒内监听 {addr}"));
    print_yellow(&format!("查看日志: {}", debug_log_path().display()));
    Err(())
}

fn stop_mock_ollama(child: &mut Child) {
    if matches!(child.try_wait(), Ok(Some(_))) {
        return;
    }
    let _ = child.kill();
    let _ = child.wait();
}

pub fn do_debug(
    args: &[String],
    _print_red: fn(&str),
    _print_green: fn(&str),
    _print_yellow: fn(&str),
    _print_cyan: fn(&str),
) -> Result<(), ()> {
    statusline::ensure_statusline_config();

    let debug_args = parse_debug_args(args).map_err(|msg| {
        print_red(&msg);
    })?;

    let choice = select_debug_choice(&debug_args)?;
    let auth_token = crypto::decrypt_token(&choice.config.auth_token_encrypted).map_err(|_| {
        print_red("无法解密 API Key");
    })?;
    let model = remove_size_marker(&choice.model);

    maybe_install_mock_ollama()?;
    maybe_update_mock_ollama();

    let mut mock = spawn_mock_ollama(&debug_args, &choice.config.base_url, &auth_token)?;
    if wait_for_mock_ollama(&mut mock, &debug_args.host, debug_args.port).is_err() {
        stop_mock_ollama(&mut mock);
        return Err(());
    }

    let local_base_url = format!("http://{}:{}", debug_args.host, debug_args.port);
    print_green(&format!("debug 代理已启动: {local_base_url}"));
    print_cyan(&format!("日志: {}", debug_log_path().display()));

    let max_ctx = context_size_marker(&choice.model);
    set_claude_env(&local_base_url, "sk-clash-debug", &model, max_ctx);
    let claude_path = claude::find_claude_binary()?;
    claude::maybe_check_update();
    let cmd_args = claude_args(&model, &debug_args.claude_args);

    let status = process::Command::new(&claude_path)
        .args(&cmd_args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|err| {
            stop_mock_ollama(&mut mock);
            print_red(&format!("无法启动 claude: {err}"));
        })?;

    stop_mock_ollama(&mut mock);
    process::exit(status.code().unwrap_or(1));
}

fn launch_selected_claude(
    extra_args: &[String],
    model_hint: Option<&str>,
    cwd_hint: Option<&str>,
) -> Result<(), ()> {
    statusline::ensure_statusline_config();

    let choice = select_run_choice_with_model_hint(model_hint)?;
    let auth_token = crypto::decrypt_token(&choice.config.auth_token_encrypted).map_err(|_| {
        print_red("无法解密 API Key");
    })?;

    let model = remove_size_marker(&choice.model);
    let max_ctx = context_size_marker(&choice.model);

    set_claude_env(&choice.config.base_url, &auth_token, &model, max_ctx);

    let claude_path = claude::find_claude_binary()?;
    claude::maybe_check_update();

    let cmd_args = claude_args(&model, extra_args);

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let mut command = process::Command::new(&claude_path);
        command.args(&cmd_args);
        if let Some(cwd) = cwd_hint {
            command.current_dir(cwd);
        }
        let err = command.exec();
        print_red(&format!("exec claude 失败: {}", err));
        process::exit(127);
    }

    #[cfg(not(unix))]
    {
        let mut command = process::Command::new(&claude_path);
        command.args(&cmd_args);
        if let Some(cwd) = cwd_hint {
            command.current_dir(cwd);
        }
        let status = command.status().expect("无法启动 claude");
        process::exit(status.code().unwrap_or(1));
    }

    #[allow(unreachable_code)]
    Ok(())
}

pub fn do_resume(
    extra_args: &[String],
    _print_red: fn(&str),
    _print_green: fn(&str),
    _print_yellow: fn(&str),
    _print_cyan: fn(&str),
) -> Result<(), ()> {
    statusline::ensure_statusline_config();

    let current_sessions =
        claude_history::load_sessions(SessionScope::CurrentProject).map_err(|err| {
            print_red(&err);
        })?;
    let all_sessions = claude_history::load_sessions(SessionScope::AllProjects).map_err(|err| {
        print_red(&err);
    })?;

    if current_sessions.is_empty() && all_sessions.is_empty() {
        print_yellow("未找到 Claude 历史会话");
        return Err(());
    }

    let session_id = tui::select_resume_session(&current_sessions, &all_sessions).ok_or(())?;
    let selected_session = current_sessions
        .iter()
        .chain(all_sessions.iter())
        .find(|session| session.id == session_id)
        .cloned();
    let model_hint = selected_session
        .as_ref()
        .and_then(|session| session.model.as_deref());
    let cwd_hint = selected_session
        .as_ref()
        .and_then(|session| session.cwd.as_deref());
    let mut resume_args = vec!["--resume".to_string(), session_id];
    resume_args.extend(extra_args.iter().cloned());
    launch_selected_claude(&resume_args, model_hint, cwd_hint)
}

pub fn do_select_and_run(
    extra_args: &[String],
    _print_red: fn(&str),
    _print_green: fn(&str),
    _print_yellow: fn(&str),
    _print_cyan: fn(&str),
) -> Result<(), ()> {
    launch_selected_claude(extra_args, None, None)
}
