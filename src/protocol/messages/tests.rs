use super::*;
use crate::protocol::Api;

fn sample_cfg() -> Config {
    Config {
        api_key: "sk-ant-api".into(),
        base_url: "https://api.anthropic.com".into(),
        model: "claude-opus-4".into(),
        provider: "anthropic".into(),
        window: None,
        api: Api::Messages,
        auth_provider: None,
        thinking: "off".into(),
        thinking_levels: vec!["off".into(), "low".into(), "high".into()],
    }
}

#[test]
fn api_key_body_omits_claude_code_identity() {
    let cfg = sample_cfg();
    let body: Value = serde_json::from_str(&body(&cfg, &[], true)).unwrap();
    assert_eq!(body["model"], "claude-opus-4");
    assert_eq!(body["max_tokens"], MAX_TOKENS);
    assert_eq!(body["stream"], true);
    assert!(body.get("system").is_none());
    assert!(body.get("betas").is_none());
    assert_eq!(body["tools"][0]["name"], "read");
    assert!(body["tools"][0].get("input_schema").is_some());
}

#[test]
fn oauth_body_uses_claude_code_identity_and_tool_names() {
    let mut cfg = sample_cfg();
    cfg.auth_provider = Some("anthropic".into());
    cfg.api_key = "sk-ant-oat-test".into();
    cfg.thinking = "high".into();
    let messages = vec![
        ChatMessage::User("hello".into()),
        ChatMessage::Assistant {
            content: "checking".into(),
            thinking: "hmm".into(),
            thinking_signature: "sig".into(),
            tool_calls: vec![ToolCall {
                id: "toolu_1".into(),
                name: "read".into(),
                arguments: r#"{"path":"CONTEXT.md"}"#.into(),
            }],
        },
        ChatMessage::Tool {
            id: "toolu_1".into(),
            content: "ok".into(),
        },
    ];
    let body: Value = serde_json::from_str(&body(&cfg, &messages, true)).unwrap();
    assert_eq!(body["system"][0]["text"], CLAUDE_CODE_IDENTITY);
    assert!(body.get("betas").is_none());
    assert_eq!(body["tools"][0]["name"], "Read");
    assert_eq!(body["thinking"]["type"], "adaptive");
    assert_eq!(body["output_config"]["effort"], "high");
    assert_eq!(body["messages"][1]["content"][0]["type"], "thinking");
    assert_eq!(body["messages"][1]["content"][0]["signature"], "sig");
    assert_eq!(body["messages"][1]["content"][2]["name"], "Read");
    assert_eq!(body["messages"][2]["content"][0]["tool_use_id"], "toolu_1");
}

#[test]
fn text_thinking_and_tool_deltas() {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut calls = BTreeMap::new();
    let mut usage = None;
    let mut truncated = false;
    apply_event(
        &json!({
            "type": "message_start",
            "message": {
                "model": "claude-opus-4",
                "usage": {
                    "input_tokens": 10,
                    "output_tokens": 0,
                    "cache_read_input_tokens": 4,
                    "cache_creation_input_tokens": 2
                }
            }
        }),
        &tx,
        &mut calls,
        &mut usage,
        &mut truncated,
        true,
    );
    apply_event(
        &json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": { "type": "thinking_delta", "thinking": "hmm" }
        }),
        &tx,
        &mut calls,
        &mut usage,
        &mut truncated,
        true,
    );
    apply_event(
        &json!({
            "type": "content_block_delta",
            "index": 1,
            "delta": { "type": "text_delta", "text": "hi" }
        }),
        &tx,
        &mut calls,
        &mut usage,
        &mut truncated,
        true,
    );
    apply_event(
        &json!({
            "type": "content_block_start",
            "index": 2,
            "content_block": { "type": "tool_use", "id": "toolu_1", "name": "Read" }
        }),
        &tx,
        &mut calls,
        &mut usage,
        &mut truncated,
        true,
    );
    apply_event(
        &json!({
            "type": "content_block_delta",
            "index": 2,
            "delta": { "type": "input_json_delta", "partial_json": "{\"path\":" }
        }),
        &tx,
        &mut calls,
        &mut usage,
        &mut truncated,
        true,
    );
    drop(tx);
    let events: Vec<_> = rx.into_iter().collect();
    assert!(
        events
            .iter()
            .any(|event| matches!(event, StreamEvent::Model(model) if model == "claude-opus-4"))
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, StreamEvent::Think(text) if text == "hmm"))
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, StreamEvent::Delta(text) if text == "hi"))
    );
    assert_eq!(calls[&2].name, "read");
    assert_eq!(calls[&2].arguments, "{\"path\":");
    let usage = usage.unwrap();
    assert_eq!(usage.input, 10);
    assert_eq!(usage.cache_read, 4);
    assert_eq!(usage.cache_write, 2);
}
