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
    pub provider: String,
    pub window: Option<u32>,
}

impl Config {
    pub fn from_env() -> Option<Self> {
        let model = nonempty("LUNAR_MODEL")?;
        let base_url = nonempty("LUNAR_BASE_URL")?;
        Some(Self {
            api_key: nonempty("LUNAR_API_KEY")?,
            provider: nonempty("LUNAR_PROVIDER").unwrap_or_else(|| provider_from_url(&base_url)),
            window: nonempty("LUNAR_CONTEXT_WINDOW")
                .and_then(|s| s.parse().ok())
                .or_else(|| guess_window(&model)),
            base_url,
            model,
        })
    }

    pub fn context_window(&self) -> Option<u32> {
        self.window
    }

    pub fn provider(&self) -> &str {
        &self.provider
    }
}

pub fn guess_window(id: &str) -> Option<u32> {
    if id.contains("grok-4.6") || id.contains("grok-4.5") {
        Some(500_000)
    } else if id.contains("grok-4.3") {
        Some(1_000_000)
    } else if id.contains("grok-build") {
        Some(256_000)
    } else {
        None
    }
}

fn provider_from_url(base: &str) -> String {
    let rest = base.split("://").nth(1).unwrap_or(base);
    let host = rest.split('/').next().unwrap_or(rest);
    let host = host.strip_prefix("api.").unwrap_or(host);
    let mut parts: Vec<&str> = host.split('.').collect();
    if parts.len() >= 2 {
        let tld = parts.last().copied().unwrap_or("");
        if matches!(tld, "com" | "ai" | "dev" | "io" | "net" | "org") {
            parts.pop();
        }
    }
    let name = parts.join("");
    if name.is_empty() {
        "openai".into()
    } else {
        name
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

#[derive(Clone, Copy, Default)]
pub struct Usage {
    pub input: u32,
    pub output: u32,
    pub cache_read: u32,
    pub cache_write: u32,
}

impl Usage {
    pub fn add(&mut self, other: Usage) {
        self.input += other.input;
        self.output += other.output;
        self.cache_read += other.cache_read;
        self.cache_write += other.cache_write;
    }

    pub fn prompt(&self) -> u32 {
        self.input + self.cache_read + self.cache_write
    }
}

pub enum StreamEvent {
    Delta(String),
    Think(String),
    Usage(Usage),
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
        "stream_options": { "include_usage": true },
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
    let mut usage = None;
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
        let _ = tx.send(StreamEvent::Tools(calls));
    }
    Ok(())
}

fn agent() -> Agent {
    static AGENT: OnceLock<Agent> = OnceLock::new();
    AGENT
        .get_or_init(|| {
            Agent::config_builder()
                .timeout_global(None)
                .timeout_connect(Some(Duration::from_secs(30)))
                .build()
                .into()
        })
        .clone()
}

fn parse_usage(value: &Value) -> Option<Usage> {
    let usage = value.get("usage")?;
    if usage.is_null() {
        return None;
    }
    let num = |v: &Value, keys: &[&str]| -> u32 {
        for key in keys {
            if let Some(n) = v.get(*key).and_then(Value::as_u64) {
                return n as u32;
            }
        }
        0
    };
    let details = usage.get("prompt_tokens_details");
    let cache_read = details
        .and_then(|d| d.get("cached_tokens").and_then(Value::as_u64))
        .unwrap_or_else(|| u64::from(num(usage, &["cache_read_tokens", "cached_tokens"])))
        as u32;
    let cache_write = details
        .and_then(|d| d.get("cache_creation_tokens").and_then(Value::as_u64))
        .unwrap_or_else(|| u64::from(num(usage, &["cache_write_tokens", "cache_creation_tokens"])))
        as u32;
    Some(Usage {
        input: num(usage, &["prompt_tokens", "input_tokens"]),
        output: num(usage, &["completion_tokens", "output_tokens"]),
        cache_read,
        cache_write,
    })
}

fn nonempty(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|s| !s.is_empty())
}
