use crate::config;
use crate::crypto;
use crate::cli::parse_auth_args;
use std::io;
use std::process::{Command, Stdio};

/// Claude Code 出站标识，Coding Plan 网关会校验
const CLAUDE_CODE_USER_AGENT: &str = "claude-cli/2.1.118 (external, cli)";

/// `clash test` 可选参数
pub struct TestOptions {
    pub idx: Option<usize>,  // None means test all accounts
    pub base_url: Option<String>,
    pub auth_key: Option<String>,
    pub model: Option<String>,
}

/// 单模型连通测试结果
pub struct ModelProbeResult {
    pub model: String,
    pub ok: bool,
    pub detail: Option<String>,
}

pub fn parse_test_args(args: &[String]) -> Result<TestOptions, ()> {
    let map = parse_auth_args(args, &["--idx", "--url", "--key", "--model", "--all"], false)?;
    let has_all = map.contains_key("--all");
    let idx = map
        .get("--idx")
        .map(|value| value.parse::<usize>().map_err(|_| ()))
        .transpose()?;
    // --all 强制测试全部，否则如果没有指定 --idx 也测试全部
    let idx = if has_all { None } else { idx };
    Ok(TestOptions {
        idx,
        base_url: map.get("--url").cloned(),
        auth_key: map.get("--key").cloned(),
        model: map.get("--model").cloned(),
    })
}

/// 拼 Anthropic Messages 端点
pub fn messages_endpoint(base_url: &str) -> String {
    let base = base_url.trim().trim_end_matches('/');
    if base.ends_with("/v1") {
        format!("{base}/messages")
    } else {
        format!("{base}/v1/messages")
    }
}

fn is_dashscope_host(base_url: &str) -> bool {
    base_url.to_lowercase().contains("dashscope")
}

fn json_string(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r");
    format!("\"{escaped}\"")
}

/// Anthropic Messages 最小请求体
fn probe_body(model: &str, base_url: &str) -> String {
    let model = json_string(model);
    if is_dashscope_host(base_url) {
        format!(
            "{{\"model\":{model},\"max_tokens\":1,\"thinking\":{{\"type\":\"disabled\"}},\"messages\":[{{\"role\":\"user\",\"content\":\"ping\"}}]}}"
        )
    } else {
        format!(
            "{{\"model\":{model},\"max_tokens\":1,\"messages\":[{{\"role\":\"user\",\"content\":\"ping\"}}]}}"
        )
    }
}

fn resolve_models(opts: &TestOptions, cfg: &config::ClashConfig) -> Result<Vec<String>, String> {
    if let Some(model) = opts.model.as_ref().filter(|s| !s.is_empty()) {
        return Ok(vec![model.clone()]);
    }
    if cfg.models.is_empty() {
        return Err("缺少模型，请配置 MODELS 或使用 --model".to_string());
    }
    Ok(cfg.models.clone())
}

/// 连通测试所需上下文
pub struct TestContext {
    pub base_url: String,
    pub auth_token: String,
    pub models: Vec<String>,
}

pub fn prepare_for_idx(idx: usize, opts: &TestOptions) -> Result<TestContext, String> {
    let cfg = config::read_config_raw_for_idx(idx).map_err(|e| e.to_string())?;

    let base_url = opts
        .base_url
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| cfg.base_url.clone());
    if base_url.is_empty() {
        return Err("缺少 BASE_URL，请先 clash config --url ...".to_string());
    }

    let auth_token = if let Some(key) = opts.auth_key.as_ref().filter(|s| !s.is_empty()) {
        key.clone()
    } else if cfg.auth_token_encrypted.is_empty() {
        return Err("缺少 API Key，请先 clash config --key ...".to_string());
    } else {
        crypto::decrypt_token(&cfg.auth_token_encrypted)
            .map_err(|_| "无法解密 API Key".to_string())?
    };

    let models = resolve_models(opts, &cfg)?;
    Ok(TestContext {
        base_url,
        auth_token,
        models,
    })
}

/// 单模型 curl 连通测试
pub fn probe_one(ctx: &TestContext, model: &str) -> ModelProbeResult {
    let probe = probe_with_curl(&ctx.base_url, &ctx.auth_token, model);
    ModelProbeResult {
        model: model.to_string(),
        ok: probe.is_ok(),
        detail: probe.err(),
    }
}

/// 用 curl 发 Anthropic 风格 POST
fn probe_with_curl(base_url: &str, auth_token: &str, model: &str) -> Result<(), String> {
    let url = messages_endpoint(base_url);
    let body = probe_body(model, base_url);

    let output = Command::new("curl")
        .arg("-sS")
        .arg("-w")
        .arg("\n%{http_code}")
        .arg("-X")
        .arg("POST")
        .arg("--max-time")
        .arg("30")
        .arg("-H")
        .arg("content-type: application/json")
        .arg("-H")
        .arg(format!("x-api-key: {auth_token}"))
        .arg("-H")
        .arg("anthropic-version: 2023-06-01")
        .arg("-H")
        .arg(format!("user-agent: {CLAUDE_CODE_USER_AGENT}"))
        .arg("-H")
        .arg("x-app: cli")
        .arg("-H")
        .arg("anthropic-beta: interleaved-thinking-2025-05-14")
        .arg("-d")
        .arg(&body)
        .arg(&url)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| {
            if e.kind() == io::ErrorKind::NotFound {
                "未找到 curl，请先安装 curl".to_string()
            } else {
                format!("curl 启动失败: {e}")
            }
        })?;

    if !output.status.success() && output.stdout.is_empty() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            "curl 执行失败".to_string()
        } else {
            format!("curl 执行失败: {stderr}")
        });
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let (resp_body, status) = split_curl_output(&text)?;

    if (200..300).contains(&status) {
        return Ok(());
    }

    let detail: String = resp_body.chars().take(300).collect();
    let mut err = if detail.is_empty() {
        format!("HTTP {status}")
    } else {
        format!("HTTP {status}: {detail}")
    };

    if status == 405 && detail.contains("Coding Agents") {
        err.push_str("（Coding Plan 需经 Claude Code 等编程工具调用）");
    }

    Err(err)
}

fn split_curl_output(text: &str) -> Result<(String, u16), String> {
    let trimmed = text.trim_end();
    let (body, code_line) = trimmed
        .rsplit_once('\n')
        .ok_or_else(|| "curl 响应格式异常".to_string())?;
    let status = code_line
        .parse::<u16>()
        .map_err(|_| format!("无法解析 HTTP 状态码: {code_line}"))?;
    Ok((body.to_string(), status))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_endpoint_appends_v1() {
        assert_eq!(
            messages_endpoint("https://api.example.com/anthropic"),
            "https://api.example.com/anthropic/v1/messages"
        );
    }

    #[test]
    fn resolve_models_uses_config_list_without_flag() {
        let cfg = config::ClashConfig {
            base_url: String::new(),
            auth_token_encrypted: String::new(),
            command: "clash".to_string(),
            models: vec!["a".into(), "b".into()],
            name: None,
        };
        let opts = TestOptions {
            idx: Some(0),
            base_url: None,
            auth_key: None,
            model: None,
        };
        assert_eq!(resolve_models(&opts, &cfg).unwrap(), vec!["a", "b"]);
    }

    #[test]
    fn split_curl_output_parses_status() {
        let (body, code) = split_curl_output("{\"ok\":true}\n200").unwrap();
        assert_eq!(body, "{\"ok\":true}");
        assert_eq!(code, 200);
    }
}
