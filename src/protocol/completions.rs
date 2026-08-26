//! Chat Completions stream.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;

use serde_json::{Value, json};

use crate::tools;

use super::http::{collect_tail, parse_usage, post_retry};
use super::{ChatMessage, Config, RequestAudit, StreamEvent, ToolCall};

/// Hard cap on reasoning + answer.
pub const MAX_TOKENS: u32 = 32_768;

impl ChatMessage {
    fn to_completions(&self) -> Value {
        match self {
            ChatMessage::User(content) => json!({"role": "user", "content": content}),
            ChatMessage::Assistant {
                content,
                tool_calls,
            } if tool_calls.is_empty() => json!({"role": "assistant", "content": content}),
            ChatMessage::Assistant {
                content,
                tool_calls,
            } => json!({
                "role": "assistant",
                "content": if content.is_empty() { Value::Null } else { Value::String(content.clone()) },
                "tool_calls": tool_calls.iter().map(|c| json!({
                    "id": c.id,
                    "type": "function",
                    "function": { "name": c.name, "arguments": c.arguments },
                })).collect::<Vec<_>>(),
            }),
            ChatMessage::Tool { id, content } => {
                json!({"role": "tool", "tool_call_id": id, "content": content})
            }
        }
    }
}

pub(super) fn stream(
    cfg: Config,
    messages: Vec<ChatMessage>,
    cancel: Arc<AtomicBool>,
    tx: &Sender<StreamEvent>,
) -> Result<(), String> {
    let url = format!("{}/chat/completions", cfg.base_url.trim_end_matches('/'));
    let body = body(&cfg, &messages);
    let audit = RequestAudit {
        provider: cfg.provider.clone(),
        model: cfg.model.clone(),
        api: cfg.api,
        url: url.clone(),
        input_items: messages.len(),
        input_bytes: body.len(),
    };

    let response = post_retry(&url, &cfg.api_key, &body, &cancel, None, None, (&audit, tx))?;

    let mut calls: BTreeMap<u64, ToolCall> = BTreeMap::new();
    let mut usage = None;
    let mut saw_done = false;
    let mut truncated = false;
    let mut reader = BufReader::new(response.into_parts().1.into_reader());
    for line in reader.by_ref().lines() {
        if cancel.load(Ordering::Relaxed) {
            let _ = tx.send(StreamEvent::Failed("aborted".into()));
            return Ok(());
        }
        let line = line.map_err(|e| e.to_string())?;
        crate::debug::event("response", json!({ "line": &line }));
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data.is_empty() {
            continue;
        }
        if data == "[DONE]" {
            saw_done = true;
            break;
        }
        let value: Value = serde_json::from_str(data).map_err(|e| e.to_string())?;
        if let Some(parsed) = parse_usage(&value) {
            usage = Some(parsed);
        }
        if let Some(err) = value.get("error") {
            let msg = err
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("request failed");
            return Err(msg.to_string());
        }
        if let Some(text) = value
            .pointer("/choices/0/delta/content")
            .and_then(Value::as_str)
            && !text.is_empty()
        {
            let _ = tx.send(StreamEvent::Delta(text.to_string()));
        }
        for field in ["reasoning_content", "reasoning", "reasoning_text"] {
            if let Some(text) = value
                .pointer(&format!("/choices/0/delta/{field}"))
                .and_then(Value::as_str)
                && !text.is_empty()
            {
                let _ = tx.send(StreamEvent::Think(text.to_string()));
                break;
            }
        }
        if let Some(chunks) = value
            .pointer("/choices/0/delta/tool_calls")
            .and_then(Value::as_array)
        {
            for chunk in chunks {
                let index = chunk.get("index").and_then(Value::as_u64).unwrap_or(0);
                let entry = calls.entry(index).or_insert_with(|| ToolCall {
                    id: String::new(),
                    name: String::new(),
                    arguments: String::new(),
                });
                if let Some(id) = chunk.get("id").and_then(Value::as_str) {
                    entry.id = id.to_string();
                }
                if let Some(name) = chunk.pointer("/function/name").and_then(Value::as_str) {
                    entry.name.push_str(name);
                }
                if let Some(args) = chunk.pointer("/function/arguments").and_then(Value::as_str) {
                    entry.arguments.push_str(args);
                }
            }
        }
        // xAI (and some proxies) may keep the socket open after the last
        // choice, or skip [DONE]. finish_reason is the real end of the turn.
        if let Some(reason) = value
            .pointer("/choices/0/finish_reason")
            .and_then(Value::as_str)
        {
            truncated = reason == "length" || reason == "max_tokens";
            break;
        }
    }

    if cancel.load(Ordering::Relaxed) {
        let _ = tx.send(StreamEvent::Failed("aborted".into()));
        return Ok(());
    }
    if !saw_done {
        collect_tail(reader, &mut usage, &mut saw_done, &cancel);
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

fn body(cfg: &Config, messages: &[ChatMessage]) -> String {
    let mut body = json!({
        "model": cfg.model,
        "stream": true,
        "stream_options": { "include_usage": true },
        "tools": tools::completions_definitions(),
        "messages": messages.iter().map(ChatMessage::to_completions).collect::<Vec<_>>(),
    });
    if is_openai_url(&cfg.base_url) {
        body["max_completion_tokens"] = json!(MAX_TOKENS);
    } else {
        body["max_tokens"] = json!(MAX_TOKENS);
    }
    if cfg.thinking != super::Thinking::Off {
        body["reasoning_effort"] = json!(cfg.thinking.as_str());
    }
    body.to_string()
}

fn is_openai_url(base_url: &str) -> bool {
    base_url
        .split("://")
        .nth(1)
        .unwrap_or(base_url)
        .split('/')
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .is_some_and(|host| host.eq_ignore_ascii_case("api.openai.com"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::Api;

    #[test]
    fn request_caps_completion_tokens() {
        let cfg = Config {
            api_key: "k".into(),
            base_url: "https://api.x.ai/v1".into(),
            model: "grok-4.6".into(),
            provider: "xai".into(),
            window: Some(500_000),
            api: Api::Completions,
            auth_provider: None,
            thinking: super::super::Thinking::Off,
        };
        let body: Value = serde_json::from_str(&body(&cfg, &[])).unwrap();
        assert_eq!(body["max_tokens"], MAX_TOKENS);
        assert_eq!(body["stream"], true);
        assert_eq!(body["model"], "grok-4.6");
    }

    #[test]
    fn configured_thinking_sets_reasoning_effort() {
        let cfg = Config {
            api_key: "k".into(),
            base_url: "https://api.x.ai/v1".into(),
            model: "grok-4.6".into(),
            provider: "xai".into(),
            window: None,
            api: Api::Completions,
            auth_provider: None,
            thinking: super::super::Thinking::High,
        };
        let parsed: Value = serde_json::from_str(&body(&cfg, &[])).unwrap();
        assert_eq!(parsed["reasoning_effort"], "high");
    }

    #[test]
    fn openai_uses_max_completion_tokens() {
        let cfg = Config {
            api_key: "k".into(),
            base_url: "https://API.OPENAI.COM:443/v1/".into(),
            model: "gpt-5.6-sol".into(),
            provider: "anything".into(),
            window: None,
            api: Api::Completions,
            auth_provider: None,
            thinking: super::super::Thinking::Off,
        };
        let parsed: Value = serde_json::from_str(&body(&cfg, &[])).unwrap();
        assert_eq!(parsed["max_completion_tokens"], MAX_TOKENS);
        assert!(parsed.get("reasoning_effort").is_none());
        assert!(parsed.get("max_tokens").is_none());
    }
}
