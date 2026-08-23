//! OpenAI Responses stream.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;

use serde_json::{Value, json};

use crate::tools;

use super::http::{parse_usage, post_retry, sse_payload};
use super::{ChatMessage, Config, StreamEvent, ToolCall};

impl ChatMessage {
    fn to_responses(&self, msg_index: usize) -> Value {
        match self {
            ChatMessage::User(content) => json!({
                "role": "user",
                "content": [{ "type": "input_text", "text": content }],
            }),
            ChatMessage::Assistant {
                content,
                tool_calls,
            } => {
                let mut items = Vec::new();
                if !content.is_empty() {
                    items.push(json!({
                        "type": "message",
                        "role": "assistant",
                        "content": [{
                            "type": "output_text",
                            "text": content,
                            "annotations": [],
                        }],
                        "status": "completed",
                        "id": format!("msg_lunar_{msg_index}"),
                    }));
                }
                for call in tool_calls {
                    items.push(json!({
                        "type": "function_call",
                        "call_id": call.id,
                        "name": call.name,
                        "arguments": call.arguments,
                    }));
                }
                json!(items)
            }
            ChatMessage::Tool { id, content } => json!({
                "type": "function_call_output",
                "call_id": id,
                "output": content,
            }),
        }
    }
}

fn flatten_input(messages: &[ChatMessage]) -> Vec<Value> {
    let mut out = Vec::new();
    for (i, message) in messages.iter().enumerate() {
        match message.to_responses(i) {
            Value::Array(items) => out.extend(items),
            other => out.push(other),
        }
    }
    out
}

pub(super) fn stream(
    cfg: Config,
    messages: Vec<ChatMessage>,
    cancel: Arc<AtomicBool>,
    tx: &Sender<StreamEvent>,
    cache_key: Option<String>,
) -> Result<(), String> {
    let url = responses_url(&cfg);
    let cache_key = cache_key
        .as_deref()
        .map(clamp_cache_key)
        .filter(|key| !key.is_empty());
    let body = body(&cfg, &messages, cache_key.as_deref());
    let account = if cfg.auth_provider.as_deref() == Some("openai") {
        Some(crate::auth::chatgpt_account_id(&cfg.api_key)?)
    } else {
        None
    };
    let response = post_retry(
        &url,
        &cfg.api_key,
        &body,
        &cancel,
        cache_key.as_deref(),
        account.as_deref(),
    )?;

    let mut calls: BTreeMap<u64, ToolCall> = BTreeMap::new();
    let mut usage = None;
    let mut truncated = false;
    let reader = BufReader::new(response.into_parts().1.into_reader());
    for line in reader.lines() {
        if cancel.load(Ordering::Relaxed) {
            let _ = tx.send(StreamEvent::Failed("aborted".into()));
            return Ok(());
        }
        let line = line.map_err(|e| e.to_string())?;
        crate::debug::event("response", json!({ "line": &line }));
        let Some(data) = sse_payload(&line) else {
            continue;
        };
        let value: Value = serde_json::from_str(data).map_err(|e| e.to_string())?;
        if let Some(err) = value.get("error").filter(|v| !v.is_null()) {
            let msg = err
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("request failed");
            return Err(msg.to_string());
        }
        if let Some(parsed) =
            parse_usage(&value).or_else(|| value.get("response").and_then(parse_usage))
        {
            usage = Some(parsed);
        }
        apply_event(&value, tx, &mut calls, &mut truncated);
        if finished(&value) {
            break;
        }
    }

    if cancel.load(Ordering::Relaxed) {
        let _ = tx.send(StreamEvent::Failed("aborted".into()));
        return Ok(());
    }
    if let Some(usage) = usage {
        let _ = tx.send(StreamEvent::Usage(usage));
    }
    let calls: Vec<ToolCall> = calls
        .into_iter()
        .map(|(index, mut call)| {
            if call.id.is_empty() {
                call.id = format!("call_{index}");
            }
            call
        })
        .collect();
    if calls.is_empty() {
        let _ = tx.send(StreamEvent::Done);
    } else {
        let _ = tx.send(StreamEvent::Tools { calls, truncated });
    }
    Ok(())
}

fn responses_url(cfg: &Config) -> String {
    let base = cfg.base_url.trim_end_matches('/');
    if cfg.auth_provider.as_deref() == Some("openai") {
        format!("{base}/codex/responses")
    } else {
        format!("{base}/responses")
    }
}

fn clamp_cache_key(key: &str) -> String {
    key.chars().take(64).collect()
}

fn body(cfg: &Config, messages: &[ChatMessage], cache_key: Option<&str>) -> String {
    let mut value = json!({
        "model": cfg.model,
        "stream": true,
        "store": false,
        "tools": tools::responses_definitions(),
        "input": flatten_input(messages),
    });
    if let Some(key) = cache_key {
        value["prompt_cache_key"] = json!(key);
    }
    value.to_string()
}

fn apply_event(
    value: &Value,
    tx: &Sender<StreamEvent>,
    calls: &mut BTreeMap<u64, ToolCall>,
    truncated: &mut bool,
) {
    let event = value.get("type").and_then(Value::as_str).unwrap_or("");
    match event {
        "response.output_text.delta" => {
            if let Some(text) = value.get("delta").and_then(Value::as_str)
                && !text.is_empty()
            {
                let _ = tx.send(StreamEvent::Delta(text.to_string()));
            }
        }
        "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
            if let Some(text) = value.get("delta").and_then(Value::as_str)
                && !text.is_empty()
            {
                let _ = tx.send(StreamEvent::Think(text.to_string()));
            }
        }
        "response.output_item.added" => {
            let Some(item) = value.get("item") else {
                return;
            };
            if item.get("type").and_then(Value::as_str) != Some("function_call") {
                return;
            }
            let index = value
                .get("output_index")
                .and_then(Value::as_u64)
                .unwrap_or(calls.len() as u64);
            let entry = calls.entry(index).or_insert_with(|| ToolCall {
                id: String::new(),
                name: String::new(),
                arguments: String::new(),
            });
            if let Some(id) = item
                .get("call_id")
                .or_else(|| item.get("id"))
                .and_then(Value::as_str)
            {
                entry.id = id.to_string();
            }
            if let Some(name) = item.get("name").and_then(Value::as_str) {
                entry.name = name.to_string();
            }
            if let Some(args) = item.get("arguments").and_then(Value::as_str) {
                entry.arguments = args.to_string();
            }
        }
        "response.function_call_arguments.delta" => {
            let index = value
                .get("output_index")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let entry = calls.entry(index).or_insert_with(|| ToolCall {
                id: String::new(),
                name: String::new(),
                arguments: String::new(),
            });
            if let Some(id) = value
                .get("call_id")
                .or_else(|| value.get("item_id"))
                .and_then(Value::as_str)
                && entry.id.is_empty()
            {
                entry.id = id.to_string();
            }
            if let Some(delta) = value.get("delta").and_then(Value::as_str) {
                entry.arguments.push_str(delta);
            }
        }
        "response.function_call_arguments.done" => {
            let index = value
                .get("output_index")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let entry = calls.entry(index).or_insert_with(|| ToolCall {
                id: String::new(),
                name: String::new(),
                arguments: String::new(),
            });
            if let Some(id) = value.get("call_id").and_then(Value::as_str) {
                entry.id = id.to_string();
            } else if entry.id.is_empty()
                && let Some(id) = value.get("item_id").and_then(Value::as_str)
            {
                entry.id = id.to_string();
            }
            if let Some(args) = value.get("arguments").and_then(Value::as_str) {
                entry.arguments = args.to_string();
            }
        }
        "response.completed" | "response.incomplete" => {
            if let Some(status) = value.pointer("/response/status").and_then(Value::as_str) {
                *truncated = status == "incomplete";
            }
            if event == "response.incomplete" {
                *truncated = true;
            }
        }
        _ => {}
    }
}

fn finished(value: &Value) -> bool {
    matches!(
        value.get("type").and_then(Value::as_str),
        Some("response.completed" | "response.incomplete" | "response.failed")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::Api;
    use std::sync::mpsc;

    fn sample_cfg() -> Config {
        Config {
            api_key: "k".into(),
            base_url: "https://api.openai.com/v1".into(),
            model: "gpt-5".into(),
            provider: "openai".into(),
            window: None,
            api: Api::Responses,
            auth_provider: None,
        }
    }

    #[test]
    fn plus_posts_codex_responses() {
        let mut cfg = sample_cfg();
        cfg.auth_provider = Some("openai".into());
        cfg.base_url = "https://chatgpt.com/backend-api".into();
        assert_eq!(
            responses_url(&cfg),
            "https://chatgpt.com/backend-api/codex/responses"
        );
        cfg.auth_provider = None;
        assert_eq!(
            responses_url(&cfg),
            "https://chatgpt.com/backend-api/responses"
        );
    }

    #[test]
    fn body_uses_input_and_flat_tools() {
        let messages = vec![
            ChatMessage::User("hi".into()),
            ChatMessage::Assistant {
                content: "calling".into(),
                tool_calls: vec![ToolCall {
                    id: "call_1".into(),
                    name: "read".into(),
                    arguments: "{\"path\":\"a\"}".into(),
                }],
            },
            ChatMessage::Tool {
                id: "call_1".into(),
                content: "ok".into(),
            },
        ];
        let parsed: Value =
            serde_json::from_str(&body(&sample_cfg(), &messages, Some("2026-08-22-1"))).unwrap();
        assert_eq!(parsed["model"], "gpt-5");
        assert_eq!(parsed["stream"], true);
        assert_eq!(parsed["store"], false);
        assert_eq!(parsed["prompt_cache_key"], "2026-08-22-1");
        assert!(parsed.get("max_output_tokens").is_none());
        assert!(parsed.get("messages").is_none());
        assert_eq!(parsed["tools"][0]["type"], "function");
        assert_eq!(parsed["tools"][0]["name"], "read");
        assert!(parsed["tools"][0].get("function").is_none());
        assert_eq!(parsed["input"].as_array().unwrap().len(), 4);
        assert!(parsed["input"][0].get("type").is_none());
        assert_eq!(parsed["input"][0]["role"], "user");
        assert_eq!(parsed["input"][0]["content"][0]["type"], "input_text");
        assert_eq!(parsed["input"][1]["type"], "message");
        assert_eq!(parsed["input"][1]["role"], "assistant");
        assert_eq!(parsed["input"][1]["status"], "completed");
        assert_eq!(parsed["input"][1]["id"], "msg_lunar_1");
        assert_eq!(parsed["input"][1]["content"][0]["annotations"], json!([]));
        assert_eq!(parsed["input"][2]["type"], "function_call");
        assert_eq!(parsed["input"][2]["call_id"], "call_1");
        assert!(parsed["input"][2].get("id").is_none());
        assert_eq!(parsed["input"][3]["type"], "function_call_output");
        assert_eq!(parsed["input"][3]["call_id"], "call_1");
    }

    #[test]
    fn body_omits_empty_cache_key_and_clamps_long_ones() {
        let messages = vec![ChatMessage::User("hi".into())];
        let none: Value = serde_json::from_str(&body(&sample_cfg(), &messages, None)).unwrap();
        assert!(none.get("prompt_cache_key").is_none());
        let long = "m".repeat(80);
        let clamped: Value = serde_json::from_str(&body(
            &sample_cfg(),
            &messages,
            Some(&clamp_cache_key(&long)),
        ))
        .unwrap();
        assert_eq!(
            clamped["prompt_cache_key"]
                .as_str()
                .unwrap()
                .chars()
                .count(),
            64
        );
    }

    #[test]
    fn text_delta_and_function_call() {
        let (tx, rx) = mpsc::channel();
        let mut calls = BTreeMap::new();
        let mut truncated = false;
        apply_event(
            &json!({"type":"response.output_text.delta","delta":"hello"}),
            &tx,
            &mut calls,
            &mut truncated,
        );
        apply_event(
            &json!({
                "type": "response.output_item.added",
                "output_index": 1,
                "item": {
                    "type": "function_call",
                    "call_id": "call_1",
                    "name": "read",
                    "arguments": ""
                }
            }),
            &tx,
            &mut calls,
            &mut truncated,
        );
        apply_event(
            &json!({
                "type": "response.function_call_arguments.delta",
                "output_index": 1,
                "delta": "{\"path\":"
            }),
            &tx,
            &mut calls,
            &mut truncated,
        );
        apply_event(
            &json!({
                "type": "response.function_call_arguments.done",
                "output_index": 1,
                "item_id": "fc_1",
                "arguments": "{\"path\":\"a\"}"
            }),
            &tx,
            &mut calls,
            &mut truncated,
        );
        apply_event(
            &json!({"type":"response.incomplete","response":{"status":"incomplete"}}),
            &tx,
            &mut calls,
            &mut truncated,
        );
        drop(tx);
        let events: Vec<_> = rx.iter().collect();
        assert!(matches!(&events[0], StreamEvent::Delta(text) if text == "hello"));
        assert_eq!(calls[&1].id, "call_1");
        assert_eq!(calls[&1].name, "read");
        assert_eq!(calls[&1].arguments, "{\"path\":\"a\"}");
        assert!(truncated);
        assert!(finished(&json!({"type":"response.completed"})));
    }
}
