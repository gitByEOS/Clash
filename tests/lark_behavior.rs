#![allow(dead_code)]

mod lark {
    pub mod types {
        include!("../src/lark/types.rs");
    }

    pub mod render {
        include!("../src/lark/render.rs");
    }

    pub mod command {
        include!("../src/lark/command.rs");
    }

    pub mod lark_cli {
        include!("../src/lark/lark_cli.rs");

        pub fn parse_chat_for_test(value: &serde_json::Value) -> Option<super::types::LarkChat> {
            parse_chat(value)
        }

        pub fn parse_text_for_test(raw_content: &str, message_type: &str) -> String {
            parse_message_text(raw_content, message_type)
        }

        pub fn parse_message_for_test(
            value: &serde_json::Value,
        ) -> Option<super::types::LarkMessage> {
            parse_lark_message(value)
        }
    }

    pub mod card {
        include!("../src/lark/card.rs");

        pub fn accept_monotonic_text_for_test(current: &mut String, text: &str) -> Option<String> {
            accept_monotonic_text(current, text)
        }

        pub fn streaming_card_for_test(text: &str, streaming: bool) -> serde_json::Value {
            build_streaming_card(text, streaming)
        }
    }
}

use lark::command::{normalize_session_chat_name, parse_create_session_command, parse_lark_args};
use lark::render::ClaudeRenderState;
use serde_json::{json, Value};

#[test]
fn parse_args_defaults() {
    let opts = parse_lark_args(&[]).unwrap();
    assert_eq!(opts.prefix, "Clash-");
    assert_eq!(opts.poll_secs, 15);
    assert!(!opts.once);
}

#[test]
fn parse_args_custom() {
    let opts = parse_lark_args(&[
        "--prefix".into(),
        "Clash-Dev-".into(),
        "--poll-secs".into(),
        "3".into(),
        "--once".into(),
    ])
    .unwrap();
    assert_eq!(opts.prefix, "Clash-Dev-");
    assert_eq!(opts.poll_secs, 3);
    assert!(opts.once);
}

#[test]
fn parse_create_session_command_accepts_separators() {
    assert_eq!(
        parse_create_session_command("新建群组:demo").unwrap(),
        "demo"
    );
    assert_eq!(
        parse_create_session_command("新建群组，demo").unwrap(),
        "demo"
    );
    assert_eq!(parse_create_session_command("新会话 demo").unwrap(), "demo");
    assert_eq!(parse_create_session_command("接入:demo").unwrap(), "demo");
    assert_eq!(parse_create_session_command("接入，demo").unwrap(), "demo");
    assert_eq!(parse_create_session_command("接入 demo").unwrap(), "demo");
}

#[test]
fn normalize_session_chat_name_adds_prefix() {
    assert_eq!(normalize_session_chat_name("demo", "Clash-"), "Clash-demo");
    assert_eq!(
        normalize_session_chat_name("Clash-demo", "Clash-"),
        "Clash-demo"
    );
}

#[test]
fn parse_chat_accepts_lark_fields() {
    let chat = lark::lark_cli::parse_chat_for_test(&json!({
        "chat_id": "oc_1",
        "name": "Clash-demo"
    }))
    .unwrap();
    assert_eq!(chat.chat_id, "oc_1");
    assert_eq!(chat.name, "Clash-demo");
}

#[test]
fn parse_text_message_content() {
    assert_eq!(
        lark::lark_cli::parse_text_for_test(r#"{"text":"你好"}"#, "text"),
        "你好"
    );
}

#[test]
fn parse_lark_message_skips_bot() {
    let value = json!({
        "sender_type": "app",
        "sender_id": "cli_x",
        "chat_id": "oc_1",
        "message_id": "om_1",
        "content": "{\"text\":\"x\"}",
        "message_type": "text"
    });
    assert!(lark::lark_cli::parse_message_for_test(&value).is_none());
}

#[test]
fn render_state_renders_thinking_and_tool_input_inline() {
    let mut state = ClaudeRenderState::new();
    let event = json!({
        "type": "assistant",
        "message": {
            "content": [
                {"type": "thinking", "thinking": "先分析"},
                {"type": "tool_use", "id": "t1", "name": "Bash", "input": {"command": "ls"}},
                {"type": "text", "text": "完成"}
            ]
        }
    });
    assert!(state.apply_event(&event));
    let rendered = state.render();
    assert!(rendered.contains("- `Thinking`\n```markdown\n先分析\n```"));
    assert!(rendered.contains("- `Bash`\n```json\n"));
    assert!(rendered.contains("```json"));
    assert!(rendered.contains(r#"{"command":"ls"}"#));
    assert!(rendered.contains("完成"));
}

#[test]
fn render_state_truncates_thinking_after_50_chars() {
    let mut state = ClaudeRenderState::new();
    let event = json!({
        "type": "assistant",
        "message": {
            "content": [
                {"type": "thinking", "thinking": "123456789012345678901234567890123456789012345678901"}
            ]
        }
    });
    assert!(state.apply_event(&event));
    assert!(state.render().contains(
        "- `Thinking`\n```markdown\n12345678901234567890123456789012345678901234567890...\n```"
    ));
}

#[test]
fn render_state_does_not_rollback_when_old_snapshot_replays() {
    let mut state = ClaudeRenderState::new();
    let newer = json!({
        "type": "assistant",
        "message": {
            "content": [
                {"type": "tool_use", "id": "t1", "name": "Bash", "input": {"command": "one"}},
                {"type": "tool_use", "id": "t2", "name": "Read", "input": {"file": "two"}},
                {"type": "tool_use", "id": "t3", "name": "Edit", "input": {"file": "three"}}
            ]
        }
    });
    let older = json!({
        "type": "assistant",
        "message": {
            "content": [
                {"type": "tool_use", "id": "t1", "name": "Bash", "input": {"command": "one"}}
            ]
        }
    });

    assert!(state.apply_event(&newer));
    let rendered = state.render();
    assert!(rendered.contains("- `Edit`"));
    assert!(!state.apply_event(&older));
    assert_eq!(state.render(), rendered);
}

#[test]
fn render_state_ignores_tool_result_output() {
    let mut state = ClaudeRenderState::new();
    let event = json!({
        "type": "user",
        "message": {
            "content": [
                {"type": "tool_result", "tool_use_id": "t1", "content": "ok"}
            ]
        }
    });
    assert!(!state.apply_event(&event));
    assert!(!state.render().contains("ok"));
}

#[test]
fn build_streaming_card_uses_body_element() {
    let card = lark::card::streaming_card_for_test("hello", true);
    assert_eq!(card["config"]["streaming_mode"], true);
    assert_eq!(
        card["body"]["elements"][0]["element_id"],
        Value::String("body_md".to_string())
    );
}

#[test]
fn accept_monotonic_text_rejects_rollback() {
    let mut current = String::new();
    assert_eq!(
        lark::card::accept_monotonic_text_for_test(&mut current, "one"),
        Some("one".to_string())
    );
    assert_eq!(
        lark::card::accept_monotonic_text_for_test(&mut current, "one two"),
        Some("one two".to_string())
    );
    assert_eq!(
        lark::card::accept_monotonic_text_for_test(&mut current, "one"),
        None
    );
    assert_eq!(current, "one two");
}
