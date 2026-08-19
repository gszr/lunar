//! Chat Completions stream. Generic OpenAI-shaped HTTP. No branded providers.

use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use serde_json::{Value, json};
use ureq::Agent;

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

pub struct ChatMessage {
    pub role: &'static str,
    pub content: String,
}

pub enum StreamEvent {
    Delta(String),
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
        "messages": messages.iter().map(|m| json!({
            "role": m.role,
            "content": m.content,
        })).collect::<Vec<_>>(),
    })
    .to_string();

    let mut response = agent()
        .post(&url)
        .header("Authorization", &format!("Bearer {}", cfg.api_key))
        .header("Content-Type", "application/json")
        .send(&body)
        .map_err(|e| e.to_string())?;

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
        let Some(text) = value
            .pointer("/choices/0/delta/content")
            .and_then(Value::as_str)
        else {
            continue;
        };
        if !text.is_empty() {
            let _ = tx.send(StreamEvent::Delta(text.to_string()));
        }
    }
    let _ = tx.send(StreamEvent::Done);
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
