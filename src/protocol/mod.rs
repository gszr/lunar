//! Completions and Responses streams. Generic OpenAI-shaped HTTP. No branded providers.

mod completions;
mod http;
mod responses;

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Sender;

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Thinking {
    #[default]
    Off,
    Low,
    Medium,
    High,
}

impl Thinking {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "off" => Some(Self::Off),
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
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
    pub auth_provider: Option<String>,
    pub thinking: Thinking,
}

impl Config {
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
    mut cfg: Config,
    messages: Vec<ChatMessage>,
    cancel: Arc<AtomicBool>,
    tx: Sender<StreamEvent>,
    cache_key: Option<String>,
) {
    if let Some(provider) = cfg.auth_provider.as_deref() {
        match crate::auth::resolve(provider) {
            Ok(access) => cfg.api_key = access,
            Err(err) => {
                let _ = tx.send(StreamEvent::Failed(err));
                return;
            }
        }
    }
    let result = match cfg.api {
        Api::Completions => completions::stream(cfg, messages, cancel, &tx),
        Api::Responses => responses::stream(cfg, messages, cancel, &tx, cache_key),
        Api::Messages => Err(format!("{} uses messages, not implemented", cfg.model)),
    };
    if let Err(err) = result {
        let _ = tx.send(StreamEvent::Failed(err));
    }
}
