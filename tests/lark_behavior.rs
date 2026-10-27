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

        pub fn accept_monotonic_text_for_test(current: &mut String, text: &str) -> bool {
            accept_monotonic_text(current, text)
        }

        pub fn current_page_for_test(text: &str, page_start: usize) -> (String, bool) {
            current_page(text, page_start)
        }

        pub fn page_content_for_test(text: &str, page_start: usize) -> (String, bool) {
            page_content(text, page_start)
        }

        pub fn streaming_card_for_test(text: &str, streaming: bool) -> serde_json::Value {
            build_streaming_card(text, "model: demo · ctx: -- · ⏱ 1s · dir: notes", streaming)
        }
    }
}

use lark::command::{
    normalize_session_chat_name, parse_create_session_command, parse_lark_args,
    parse_session_command, resolve_working_dir, SessionCommand,
};
use lark::render::{
    format_context_percent, format_elapsed, format_session_footer, queue_fold, queue_result,
    queue_stop, ClaudeRenderState, Fold, OutboundOp, TokenUsage,
};
use serde_json::{json, Value};
use std::collections::VecDeque;

#[test]
fn parse_args_defaults() {
    let opts = parse_lark_args(&[]).unwrap();
    assert_eq!(opts.prefix, "Clash-");
    assert_eq!(opts.poll_secs, 15);
    assert!(!opts.once);
    assert_eq!(opts.model_index, None);
}

#[test]
fn parse_args_accepts_model_index() {
    let opts = parse_lark_args(&["-m".into(), "3".into()]).unwrap();
    assert_eq!(opts.model_index, Some(3));
}

#[test]
fn parse_args_accepts_zero_model_index() {
    let opts = parse_lark_args(&["-m".into(), "0".into()]).unwrap();
    assert_eq!(opts.model_index, Some(0));
}

#[test]
fn parse_args_rejects_invalid_model_index() {
    assert!(parse_lark_args(&["-m".into(), "abc".into()]).is_err());
    assert!(parse_lark_args(&["-m".into()]).is_err());
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
fn parse_session_command_stop_is_whole_message() {
    assert_eq!(parse_session_command("/stop"), Some(SessionCommand::Stop));
    assert_eq!(parse_session_command(" /STOP "), Some(SessionCommand::Stop));
    assert_eq!(parse_session_command("请 /stop"), None);
    assert_eq!(parse_session_command("/clear"), None);
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
fn resolve_working_dir_ignores_empty() {
    assert_eq!(resolve_working_dir(None).unwrap(), None);
    assert_eq!(resolve_working_dir(Some("")).unwrap(), None);
    assert_eq!(resolve_working_dir(Some("  ")).unwrap(), None);
}

#[test]
fn resolve_working_dir_rejects_missing() {
    let err = resolve_working_dir(Some("/no-such-clash-dir-xyz")).unwrap_err();
    assert!(err.contains("DIR 不是目录"));
}

#[test]
fn resolve_working_dir_accepts_existing() {
    let path = std::env::temp_dir();
    let resolved = resolve_working_dir(Some(path.to_str().unwrap()))
        .unwrap()
        .unwrap();
    assert_eq!(resolved, path);
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
    assert_ne!(state.apply_event(&event), Fold::Unchanged);
    let rendered = state.render();
    assert!(rendered.contains("**进度**"), "应带进度标题：{rendered}");
    assert!(rendered.contains("- 💭 先分析"), "思考应为单行：{rendered}");
    assert!(
        rendered.contains("- `$ ls`"),
        "命令应为单行语义：{rendered}"
    );
    assert!(
        !rendered.contains("```json"),
        "不应倾倒原始 JSON：{rendered}"
    );
    assert!(rendered.contains("完成"));
}

#[test]
fn render_stream_omits_answer() {
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
    assert_ne!(state.apply_event(&event), Fold::Unchanged);
    let streamed = state.render_stream();
    assert!(streamed.contains("- `$ ls`"), "{streamed}");
    assert!(
        !streamed.contains("完成"),
        "流式进度不应带最终结果：{streamed}"
    );
    assert!(state.render().contains("完成"));
}

#[test]
fn render_state_keeps_full_thinking() {
    let mut state = ClaudeRenderState::new();
    let long = "123456789012345678901234567890123456789012345678901234567890";
    let event = json!({
        "type": "assistant",
        "message": {
            "content": [
                {"type": "thinking", "thinking": long}
            ]
        }
    });
    assert_ne!(state.apply_event(&event), Fold::Unchanged);
    let rendered = state.render();
    assert!(
        rendered.contains(&format!("- 💭 {long}")),
        "思考不应被截断：{rendered}"
    );
    assert!(!rendered.contains("..."), "不应出现省略号：{rendered}");
}

#[test]
fn render_state_grows_thinking_across_partial_events() {
    let mut state = ClaudeRenderState::new();
    let first = json!({
        "type": "assistant",
        "message": {"content": [{"type": "thinking", "thinking": "先分析问题"}]}
    });
    let second = json!({
        "type": "assistant",
        "message": {"content": [{"type": "thinking", "thinking": "先分析问题，然后读取配置"}]}
    });
    assert_ne!(state.apply_event(&first), Fold::Unchanged);
    assert_eq!(state.apply_event(&second), Fold::Patch);
    let rendered = state.render();
    assert!(
        rendered.contains("- 💭 先分析问题，然后读取配置"),
        "{rendered}"
    );
    assert!(
        !rendered.contains("先分析问题\n"),
        "同轮思考不应重复成两行：{rendered}"
    );
}

#[test]
fn render_state_rejects_thinking_rollback() {
    let mut state = ClaudeRenderState::new();
    let long = json!({
        "type": "assistant",
        "message": {"content": [{"type": "thinking", "thinking": "先分析问题再动手"}]}
    });
    let short = json!({
        "type": "assistant",
        "message": {"content": [{"type": "thinking", "thinking": "先分析"}]}
    });
    assert_ne!(state.apply_event(&long), Fold::Unchanged);
    assert_eq!(
        state.apply_event(&short),
        Fold::Unchanged,
        "旧快照回放不应产生变化"
    );
    assert!(state.render().contains("- 💭 先分析问题再动手"));
}

#[test]
fn render_state_separates_thinking_rounds() {
    let mut state = ClaudeRenderState::new();
    let event = json!({
        "type": "assistant",
        "message": {"content": [
            {"type": "thinking", "thinking": "第一轮思考"},
            {"type": "tool_use", "id": "t1", "name": "Read", "input": {"file_path": "a.rs"}},
            {"type": "thinking", "thinking": "第二轮思考"}
        ]}
    });
    assert_ne!(state.apply_event(&event), Fold::Unchanged);
    let rendered = state.render();
    assert!(rendered.contains("- 💭 第一轮思考"), "{rendered}");
    assert!(rendered.contains("- 💭 第二轮思考"), "{rendered}");
}

#[test]
fn render_state_appends_thinking_deltas_from_stream_events() {
    let mut state = ClaudeRenderState::new();
    let start = json!({
        "type": "stream_event",
        "event": {
            "type": "content_block_delta",
            "delta": {"type": "thinking_delta", "thinking": "先分析"}
        }
    });
    let more = json!({
        "type": "stream_event",
        "event": {
            "type": "content_block_delta",
            "delta": {"type": "thinking_delta", "thinking": "问题"}
        }
    });
    assert_eq!(state.apply_event(&start), Fold::Patch);
    assert_eq!(state.apply_event(&more), Fold::Patch);
    let rendered = state.render();
    assert!(
        rendered.contains("- 💭 先分析问题"),
        "delta 应拼成一行：{rendered}"
    );
}

#[test]
fn render_state_does_not_rollback_when_old_snapshot_replays() {
    let mut state = ClaudeRenderState::new();
    let newer = json!({
        "type": "assistant",
        "message": {
            "content": [
                {"type": "tool_use", "id": "t1", "name": "Bash", "input": {"command": "one"}},
                {"type": "tool_use", "id": "t2", "name": "Read", "input": {"file_path": "two"}},
                {"type": "tool_use", "id": "t3", "name": "Edit", "input": {"file_path": "three"}}
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

    assert_eq!(state.apply_event(&newer), Fold::Commit);
    let rendered = state.render();
    assert!(
        rendered.contains("- 修改 `three`"),
        "应按语义渲染：{rendered}"
    );
    assert_eq!(state.apply_event(&older), Fold::Unchanged);
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
    assert_eq!(state.apply_event(&event), Fold::Unchanged);
    assert!(!state.render().contains("ok"));
}

#[test]
fn render_state_maps_tools_to_semantic_lines() {
    let mut state = ClaudeRenderState::new();
    let event = json!({
        "type": "assistant",
        "message": {
            "content": [
                {"type": "tool_use", "id": "t1", "name": "Read", "input": {"file_path": "src/main.rs"}},
                {"type": "tool_use", "id": "t2", "name": "Write", "input": {"file_path": "a.txt"}},
                {"type": "tool_use", "id": "t3", "name": "Grep", "input": {"pattern": "fn main"}},
                {"type": "tool_use", "id": "t4", "name": "UnknownTool", "input": {}}
            ]
        }
    });
    assert_ne!(state.apply_event(&event), Fold::Unchanged);
    let rendered = state.render();
    assert!(rendered.contains("- 读取 `src/main.rs`"), "{rendered}");
    assert!(rendered.contains("- 写入 `a.txt`"), "{rendered}");
    assert!(rendered.contains("- 检索 `fn main`"), "{rendered}");
    assert!(rendered.contains("- `UnknownTool`"), "{rendered}");
}

#[test]
fn render_state_wraps_backtick_in_tool_text() {
    let mut state = ClaudeRenderState::new();
    let event = json!({
        "type": "assistant",
        "message": {
            "content": [
                {"type": "tool_use", "id": "t1", "name": "Bash",
                 "input": {"command": "echo `date`"}}
            ]
        }
    });
    assert_ne!(state.apply_event(&event), Fold::Unchanged);
    let rendered = state.render();
    assert!(rendered.contains("- ``$ echo `date```"), "{rendered}");
}

#[test]
fn build_streaming_card_uses_body_element() {
    let card = lark::card::streaming_card_for_test("hello", true);
    assert_eq!(card["config"]["streaming_mode"], true);
    assert_eq!(
        card["body"]["elements"][0]["element_id"],
        Value::String("body_md".to_string())
    );
    assert_eq!(
        card["body"]["elements"][1]["element_id"],
        Value::String("footer_md".to_string())
    );
}

#[test]
fn session_footer_formats_status_line() {
    assert_eq!(format_elapsed(std::time::Duration::from_secs(33)), "33s");
    assert_eq!(format_elapsed(std::time::Duration::from_secs(125)), "2m");
    assert_eq!(
        format_elapsed(std::time::Duration::from_secs(20 * 3600 + 33 * 60)),
        "20h33m"
    );
    assert_eq!(format_context_percent(Some(98), Some(1000)), "9.80%");
    assert_eq!(format_context_percent(None, Some(1000)), "--");
    assert_eq!(
        format_session_footer("deepseek-v4.1-flash", "9.80%", "20h33m", "notes"),
        "model: deepseek-v4.1-flash · ctx: 9.80% · ⏱ 20h33m · dir: notes"
    );
}

fn stream_usage(event_type: &str, usage: Value) -> Value {
    let event = if event_type == "message_start" {
        json!({ "type": event_type, "message": { "usage": usage } })
    } else {
        json!({ "type": event_type, "usage": usage })
    };
    json!({ "type": "stream_event", "event": event, "parent_tool_use_id": null })
}

fn token_snapshot(input: u64, output: u64, cache_creation: u64, cache_read: u64) -> Value {
    json!({
        "input_tokens": input,
        "output_tokens": output,
        "cache_creation_input_tokens": cache_creation,
        "cache_read_input_tokens": cache_read
    })
}

#[test]
fn token_usage_sums_input_cache_and_output() {
    let mut usage = TokenUsage::default();
    usage.observe(&stream_usage("message_start", token_snapshot(10, 99, 3, 7)));
    assert_eq!(usage.total(), Some(119));
}

#[test]
fn token_usage_ignores_empty_snapshot() {
    let mut usage = TokenUsage::default();
    usage.observe(&stream_usage("message_start", json!({})));
    assert_eq!(usage.total(), None);
}

#[test]
fn token_usage_reads_stream_event_message_usage() {
    let mut usage = TokenUsage::default();
    usage.observe(&stream_usage("message_start", json!({
        "input_tokens": 10,
        "cache_read_input_tokens": 90,
        "output_tokens": 0
    })));
    assert_eq!(usage.total(), Some(100));
}

#[test]
fn token_usage_keeps_input_when_delta_only_has_output() {
    // 与 mock-ollama 的流式样例一致：31 总输入 + 5 输出
    let mut usage = TokenUsage::default();
    usage.observe(&stream_usage("message_start", token_snapshot(20, 0, 3, 8)));
    usage.observe(&stream_usage("message_delta", json!({ "output_tokens": 5 })));
    assert_eq!(usage.total(), Some(36));
    assert_eq!(format_context_percent(usage.total(), Some(200)), "18.00%");
}

#[test]
fn token_usage_merges_partial_input_and_cache_fields() {
    let mut usage = TokenUsage::default();
    usage.observe(&stream_usage("message_start", token_snapshot(20, 5, 3, 8)));
    usage.observe(&stream_usage("message_delta", json!({ "input_tokens": 6 })));
    assert_eq!(usage.total(), Some(22));
    usage.observe(&stream_usage("message_delta", json!({ "cache_read_input_tokens": 12 })));
    assert_eq!(usage.total(), Some(26));
    usage.observe(&stream_usage("message_delta", json!({ "cache_creation_input_tokens": 0 })));
    assert_eq!(usage.total(), Some(23));
}

#[test]
fn token_usage_overwrites_explicit_zero() {
    let mut usage = TokenUsage::default();
    usage.observe(&stream_usage("message_start", token_snapshot(20, 5, 3, 8)));
    usage.observe(&stream_usage("message_delta", token_snapshot(0, 0, 0, 0)));
    assert_eq!(usage.total(), Some(0));
    assert_eq!(format_context_percent(usage.total(), Some(200)), "0.00%");
}

#[test]
fn token_usage_uses_latest_request_not_max() {
    let mut usage = TokenUsage::default();
    usage.observe(&stream_usage("message_start", token_snapshot(50000, 47400, 0, 0)));
    usage.observe(&stream_usage("message_start", token_snapshot(6, 177, 252, 24342)));
    assert_eq!(usage.total(), Some(24777));
}

#[test]
fn token_usage_message_start_clears_previous_output_and_cache() {
    let mut usage = TokenUsage::default();
    usage.observe(&stream_usage("message_start", token_snapshot(20, 5, 3, 8)));
    usage.observe(&stream_usage("message_start", json!({ "input_tokens": 10 })));
    assert_eq!(usage.total(), Some(10));
    usage.observe(&stream_usage("message_delta", json!({ "output_tokens": 2 })));
    assert_eq!(usage.total(), Some(12));
}

#[test]
fn token_usage_new_request_without_usage_clears_previous_request() {
    let mut usage = TokenUsage::default();
    usage.observe(&stream_usage("message_start", token_snapshot(20, 5, 3, 8)));
    usage.observe(&json!({
        "type": "stream_event",
        "event": { "type": "message_start", "message": {} }
    }));
    assert_eq!(usage.total(), None);
    usage.observe(&stream_usage("message_delta", json!({ "output_tokens": 2 })));
    assert_eq!(usage.total(), Some(2));
}

#[test]
fn token_usage_ignores_cumulative_result() {
    let mut usage = TokenUsage::default();
    usage.observe(&stream_usage("message_start", token_snapshot(20, 5, 3, 8)));
    usage.observe(&stream_usage("message_start", token_snapshot(30, 10, 4, 16)));
    usage.observe(&json!({
        "type": "result",
        "usage": token_snapshot(50, 15, 7, 24)
    }));
    assert_eq!(usage.total(), Some(60));
}

#[test]
fn token_usage_result_without_request_stays_unknown() {
    let mut usage = TokenUsage::default();
    usage.observe(&json!({ "type": "result", "usage": token_snapshot(20, 5, 3, 8) }));
    assert_eq!(usage.total(), None);
}

#[test]
fn token_usage_reads_complete_assistant_snapshot() {
    let mut usage = TokenUsage::default();
    usage.observe(&json!({
        "type": "assistant",
        "message": { "usage": token_snapshot(20, 5, 3, 8) },
        "parent_tool_use_id": null
    }));
    assert_eq!(usage.total(), Some(36));
}

#[test]
fn token_usage_assistant_snapshot_replaces_previous_request() {
    let mut usage = TokenUsage::default();
    usage.observe(&stream_usage("message_start", token_snapshot(20, 5, 3, 8)));
    usage.observe(&json!({
        "type": "assistant",
        "message": { "usage": { "input_tokens": 10, "output_tokens": 2 } }
    }));
    assert_eq!(usage.total(), Some(12));
}

#[test]
fn token_usage_repeated_snapshots_do_not_accumulate() {
    let mut usage = TokenUsage::default();
    usage.observe(&stream_usage("message_start", token_snapshot(20, 0, 3, 8)));
    for output in [2, 5, 5, 3] {
        usage.observe(&stream_usage("message_delta", json!({ "output_tokens": output })));
        assert_eq!(usage.total(), Some(31 + output));
    }
    let assistant = json!({
        "type": "assistant",
        "message": { "usage": token_snapshot(20, 3, 3, 8) }
    });
    usage.observe(&assistant);
    usage.observe(&assistant);
    assert_eq!(usage.total(), Some(34));
}

#[test]
fn token_usage_ignores_subagent_events() {
    let mut usage = TokenUsage::default();
    usage.observe(&stream_usage("message_start", token_snapshot(20, 5, 3, 8)));
    for event_type in ["message_start", "message_delta"] {
        let mut event = stream_usage(event_type, token_snapshot(1000, 100, 200, 300));
        event["parent_tool_use_id"] = json!("tool_subagent");
        usage.observe(&event);
        assert_eq!(usage.total(), Some(36));
    }
    usage.observe(&json!({
        "type": "assistant",
        "message": { "usage": token_snapshot(1000, 100, 200, 300) },
        "parent_tool_use_id": "tool_subagent"
    }));
    assert_eq!(usage.total(), Some(36));
}

#[test]
fn token_usage_ignores_invalid_delta_fields() {
    let mut usage = TokenUsage::default();
    usage.observe(&stream_usage("message_start", token_snapshot(20, 5, 3, 8)));
    usage.observe(&stream_usage("message_delta", json!({
        "input_tokens": "100",
        "cache_read_input_tokens": -1,
        "cache_creation_input_tokens": null,
        "output_tokens": 2
    })));
    assert_eq!(usage.total(), Some(33));
}

#[test]
fn token_usage_does_not_double_count_embedded_cached_tokens() {
    let mut usage = TokenUsage::default();
    usage.observe(&json!({
        "type": "assistant",
        "message": { "usage": {
            "input_tokens": 31,
            "input_tokens_details": { "cached_tokens": 8 },
            "output_tokens": 5
        } }
    }));
    assert_eq!(usage.total(), Some(36));
}

#[test]
fn token_usage_saturates_overflowing_external_counts() {
    let mut usage = TokenUsage::default();
    usage.observe(&stream_usage("message_start", token_snapshot(u64::MAX, 5, 3, 8)));
    assert_eq!(usage.total(), Some(u64::MAX));
}

#[test]
fn token_usage_reset_clears_history() {
    let mut usage = TokenUsage::default();
    usage.observe(&stream_usage("message_start", token_snapshot(1000, 47400, 0, 0)));
    usage.reset();
    assert_eq!(usage.total(), None);
    usage.observe(&stream_usage("message_start", token_snapshot(24600, 177, 0, 0)));
    assert_eq!(usage.total(), Some(24777));
}

#[test]
fn render_state_ignores_meta_user_event() {
    // 工具元数据消息（isMeta=true）不写入卡片
    let mut state = ClaudeRenderState::new();
    let event = json!({
        "type": "user",
        "isMeta": true,
        "message": {
            "role": "user",
            "content": [{"type": "text", "text": "工具元数据不应显示"}]
        }
    });
    let fold = state.apply_event(&event);
    assert_eq!(fold, Fold::Unchanged);
    assert!(!state.has_answer());
}

#[test]
fn answer_delta_queues_patch_for_streaming() {
    let mut state = ClaudeRenderState::new();
    let mut ops = VecDeque::new();
    let event = json!({
        "type": "stream_event",
        "event": {
            "type": "content_block_delta",
            "delta": {"type": "text_delta", "text": "你好"}
        }
    });
    let fold = state.apply_event(&event);
    assert_eq!(fold, Fold::Answer);
    assert!(!fold.breaks_pump());
    queue_fold(&mut ops, fold);
    assert_eq!(
        ops.iter().copied().collect::<Vec<_>>(),
        vec![OutboundOp::PatchProgress]
    );
    assert!(state.patch_body().contains("你好"));
}

#[test]
fn thinking_deltas_coalesce_to_one_patch_progress() {
    let mut state = ClaudeRenderState::new();
    let mut ops = VecDeque::new();
    let start = json!({
        "type": "stream_event",
        "event": {
            "type": "content_block_delta",
            "delta": {"type": "thinking_delta", "thinking": "先分析"}
        }
    });
    let more = json!({
        "type": "stream_event",
        "event": {
            "type": "content_block_delta",
            "delta": {"type": "thinking_delta", "thinking": "问题"}
        }
    });
    let first = state.apply_event(&start);
    let second = state.apply_event(&more);
    assert_eq!(first, Fold::Patch);
    assert_eq!(second, Fold::Patch);
    assert!(!first.breaks_pump());
    queue_fold(&mut ops, first);
    queue_fold(&mut ops, second);
    assert_eq!(
        ops.iter().copied().collect::<Vec<_>>(),
        vec![OutboundOp::PatchProgress]
    );
}

#[test]
fn bash_commit_breaks_pump_then_result_closes_before_post() {
    let mut state = ClaudeRenderState::new();
    let mut ops = VecDeque::new();
    let thinking = json!({
        "type": "stream_event",
        "event": {
            "type": "content_block_delta",
            "delta": {"type": "thinking_delta", "thinking": "先"}
        }
    });
    let bash = json!({
        "type": "assistant",
        "message": {
            "content": [{
                "type": "tool_use",
                "id": "t1",
                "name": "Bash",
                "input": {"command": "ls"}
            }]
        }
    });
    queue_fold(&mut ops, state.apply_event(&thinking));
    let fold = state.apply_event(&bash);
    assert_eq!(fold, Fold::Commit);
    assert!(fold.breaks_pump());
    queue_fold(&mut ops, fold);
    queue_result(&mut ops);
    assert_eq!(
        ops.iter().copied().collect::<Vec<_>>(),
        vec![OutboundOp::PatchProgress, OutboundOp::CloseProgress]
    );
}

#[test]
fn queue_stop_drops_post_result() {
    let mut ops = VecDeque::from([OutboundOp::PatchProgress, OutboundOp::CloseProgress]);
    queue_stop(&mut ops);
    assert_eq!(
        ops.iter().copied().collect::<Vec<_>>(),
        vec![OutboundOp::PatchProgress, OutboundOp::CloseProgress]
    );
}

#[test]
fn empty_write_is_not_commit_until_path_arrives() {
    let mut state = ClaudeRenderState::new();
    let empty = json!({
        "type": "stream_event",
        "event": {
            "type": "content_block_start",
            "content_block": {
                "type": "tool_use",
                "id": "t1",
                "name": "Write",
                "input": {}
            }
        }
    });
    assert_eq!(state.apply_event(&empty), Fold::Unchanged);
    assert!(!Fold::Unchanged.breaks_pump());
}

#[test]
fn render_state_waits_for_tool_input_before_listing() {
    let mut state = ClaudeRenderState::new();
    let empty = json!({
        "type": "stream_event",
        "event": {
            "type": "content_block_start",
            "content_block": {
                "type": "tool_use",
                "id": "t1",
                "name": "Write",
                "input": {}
            }
        }
    });
    let ready = json!({
        "type": "assistant",
        "message": {
            "content": [{
                "type": "tool_use",
                "id": "t1",
                "name": "Write",
                "input": { "file_path": "a.txt" }
            }]
        }
    });
    assert_eq!(state.apply_event(&empty), Fold::Unchanged);
    assert!(!state.render().contains("写入"));
    assert_eq!(state.apply_event(&ready), Fold::Commit);
    assert!(
        state.render().contains("- 写入 `a.txt`"),
        "{}",
        state.render()
    );
}

#[test]
fn accept_monotonic_text_updates_on_any_change() {
    // render 单调增长，任何内容变化都应更新；回滚不会真实发生
    let mut current = String::new();
    assert!(lark::card::accept_monotonic_text_for_test(&mut current, "one"));
    assert_eq!(current, "one");
    assert!(lark::card::accept_monotonic_text_for_test(&mut current, "one two"));
    assert_eq!(current, "one two");
    // 中间行增长（如 thinking 就地扩展）也必须更新，不能误判回滚卡死
    assert!(lark::card::accept_monotonic_text_for_test(&mut current, "one two three"));
    assert_eq!(current, "one two three");
    // 内容未变化不重复写入
    assert!(!lark::card::accept_monotonic_text_for_test(&mut current, "one two three"));
}

#[test]
fn card_content_continues_after_unbalanced_fence() {
    // current 存 normalized（不闭合），后续增长仍能更新；闭合围栏由 page_content 负责
    let mut current = String::new();
    assert!(lark::card::accept_monotonic_text_for_test(&mut current, "```text\n代码"));
    assert_eq!(current, "```text\n代码");
    assert!(lark::card::accept_monotonic_text_for_test(&mut current, "```text\n代码\n更多\n```"));
    assert_eq!(current, "```text\n代码\n更多\n```");
}

#[test]
fn current_page_splits_when_over_page_size() {
    // 全量超过 page_start+PAGE_SIZE 触发翻页，当前页只取 PAGE_SIZE 字符
    let text = "x".repeat(10_001);
    let (page, need_split) = lark::card::current_page_for_test(&text, 0);
    assert!(need_split);
    assert_eq!(page.chars().count(), 10_000);
}

#[test]
fn current_page_no_split_within_page() {
    // 全量刚好等于 PAGE_SIZE 不翻页
    let text = "x".repeat(10_000);
    let (page, need_split) = lark::card::current_page_for_test(&text, 0);
    assert!(!need_split);
    assert_eq!(page.chars().count(), 10_000);
}

#[test]
fn current_page_advances_after_split() {
    // 翻页后 page_start 推进，新页从偏移处取剩余
    let text = "x".repeat(20_500);
    let (_, _) = lark::card::current_page_for_test(&text, 0);
    let (next, need_split) = lark::card::current_page_for_test(&text, 10_000);
    assert!(need_split);
    assert_eq!(next.chars().count(), 10_000);
    let (tail, tail_split) = lark::card::current_page_for_test(&text, 20_000);
    assert!(!tail_split);
    assert_eq!(tail.chars().count(), 500);
}

#[test]
fn page_content_reopens_code_block_when_split_inside() {
    // 全量是未闭合的 code block，切片后 page_start 落在 code 内部，开头应补开始围栏
    let text = format!("```python\n{}\n", "代码".repeat(6000));
    let (content, _) = lark::card::page_content_for_test(&text, 10_000);
    assert!(
        content.starts_with("```\n"),
        "开头应补开始围栏，实际: {:?}",
        content.chars().take(20).collect::<String>()
    );
    let fences = content.lines().filter(|l| l.trim().starts_with("```")).count();
    assert_eq!(fences % 2, 0, "围栏应配对");
}

#[test]
fn page_content_keeps_paired_when_not_in_code() {
    // page_start 不在 code 内部，开头不补围栏
    let text = "x".repeat(20_000);
    let (content, _) = lark::card::page_content_for_test(&text, 10_000);
    assert!(!content.starts_with("```"));
}
