//! Application state.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Receiver;

use ratatui::text::Line;

use crate::protocol::{Config, StreamEvent, ToolCall, Usage};
use crate::{lua, mission};

pub(crate) struct App {
    pub(crate) input: String,
    pub(crate) cursor: usize,
    pub(crate) notice: Option<String>,
    pub(crate) messages: Vec<Message>,
    pub(crate) config: Option<Config>,
    pub(crate) startup_config: Option<Config>,
    pub(crate) thinking_override: Option<String>,
    pub(crate) models: Vec<lua::ModelChoice>,
    pub(crate) stream_rx: Option<Receiver<StreamEvent>>,
    pub(crate) cancel: Option<Arc<AtomicBool>>,
    pub(crate) rounds: u32,
    pub(crate) usage: Usage,
    pub(crate) last_prompt: u32,
    pub(crate) preamble: Option<String>,
    pub(crate) mission: Option<mission::Mission>,
    pub(crate) mode: Mode,
    pub(crate) complete_sel: usize,
    pub(crate) quit: bool,
    pub(crate) scroll: usize,
    pub(crate) follow: bool,
    pub(crate) transcript_w: u16,
    pub(crate) transcript_h: u16,
    pub(crate) paint_width: usize,
    pub(crate) paint_frozen: Vec<Line<'static>>,
    pub(crate) paint_upto: usize,
    pub(crate) paint_prev_tool: bool,
    pub(crate) history: Vec<String>,
    pub(crate) history_cursor: Option<usize>,
    pub(crate) history_draft: String,
    pub(crate) search: Option<HistorySearch>,
    pub(crate) auth_rx: Option<Receiver<AuthEvent>>,
    pub(crate) auth_cancel: Option<Arc<AtomicBool>>,
    pub(crate) auth_prompt: Option<AuthPrompt>,
    pub(crate) auth_brand: Option<&'static str>,
}

pub(crate) struct HistorySearch {
    pub(crate) draft: String,
    pub(crate) draft_cursor: usize,
    pub(crate) query: String,
    pub(crate) matched: Option<usize>,
}

pub(crate) enum AuthEvent {
    DeviceCode {
        url: String,
        code: String,
        browser_opened: bool,
    },
    Done,
    Failed(String),
}

pub(crate) struct AuthPrompt {
    pub(crate) url: String,
    pub(crate) code: String,
    pub(crate) browser_opened: bool,
}

pub(crate) enum Mode {
    Chat,
    LoginProvider {
        cursor: usize,
    },
    LoginMethod {
        cursor: usize,
    },
    ApiKey,
    Context {
        text: String,
        scroll: usize,
    },
    Resume {
        items: Vec<mission::Meta>,
        cursor: usize,
        title: String,
        query: Option<String>,
    },
    Model {
        items: Vec<lua::ModelChoice>,
        cursor: usize,
    },
    Thinking {
        cursor: usize,
    },
}

pub(crate) struct Message {
    pub(crate) role: Role,
    pub(crate) text: String,
    pub(crate) thinking: String,
    pub(crate) tool_calls: Vec<ToolCall>,
    pub(crate) tool_id: String,
    pub(crate) tool_title: String,
}

pub(crate) enum Role {
    User,
    Assistant,
    Tool,
}

impl App {
    pub(crate) fn new(loaded: crate::lua::Loaded) -> Self {
        let startup_config = loaded.config.clone();
        Self {
            input: String::new(),
            cursor: 0,
            notice: loaded.notice,
            messages: Vec::new(),
            config: loaded.config,
            startup_config,
            thinking_override: None,
            models: loaded.models,
            stream_rx: None,
            cancel: None,
            rounds: 0,
            usage: Usage::default(),
            last_prompt: 0,
            preamble: None,
            mission: None,
            mode: Mode::Chat,
            complete_sel: 0,
            quit: false,
            scroll: 0,
            follow: true,
            transcript_w: 0,
            transcript_h: 0,
            paint_width: 0,
            paint_frozen: Vec::new(),
            paint_upto: 0,
            paint_prev_tool: false,
            history: crate::history::load().unwrap_or_default(),
            history_cursor: None,
            history_draft: String::new(),
            search: None,
            auth_rx: None,
            auth_cancel: None,
            auth_prompt: None,
            auth_brand: None,
        }
    }
}

impl Message {
    pub(crate) fn user(text: String) -> Self {
        Self {
            role: Role::User,
            text,
            thinking: String::new(),
            tool_calls: Vec::new(),
            tool_id: String::new(),
            tool_title: String::new(),
        }
    }

    pub(crate) fn assistant() -> Self {
        Self {
            role: Role::Assistant,
            text: String::new(),
            thinking: String::new(),
            tool_calls: Vec::new(),
            tool_id: String::new(),
            tool_title: String::new(),
        }
    }

    pub(crate) fn tool(id: String, title: String, content: String) -> Self {
        Self {
            role: Role::Tool,
            text: content,
            thinking: String::new(),
            tool_calls: Vec::new(),
            tool_id: id,
            tool_title: title,
        }
    }
}
