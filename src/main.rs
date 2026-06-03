mod api_test;
mod config;
mod crypto;
mod fuzzy;
mod tui;

use config::ClashConfig;
use std::env;
use std::io::Write;
use std::process;

const APP_VERSION: &str = concat!("v", env!("CARGO_PKG_VERSION"));
const DEFAULT_RAW_BASE_URL: &str = "https://raw.githubusercontent.com/gitByEOS/Clash/master";

fn print_red(msg: &str) {
    print!("\x1b[1;31m{}\x1b[0m\n", msg);
}
fn print_green(msg: &str) {
    print!("\x1b[1;32m{}\x1b[0m\n", msg);
}
fn print_yellow(msg: &str) {
    print!("\x1b[1;33m{}\x1b[0m\n", msg);
}
fn print_cyan(msg: &str) {
    print!("\x1b[1;36m{}\x1b[0m\n", msg);
}

/// ── version / update ────────────────────────────────────────────────

fn raw_base_url() -> String {
    env::var("CLASH_INSTALL_BASE_URL").unwrap_or_else(|_| DEFAULT_RAW_BASE_URL.to_string())
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

fn do_version() {
    println!("{}", APP_VERSION);
}

fn do_update() -> Result<(), ()> {
    let base_url = raw_base_url();
    let cargo_toml_url = format!("{base_url}/Cargo.toml");
    let cargo_toml = fetch_text(&cargo_toml_url).map_err(|err| {
        print_red(&err);
    })?;
    let latest = latest_version_from_cargo_toml(&cargo_toml).ok_or_else(|| {
        print_red("无法从 Cargo.toml 读取最新版本");
    })?;

    if latest == APP_VERSION {
        print_green(&format!("已是最新版本: {}", APP_VERSION));
        return Ok(());
    }

    print_cyan(&format!("发现新版本: {} -> {}", APP_VERSION, latest));
    let install_url = format!("{base_url}/install.sh");
    let status = process::Command::new("bash")
        .arg("-c")
        .arg(format!(
            "curl -fsSL '{}' | bash",
            install_url.replace('\'', "'\\''")
        ))
        .status()
        .map_err(|e| {
            print_red(&format!("无法执行安装脚本: {e}"));
        })?;

    if status.success() {
        Ok(())
    } else {
        print_red("更新失败");
        Err(())
    }
}

/// ── config ─────────────────────────────────────────────────────────

struct ConfigSetArgs {
    base_url: Option<String>,
    auth_key: Option<String>,
    models: Option<String>,
}

fn parse_config_set_args(args: &[String]) -> Result<ConfigSetArgs, ()> {
    let mut base_url = None;
    let mut auth_key = None;
    let mut models = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--url" => {
                if i + 1 >= args.len() {
                    print_red("--url 缺少值");
                    return Err(());
                }
                i += 1;
                base_url = Some(args[i].clone());
            }
            "--key" => {
                if i + 1 >= args.len() {
                    print_red("--key 缺少值");
                    return Err(());
                }
                i += 1;
                auth_key = Some(args[i].clone());
            }
            "--models" => {
                if i + 1 >= args.len() {
                    print_red("--models 缺少值");
                    return Err(());
                }
                i += 1;
                models = Some(args[i].clone());
            }
            other => {
                print_red(&format!("未知参数: {}", other));
                return Err(());
            }
        }
        i += 1;
    }

    Ok(ConfigSetArgs {
        base_url,
        auth_key,
        models,
    })
}

fn save_config(base_url: String, auth_token: String, models: Vec<String>) -> Result<(), ()> {
    let cfg = ClashConfig {
        base_url,
        auth_token_encrypted: crypto::encrypt_token(&auth_token).map_err(|_| ())?,
        command: "clash".to_string(),
        models,
    };

    config::write_config(&cfg).map_err(|_| ())?;
    let config_path = config::config_path();
    print_green(&format!("配置已保存到 {}", config_path.display()));
    print_green("API Key 已加密存储");
    auto_test_after_config()
}

fn do_configure_interactive() -> Result<(), ()> {
    print_cyan("Clash 配置向导");

    let mut buf = String::new();
    print!("请输入 Anthropic 兼容 API 地址\n> ");
    std::io::stdout().flush().unwrap();
    std::io::stdin().read_line(&mut buf).unwrap();
    let base_url = buf.trim().to_string();
    if base_url.is_empty() {
        print_red("地址不能为空");
        return Err(());
    }

    buf.clear();
    print!("请输入 API Key\n> ");
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
        print!("请输入模型列表，多个模型用逗号分隔\n> ");
        std::io::stdout().flush().unwrap();
        std::io::stdin().read_line(&mut buf).unwrap();
        model_list = config::normalize_models(buf.trim());
        if model_list.is_empty() {
            print_red("模型列表不能为空");
        }
    }

    save_config(base_url, auth_token, model_list)
}

fn load_config_for_update() -> Result<ClashConfig, ()> {
    match config::read_config_raw() {
        Ok(cfg) => Ok(cfg),
        Err(config::ConfigError::NotFound) => Ok(ClashConfig {
            base_url: String::new(),
            auth_token_encrypted: String::new(),
            command: "clash".to_string(),
            models: vec![],
        }),
        Err(_) => Err(()),
    }
}

fn do_config(args: &[String]) -> Result<(), ()> {
    if args.is_empty() {
        return do_config_show();
    }

    let parsed = parse_config_set_args(args)?;
    if parsed.base_url.is_none() && parsed.auth_key.is_none() && parsed.models.is_none() {
        print_red("请至少提供一个 --url、--key 或 --models");
        return Err(());
    }

    let mut cfg = load_config_for_update()?;

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

    config::write_config(&cfg).map_err(|_| ())?;
    let config_path = config::config_path();
    print_green(&format!("配置已保存到 {}", config_path.display()));
    if !cfg.auth_token_encrypted.is_empty() {
        print_green("API Key 已加密存储");
    }
    auto_test_after_config()
}

fn do_config_show() -> Result<(), ()> {
    let cfg = config::read_config_raw().map_err(|_| {
        print_yellow("未配置，请运行 clash 进行初始化");
    })?;

    if cfg.base_url.is_empty() && cfg.auth_token_encrypted.is_empty() && cfg.models.is_empty() {
        print_yellow("未配置，请运行 clash 进行初始化");
        return Err(());
    }

    print_cyan("=== 当前配置 ===");
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

fn do_reset() -> Result<(), ()> {
    config::delete_config().map_err(|_| ())?;
    print_green(&format!("已删除配置 {}", config::config_path().display()));
    Ok(())
}

/// ── add-model ──────────────────────────────────────────────────────

fn do_add_model(new_model: &str) -> Result<(), ()> {
    let mut cfg = config::read_config().map_err(|_| {
        print_red("未找到配置，请先运行 clash");
    })?;

    if new_model.is_empty() {
        print_red("用法: clash add-model <模型名>");
        return Err(());
    }

    if cfg.models.iter().any(|m| m == new_model) {
        print_yellow(&format!("模型 {} 已存在", new_model));
        return Ok(());
    }

    cfg.models.push(new_model.to_string());
    config::write_config(&cfg).map_err(|_| ())?;
    print_green(&format!("已添加模型: {}", new_model));
    auto_test_after_config()
}

/// ── change-token ───────────────────────────────────────────────────

fn do_change_token(new_token: &str) -> Result<(), ()> {
    let mut cfg = config::read_config().map_err(|_| {
        print_red("未找到配置，请先运行 clash");
    })?;

    if new_token.is_empty() {
        print_red("用法: clash change-token <新Key>");
        return Err(());
    }

    cfg.auth_token_encrypted = crypto::encrypt_token(new_token).map_err(|_| ())?;
    config::write_config(&cfg).map_err(|_| ())?;
    print_green("API Key 已更新");
    auto_test_after_config()
}

/// ── test ───────────────────────────────────────────────────────────

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
fn auto_test_after_config() -> Result<(), ()> {
    if should_skip_auto_test() {
        return Ok(());
    }

    let cfg = match config::read_config_raw() {
        Ok(c) => c,
        Err(_) => return Ok(()),
    };
    if cfg.base_url.is_empty() || cfg.auth_token_encrypted.is_empty() || cfg.models.is_empty() {
        print_yellow("配置不完整，跳过连通性测试");
        return Ok(());
    }

    print_cyan("正在进行 Anthropic 兼容 API 连通测试（curl POST /v1/messages）...");
    let opts = api_test::TestOptions {
        base_url: None,
        auth_key: None,
        model: None,
    };

    let ctx = api_test::prepare(&opts).map_err(|err| {
        print_red(&err);
    })?;
    if run_model_probes(&ctx) {
        Ok(())
    } else {
        Err(())
    }
}

fn do_test(args: &[String]) -> Result<(), ()> {
    let opts = api_test::parse_test_args(args).map_err(|_| {
        print_red("用法: clash test [--url <地址>] [--key <Key>] [--model <模型>]");
    })?;

    print_cyan("正在进行 Anthropic 兼容 API 连通测试（curl POST /v1/messages）...");
    flush_stdout();

    let ctx = api_test::prepare(&opts).map_err(|err| {
        print_red(&err);
    })?;

    if run_model_probes(&ctx) {
        Ok(())
    } else {
        Err(())
    }
}

/// ── select and run ─────────────────────────────────────────────────

fn do_select_and_run(extra_args: &[String]) -> Result<(), ()> {
    let mut cfg = match config::read_config() {
        Ok(c) => c,
        Err(_) => {
            print_yellow("未找到配置，请先配置厂商地址和 API Key");
            do_configure_interactive()?;
            config::read_config().map_err(|_| ())?
        }
    };

    if cfg.base_url.is_empty() || cfg.auth_token_encrypted.is_empty() || cfg.models.is_empty() {
        print_red("配置不完整，请重新配置");
        do_configure_interactive()?;
        cfg = config::read_config().map_err(|_| ())?;
    }

    let auth_token = crypto::decrypt_token(&cfg.auth_token_encrypted).map_err(|_| {
        print_red("无法解密 API Key");
    })?;

    let model = tui::select_model(&cfg.models).ok_or_else(|| ())?;

    print_cyan(&format!("模型: {}", model));
    print_cyan(&format!("地址: {}", cfg.base_url));

    // Set environment variables
    env::set_var("ANTHROPIC_BASE_URL", &cfg.base_url);
    env::set_var("ANTHROPIC_AUTH_TOKEN", &auth_token);
    env::set_var("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1");
    env::set_var("CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS", "1");
    env::set_var("CLAUDE_CODE_ATTRIBUTION_HEADER", "0");
    env::set_var("CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS", "1");
    env::set_var("CLAUDE_CODE_ENABLE_AUTO_MODE", "1");
    env::set_var("CLAUDE_CODE_SUBAGENT_MODEL", &model);
    env::set_var("ANTHROPIC_MODEL", &model);
    env::set_var("ANTHROPIC_SMALL_FAST_MODEL", &model);
    env::set_var("ANTHROPIC_DEFAULT_SONNET_MODEL", &model);
    env::set_var("ANTHROPIC_DEFAULT_OPUS_MODEL", &model);
    env::set_var("ANTHROPIC_DEFAULT_HAIKU_MODEL", &model);

    // Find and exec claude
    let claude_path = find_claude_binary();
    let mut cmd_args = vec![
        "--permission-mode",
        "bypassPermissions",
        "--effort",
        "max",
        "--model",
        &model,
    ];
    for arg in extra_args {
        cmd_args.push(arg.as_str());
    }

    // 替换当前进程；argv 须跨 exec 调用存活，块内临时 vec 会悬空导致 EFAULT
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let err = process::Command::new(&claude_path).args(&cmd_args).exec();
        print_red(&format!("exec claude 失败: {}", err));
        process::exit(127);
    }

    #[cfg(not(unix))]
    {
        let status = process::Command::new(&claude_path)
            .args(&cmd_args)
            .status()
            .expect("无法启动 claude");
        process::exit(status.code().unwrap_or(1));
    }

    #[allow(unreachable_code)]
    Ok(())
}

fn find_claude_binary() -> String {
    // Look for "claude" in PATH
    if let Ok(path_env) = env::var("PATH") {
        for dir in path_env.split(':') {
            let candidate = format!("{}/claude", dir);
            if std::path::Path::new(&candidate).exists() {
                return candidate;
            }
        }
    }
    // Fallback: just "claude", let the shell find it
    "claude".to_string()
}

/// ── main ───────────────────────────────────────────────────────────

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();

    if args.is_empty() {
        if let Err(()) = do_select_and_run(&[]) {
            process::exit(1);
        }
        return;
    }

    match args[0].as_str() {
        "version" => {
            do_version();
        }
        "update" => {
            if let Err(()) = do_update() {
                process::exit(1);
            }
        }
        "run" => {
            if let Err(()) = do_select_and_run(&args[1..]) {
                process::exit(1);
            }
        }
        "config" => {
            if let Err(()) = do_config(&args[1..]) {
                process::exit(1);
            }
        }
        "reset" => {
            if let Err(()) = do_reset() {
                process::exit(1);
            }
        }
        "add-model" => {
            let model = args.get(1).map(|s| s.as_str()).unwrap_or("");
            if let Err(()) = do_add_model(model) {
                process::exit(1);
            }
        }
        "change-token" => {
            let token = args.get(1).map(|s| s.as_str()).unwrap_or("");
            if let Err(()) = do_change_token(token) {
                process::exit(1);
            }
        }
        "test" => {
            if let Err(()) = do_test(&args[1..]) {
                process::exit(1);
            }
        }
        _ => {
            // Unknown subcommand, treat as claude args
            if let Err(()) = do_select_and_run(&args) {
                process::exit(1);
            }
        }
    }
}
