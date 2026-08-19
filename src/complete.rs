//! Chat Completions stream. Generic OpenAI-shaped HTTP. No branded providers.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use serde_json::{Value, json};
use ureq::Agent;

use crate::tools;

#[derive(Clone)]
pub struct Config {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
}

impl Config {
    pub fn from_env() -> Option<Self> {
        Some(Self {
            api_key: nonempty("LUNAR_API_KEY")?,
            base_url: nonempty("LUNAR_BASE_URL")?,
            model: nonempty("LUNAR_MODEL")?,
        })
    }
}

#[derive(Clone)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

pub enum ChatMessage {
    User(String),
    Assistant {
        content: String,
        tool_calls: Vec<ToolCall>,
    },
    Tool {
        id: String,
        content: String,
    },
}

impl ChatMessage {
    fn to_json(&self) -> Value {
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

pub struct ToolResult {
    pub id: String,
    pub title: String,
    pub content: String,
}

pub enum StreamEvent {
    Delta(String),
    Tools(Vec<ToolCall>),
    ToolResults(Vec<ToolResult>),
    Done,
    Failed(String),
}

pub fn stream(
    cfg: Config,
    messages: Vec<ChatMessage>,
    cancel: Arc<AtomicBool>,
    tx: Sender<StreamEvent>,
) {
    if let Err(err) = stream_inner(cfg, messages, cancel, &tx) {
        let _ = tx.send(StreamEvent::Failed(err));
    }
}

fn stream_inner(
    cfg: Config,
    messages: Vec<ChatMessage>,
    cancel: Arc<AtomicBool>,
    tx: &Sender<StreamEvent>,
) -> Result<(), String> {
    let url = format!("{}/chat/completions", cfg.base_url.trim_end_matches('/'));
    let body = json!({
        "model": cfg.model,
        "stream": true,
        "tools": tools::definitions(),
        "messages": messages.iter().map(ChatMessage::to_json).collect::<Vec<_>>(),
    })
    .to_string();

    let mut response = agent()
        .post(&url)
        .header("Authorization", &format!("Bearer {}", cfg.api_key))
        .header("Content-Type", "application/json")
        .send(&body)
        .map_err(|e| e.to_string())?;

    let mut calls: BTreeMap<u64, ToolCall> = BTreeMap::new();
    let reader = BufReader::new(response.body_mut().as_reader());
    for line in reader.lines() {
        if cancel.load(Ordering::Relaxed) {
            let _ = tx.send(StreamEvent::Failed("aborted".into()));
            return Ok(());
        }
        let line = line.map_err(|e| e.to_string())?;
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data.is_empty() {
            continue;
        }
        if data == "[DONE]" {
            break;
        }
        let value: Value = serde_json::from_str(data).map_err(|e| e.to_string())?;
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
        let _ = tx.send(StreamEvent::Tools(calls));
    }
    Ok(())
}

fn agent() -> Agent {
    static AGENT: OnceLock<Agent> = OnceLock::new();
    AGENT
        .get_or_init(|| {
            Agent::config_builder()
                .timeout_global(Some(Duration::from_secs(300)))
                .build()
                .into()
        })
        .clone()
}

fn nonempty(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|s| !s.is_empty())
}
