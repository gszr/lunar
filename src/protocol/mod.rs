//! Completions and Responses streams. Generic OpenAI-shaped HTTP. No branded providers.

mod completions;
mod http;
mod responses;

use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Sender;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Api {
    Completions,
    Responses,
    Messages,
}

impl Api {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "completions" => Some(Self::Completions),
            "responses" => Some(Self::Responses),
            "messages" => Some(Self::Messages),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Completions => "completions",
            Self::Responses => "responses",
            Self::Messages => "messages",
        }
    }
}

#[derive(Clone)]
pub struct Config {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    pub provider: String,
    pub window: Option<u32>,
    pub api: Api,
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
            api: Api::Completions,
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
    Tools {
        calls: Vec<ToolCall>,
        truncated: bool,
    },
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
    let result = match cfg.api {
        Api::Completions => completions::stream(cfg, messages, cancel, &tx),
        Api::Responses => responses::stream(cfg, messages, cancel, &tx),
        Api::Messages => Err(format!("{} uses messages, not implemented", cfg.model)),
    };
    if let Err(err) = result {
        let _ = tx.send(StreamEvent::Failed(err));
    }
}

fn nonempty(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|s| !s.is_empty())
}
