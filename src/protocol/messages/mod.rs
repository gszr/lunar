//! Anthropic Messages stream.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;

use serde_json::{Value, json};

use crate::tools;

use super::http::{post_retry, sse_payload, stream_error};
use super::{ChatMessage, Config, StreamEvent, ToolCall, Usage};

const MAX_TOKENS: u32 = 32_768;
const CLAUDE_CODE_IDENTITY: &str = "You are Claude Code, Anthropic's official CLI for Claude.";

pub(super) fn stream(
    cfg: Config,
    messages: Vec<ChatMessage>,
    cancel: Arc<AtomicBool>,
    tx: &Sender<StreamEvent>,
    tools: bool,
    debug_headers: bool,
) -> Result<(), String> {
    let url = format!("{}/v1/messages", cfg.base_url.trim_end_matches('/'));
    let body = body(&cfg, &messages, tools);
    let response = post_retry(
        &url,
        &cfg,
        &body,
        &cancel,
        None,
        None,
        debug_headers.then_some(tx),
    )?;
    let oauth = is_oauth(&cfg);
    let mut calls: BTreeMap<u64, ToolCall> = BTreeMap::new();
    let mut usage = None;
    let mut truncated = false;
    let reader = BufReader::new(response.into_parts().1.into_reader());
    for line in reader.lines() {
        if cancel.load(Ordering::Relaxed) {
            let _ = tx.send(StreamEvent::Failed("aborted".into()));
            return Ok(());
        }
        let line = line.map_err(|err| stream_error(&cfg.provider, err))?;
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
        apply_event(&value, tx, &mut calls, &mut usage, &mut truncated, oauth);
        if value.get("type").and_then(Value::as_str) == Some("message_stop") {
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
    finish(tx, calls.into_values().collect(), truncated, tools);
    Ok(())
}

fn finish(tx: &Sender<StreamEvent>, calls: Vec<ToolCall>, truncated: bool, tools: bool) {
    if !tools && (truncated || !calls.is_empty()) {
        let message = if truncated {
            "generation hit the token cap"
        } else {
            "model attempted to call a tool"
        };
        let _ = tx.send(StreamEvent::Failed(message.into()));
    } else if calls.is_empty() {
        let _ = tx.send(if tools {
            StreamEvent::Done
        } else {
            StreamEvent::CompactDone
        });
    } else {
        let _ = tx.send(StreamEvent::Tools { calls, truncated });
    }
}

fn apply_event(
    value: &Value,
    tx: &Sender<StreamEvent>,
    calls: &mut BTreeMap<u64, ToolCall>,
    usage: &mut Option<Usage>,
    truncated: &mut bool,
    oauth: bool,
) {
    match value.get("type").and_then(Value::as_str).unwrap_or("") {
        "message_start" => {
            if let Some(model) = value.pointer("/message/model").and_then(Value::as_str) {
                let _ = tx.send(StreamEvent::Model(model.to_string()));
            }
            if let Some(parsed) = parse_messages_usage(value.pointer("/message/usage")) {
                *usage = Some(parsed);
            }
        }
        "content_block_start" => start_block(value, tx, calls, oauth),
        "content_block_delta" => delta_block(value, tx, calls),
        "message_delta" => {
            if value.pointer("/delta/stop_reason").and_then(Value::as_str) == Some("max_tokens") {
                *truncated = true;
            }
            if let Some(parsed) = parse_messages_usage(value.get("usage")) {
                merge_usage(usage, parsed);
            }
        }
        _ => {}
    }
}

fn start_block(
    value: &Value,
    tx: &Sender<StreamEvent>,
    calls: &mut BTreeMap<u64, ToolCall>,
    oauth: bool,
) {
    let index = value.get("index").and_then(Value::as_u64).unwrap_or(0);
    let block = value.get("content_block");
    match block.and_then(|b| b.get("type")).and_then(Value::as_str) {
        Some("tool_use") => {
            calls.insert(
                index,
                ToolCall {
                    id: block
                        .and_then(|b| b.get("id"))
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .into(),
                    name: local_tool_name(
                        block
                            .and_then(|b| b.get("name"))
                            .and_then(Value::as_str)
                            .unwrap_or(""),
                        oauth,
                    ),
                    arguments: String::new(),
                },
            );
        }
        Some("thinking") => {
            send_text(tx, block, "thinking", true);
            send_signature(tx, block.and_then(|b| b.get("signature")));
        }
        Some("redacted_thinking") => send_signature(tx, block.and_then(|b| b.get("data"))),
        Some("text") => send_text(tx, block, "text", false),
        _ => {}
    }
}

fn delta_block(value: &Value, tx: &Sender<StreamEvent>, calls: &mut BTreeMap<u64, ToolCall>) {
    let index = value.get("index").and_then(Value::as_u64).unwrap_or(0);
    let delta = value.get("delta");
    match delta.and_then(|d| d.get("type")).and_then(Value::as_str) {
        Some("text_delta") => send_text(tx, delta, "text", false),
        Some("thinking_delta") => send_text(tx, delta, "thinking", true),
        Some("signature_delta") => send_signature(tx, delta.and_then(|d| d.get("signature"))),
        Some("input_json_delta") => {
            if let Some(partial) = delta
                .and_then(|d| d.get("partial_json"))
                .and_then(Value::as_str)
            {
                calls
                    .entry(index)
                    .or_insert_with(|| ToolCall {
                        id: format!("call_{index}"),
                        name: String::new(),
                        arguments: String::new(),
                    })
                    .arguments
                    .push_str(partial);
            }
        }
        _ => {}
    }
}

fn send_signature(tx: &Sender<StreamEvent>, value: Option<&Value>) {
    if let Some(signature) = value.and_then(Value::as_str)
        && !signature.is_empty()
    {
        let _ = tx.send(StreamEvent::ThinkSignature(signature.into()));
    }
}

fn send_text(tx: &Sender<StreamEvent>, value: Option<&Value>, key: &str, think: bool) {
    if let Some(text) = value.and_then(|v| v.get(key)).and_then(Value::as_str)
        && !text.is_empty()
    {
        let _ = tx.send(if think {
            StreamEvent::Think(text.into())
        } else {
            StreamEvent::Delta(text.into())
        });
    }
}

fn body(cfg: &Config, messages: &[ChatMessage], include_tools: bool) -> String {
    let oauth = is_oauth(cfg);
    let mut value = json!({
        "model": cfg.model,
        "max_tokens": MAX_TOKENS,
        "stream": true,
        "messages": to_messages(messages, oauth),
    });
    if include_tools {
        value["tools"] = tools::messages_definitions(oauth);
    }
    if oauth {
        value["system"] = json!([{"type": "text", "text": CLAUDE_CODE_IDENTITY}]);
    }
    if cfg.thinking != "off" {
        value["thinking"] = json!({"type": "adaptive", "display": "summarized"});
        value["output_config"] = json!({ "effort": &cfg.thinking });
    }
    value.to_string()
}

fn to_messages(messages: &[ChatMessage], oauth: bool) -> Vec<Value> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < messages.len() {
        match &messages[i] {
            ChatMessage::User(content) => {
                out.push(json!({"role": "user", "content": content}));
                i += 1;
            }
            ChatMessage::Assistant {
                content,
                thinking,
                thinking_signature,
                tool_calls,
            } => {
                let mut blocks = Vec::new();
                if !thinking_signature.is_empty() {
                    blocks.push(json!({
                        "type": "thinking",
                        "thinking": thinking,
                        "signature": thinking_signature,
                    }));
                } else if !thinking.is_empty() {
                    blocks.push(json!({"type": "text", "text": thinking}));
                }
                if !content.is_empty() {
                    blocks.push(json!({"type": "text", "text": content}));
                }
                for call in tool_calls {
                    let input = serde_json::from_str(&call.arguments)
                        .unwrap_or_else(|_| json!({"raw": call.arguments}));
                    blocks.push(json!({
                        "type": "tool_use",
                        "id": call.id,
                        "name": wire_tool_name(&call.name, oauth),
                        "input": input,
                    }));
                }
                if !blocks.is_empty() {
                    out.push(json!({"role": "assistant", "content": blocks}));
                }
                i += 1;
            }
            ChatMessage::Tool { .. } => {
                let mut results = Vec::new();
                while i < messages.len() {
                    let ChatMessage::Tool { id, content } = &messages[i] else {
                        break;
                    };
                    results.push(json!({
                        "type": "tool_result",
                        "tool_use_id": id,
                        "content": content,
                    }));
                    i += 1;
                }
                if !results.is_empty() {
                    out.push(json!({"role": "user", "content": results}));
                }
            }
        }
    }
    out
}

fn parse_messages_usage(usage: Option<&Value>) -> Option<Usage> {
    let usage = usage.filter(|value| !value.is_null())?;
    let num = |key: &str| usage.get(key).and_then(Value::as_u64).unwrap_or(0) as u32;
    Some(Usage {
        input: num("input_tokens"),
        output: num("output_tokens"),
        cache_read: num("cache_read_input_tokens"),
        cache_write: num("cache_creation_input_tokens"),
    })
}

fn merge_usage(usage: &mut Option<Usage>, parsed: Usage) {
    match usage {
        Some(existing) => {
            if parsed.input != 0 {
                existing.input = parsed.input;
            }
            if parsed.output != 0 {
                existing.output = parsed.output;
            }
            if parsed.cache_read != 0 {
                existing.cache_read = parsed.cache_read;
            }
            if parsed.cache_write != 0 {
                existing.cache_write = parsed.cache_write;
            }
        }
        None => *usage = Some(parsed),
    }
}

fn is_oauth(cfg: &Config) -> bool {
    cfg.auth_provider.as_deref() == Some("anthropic") || cfg.api_key.contains("sk-ant-oat")
}

fn wire_tool_name(name: &str, oauth: bool) -> &str {
    if !oauth {
        return name;
    }
    match name {
        "read" => "Read",
        "write" => "Write",
        "edit" => "Edit",
        "bash" => "Bash",
        other => other,
    }
}

fn local_tool_name(name: &str, oauth: bool) -> String {
    if !oauth {
        return name.to_string();
    }
    match name {
        "Read" => "read".into(),
        "Write" => "write".into(),
        "Edit" => "edit".into(),
        "Bash" => "bash".into(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests;
