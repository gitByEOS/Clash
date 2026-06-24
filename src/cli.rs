use crate::chat::do_chat;
use crate::lark;
use crate::ops::{
    do_config, do_debug, do_hooks, do_prompts, do_rename, do_reset, do_resume, do_select_and_run,
    do_test, do_update, do_version,
};
use crate::statusline;
use std::collections::HashMap;
use std::env;
use std::process;

const APP_VERSION: &str = concat!("v", env!("CARGO_PKG_VERSION"));
const DEFAULT_RAW_BASE_URL: &str = "https://raw.githubusercontent.com/gitByEOS/Clash/master";

#[cfg(unix)]
pub fn print_red(msg: &str) {
    println!("\x1b[1;31m{}\x1b[0m", msg);
}
#[cfg(unix)]
pub fn print_green(msg: &str) {
    println!("\x1b[1;32m{}\x1b[0m", msg);
}
#[cfg(unix)]
pub fn print_yellow(msg: &str) {
    println!("\x1b[1;33m{}\x1b[0m", msg);
}
#[cfg(unix)]
pub fn print_cyan(msg: &str) {
    println!("\x1b[1;36m{}\x1b[0m", msg);
}

#[cfg(not(unix))]
pub fn print_red(msg: &str) {
    println!("{}", msg);
}
#[cfg(not(unix))]
pub fn print_green(msg: &str) {
    println!("{}", msg);
}
#[cfg(not(unix))]
pub fn print_yellow(msg: &str) {
    println!("{}", msg);
}
#[cfg(not(unix))]
pub fn print_cyan(msg: &str) {
    println!("{}", msg);
}

pub struct ConfigSetArgs {
    pub idx: usize,
    pub base_url: Option<String>,
    pub auth_key: Option<String>,
    pub models: Option<String>,
}

/// 通用参数解析：遍历 args，遇 --xxx 取下一元素为值
#[allow(clippy::result_unit_err)]
pub fn parse_auth_args(
    args: &[String],
    flags: &[&str],
    verbose: bool,
) -> Result<HashMap<String, String>, ()> {
    let mut result = HashMap::new();
    let mut i = 0;

    while i < args.len() {
        let flag = &args[i];
        if !flags.contains(&flag.as_str()) {
            if verbose {
                print_red(&format!("未知参数: {}", flag));
            }
            return Err(());
        }
        if i + 1 >= args.len() {
            if verbose {
                print_red(&format!("{} 缺少值", flag));
            }
            return Err(());
        }
        i += 1;
        result.insert(flag.clone(), args[i].clone());
        i += 1;
    }

    Ok(result)
}

pub fn parse_idx(value: &str) -> Result<usize, ()> {
    value.parse::<usize>().map_err(|_| {
        print_red("--idx 必须是 0 或正整数");
    })
}

pub fn parse_config_set_args(args: &[String]) -> Result<ConfigSetArgs, ()> {
    let map = parse_auth_args(args, &["--idx", "--url", "--key", "--models"], true)?;
    let idx = map
        .get("--idx")
        .map(|value| parse_idx(value))
        .transpose()?
        .unwrap_or(0);
    Ok(ConfigSetArgs {
        idx,
        base_url: map.get("--url").cloned(),
        auth_key: map.get("--key").cloned(),
        models: map.get("--models").cloned(),
    })
}

pub fn exit_on_err(result: Result<(), ()>) {
    if result.is_err() {
        process::exit(1);
    }
}

pub fn launch() {
    let args: Vec<String> = env::args().skip(1).collect();

    if args.is_empty() {
        exit_on_err(do_select_and_run(
            &[],
            print_red,
            print_green,
            print_yellow,
            print_cyan,
        ));
        return;
    }

    match args[0].as_str() {
        "version" => do_version(APP_VERSION),
        "update" => exit_on_err(do_update(
            APP_VERSION,
            DEFAULT_RAW_BASE_URL,
            print_red,
            print_green,
            print_cyan,
        )),
        "statusline" => statusline::do_statusline(),
        "run" => exit_on_err(do_select_and_run(
            &args[1..],
            print_red,
            print_green,
            print_yellow,
            print_cyan,
        )),
        "debug" => exit_on_err(do_debug(
            &args[1..],
            print_red,
            print_green,
            print_yellow,
            print_cyan,
        )),
        "config" => exit_on_err(do_config(
            &args[1..],
            print_red,
            print_green,
            print_yellow,
            print_cyan,
            parse_config_set_args,
        )),
        "reset" => exit_on_err(do_reset(print_red, print_green)),
        "resume" => exit_on_err(do_resume(
            &args[1..],
            print_red,
            print_green,
            print_yellow,
            print_cyan,
        )),
        "test" => exit_on_err(do_test(
            &args[1..],
            print_red,
            print_green,
            print_yellow,
            print_cyan,
        )),
        "lark" => exit_on_err(lark::do_lark(&args[1..])),
        "chat" => exit_on_err(do_chat(&args[1..])),
        "hooks" => exit_on_err(do_hooks()),
        "prompts" => exit_on_err(do_prompts(&args[1..], print_red, print_green)),
        "rename" => exit_on_err(do_rename(print_red, print_green, print_yellow, print_cyan)),
        _ => exit_on_err(do_select_and_run(
            &args,
            print_red,
            print_green,
            print_yellow,
            print_cyan,
        )),
    }
}
