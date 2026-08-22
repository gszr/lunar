//! Completions and Responses streams. Generic OpenAI-shaped HTTP. No branded providers.

use std::collections::BTreeMap;
use std::io::{self, BufRead, BufReader, Read};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use ureq::Agent;

use crate::tools;

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

fn flatten_responses_input(messages: &[ChatMessage]) -> Vec<Value> {
    let mut out = Vec::new();
    for (i, message) in messages.iter().enumerate() {
        match message.to_responses(i) {
            Value::Array(items) => out.extend(items),
            other => out.push(other),
        }
    }
    out
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
        Api::Completions => stream_completions(cfg, messages, cancel, &tx),
        Api::Responses => stream_responses(cfg, messages, cancel, &tx),
        Api::Messages => Err(format!("{} uses messages, not implemented", cfg.model)),
    };
    if let Err(err) = result {
        let _ = tx.send(StreamEvent::Failed(err));
    }
}

/// Hard cap on reasoning + answer. grok-4.6 ignores thinking level.
const MAX_TOKENS: u32 = 32_768;
const MAX_RETRIES: u32 = 3;

fn completions_body(cfg: &Config, messages: &[ChatMessage]) -> String {
    let mut body = json!({
        "model": cfg.model,
        "stream": true,
        "stream_options": { "include_usage": true },
        "tools": tools::definitions(),
        "messages": messages.iter().map(ChatMessage::to_json).collect::<Vec<_>>(),
    });
    if is_openai_url(&cfg.base_url) {
        body["max_completion_tokens"] = json!(MAX_TOKENS);
        body["reasoning_effort"] = json!("none");
    } else {
        body["max_tokens"] = json!(MAX_TOKENS);
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

fn stream_completions(
    cfg: Config,
    messages: Vec<ChatMessage>,
    cancel: Arc<AtomicBool>,
    tx: &Sender<StreamEvent>,
) -> Result<(), String> {
    let url = format!("{}/chat/completions", cfg.base_url.trim_end_matches('/'));
    let body = completions_body(&cfg, &messages);

    let response = post_retry(&url, &cfg.api_key, &body, &cancel)?;

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

fn responses_body(cfg: &Config, messages: &[ChatMessage]) -> String {
    json!({
        "model": cfg.model,
        "stream": true,
        "store": false,
        "tools": tools::responses_definitions(),
        "input": flatten_responses_input(messages),
    })
    .to_string()
}

fn stream_responses(
    cfg: Config,
    messages: Vec<ChatMessage>,
    cancel: Arc<AtomicBool>,
    tx: &Sender<StreamEvent>,
) -> Result<(), String> {
    let url = format!("{}/responses", cfg.base_url.trim_end_matches('/'));
    let body = responses_body(&cfg, &messages);
    let response = post_retry(&url, &cfg.api_key, &body, &cancel)?;

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
        if let Some(parsed) = parse_usage(&value).or_else(|| {
            value
                .get("response")
                .and_then(parse_usage)
        }) {
            usage = Some(parsed);
        }
        apply_responses_event(&value, tx, &mut calls, &mut truncated);
        if responses_finished(&value) {
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

fn apply_responses_event(
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
            let index = value.get("output_index").and_then(Value::as_u64).unwrap_or(0);
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
            let index = value.get("output_index").and_then(Value::as_u64).unwrap_or(0);
            let entry = calls.entry(index).or_insert_with(|| ToolCall {
                id: String::new(),
                name: String::new(),
                arguments: String::new(),
            });
            if let Some(id) = value
                .get("call_id")
                .or_else(|| value.get("item_id"))
                .and_then(Value::as_str)
            {
                entry.id = id.to_string();
            }
            if let Some(args) = value.get("arguments").and_then(Value::as_str) {
                entry.arguments = args.to_string();
            }
        }
        "response.completed" | "response.incomplete" => {
            if let Some(status) = value
                .pointer("/response/status")
                .and_then(Value::as_str)
            {
                *truncated = status == "incomplete";
            }
            if event == "response.incomplete" {
                *truncated = true;
            }
        }
        _ => {}
    }
}

fn responses_finished(value: &Value) -> bool {
    matches!(
        value.get("type").and_then(Value::as_str),
        Some("response.completed" | "response.incomplete" | "response.failed")
    )
}

/// Usage often arrives after finish_reason. Wait briefly for it, then keep
/// reading in the background so the socket can return to the pool.
fn collect_tail(
    reader: impl BufRead + Send + 'static,
    usage: &mut Option<Usage>,
    saw_done: &mut bool,
    cancel: &AtomicBool,
) {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut lines = reader.lines();
        while let Some(line) = lines.next() {
            let Ok(line) = line else {
                return;
            };
            let done = is_done_line(&line);
            if tx.send(line).is_err() {
                if done {
                    return;
                }
                for line in lines {
                    let Ok(line) = line else {
                        return;
                    };
                    if is_done_line(&line) {
                        return;
                    }
                }
                return;
            }
        }
    });
    if usage.is_some() {
        return;
    }
    let deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(line) => {
                if is_done_line(&line) {
                    *saw_done = true;
                    break;
                }
                if let Some(data) = sse_payload(&line)
                    && let Ok(value) = serde_json::from_str::<Value>(data)
                    && let Some(parsed) = parse_usage(&value)
                {
                    *usage = Some(parsed);
                    break;
                }
            }
            Err(_) => break,
        }
    }
}

fn is_done_line(line: &str) -> bool {
    line.strip_prefix("data:")
        .is_some_and(|d| d.trim() == "[DONE]")
}

fn sse_payload(line: &str) -> Option<&str> {
    let data = line.strip_prefix("data:")?.trim();
    if data.is_empty() || data == "[DONE]" {
        None
    } else {
        Some(data)
    }
}

fn post_retry(
    url: &str,
    api_key: &str,
    body: &str,
    cancel: &AtomicBool,
) -> Result<ureq::http::Response<ureq::Body>, String> {
    let mut attempt = 0;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("aborted".into());
        }
        crate::debug::event(
            "request",
            json!({
                "attempt": attempt + 1,
                "method": "POST",
                "url": url,
                "headers": {
                    "authorization": "Bearer [REDACTED]",
                    "content-type": "application/json"
                },
                "body": serde_json::from_str::<Value>(body).unwrap_or_else(|_| Value::String(body.into())),
            }),
        );
        let response = agent()
            .post(url)
            .header("Authorization", &format!("Bearer {api_key}"))
            .header("Content-Type", "application/json")
            .send(body);
        match response {
            Ok(response) if response.status().is_success() => {
                crate::debug::event(
                    "response_start",
                    json!({
                        "attempt": attempt + 1,
                        "status": response.status().as_u16(),
                    }),
                );
                return Ok(response);
            }
            Ok(response) => {
                let status = response.status().as_u16();
                let retry = attempt < MAX_RETRIES && should_retry_status(status);
                let delay = retry.then(|| retry_delay(attempt));
                let body = response.into_body().read_to_string().unwrap_or_default();
                crate::debug::event(
                    "response_error",
                    json!({
                        "attempt": attempt + 1,
                        "status": status,
                        "body": body,
                        "retry": retry,
                        "retry_delay_ms": delay.map(|d| d.as_millis()),
                    }),
                );
                if !retry {
                    return Err(error_response(status, &body));
                }
                sleep_cancel(delay.unwrap(), cancel)?;
                attempt += 1;
            }
            Err(err) => {
                let retry = attempt < MAX_RETRIES && should_retry(&err);
                let delay = retry.then(|| retry_delay(attempt));
                crate::debug::event(
                    "request_error",
                    json!({
                        "attempt": attempt + 1,
                        "error": err.to_string(),
                        "retry": retry,
                        "retry_delay_ms": delay.map(|d| d.as_millis()),
                    }),
                );
                if !retry {
                    return Err(err.to_string());
                }
                sleep_cancel(delay.unwrap(), cancel)?;
                attempt += 1;
            }
        }
    }
}

fn should_retry_status(code: u16) -> bool {
    matches!(code, 408 | 409 | 429) || code >= 500
}

fn error_response(status: u16, body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .pointer("/error/message")
                .or_else(|| value.get("message"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .filter(|message| !message.is_empty())
        .unwrap_or_else(|| format!("http status: {status}"))
}

fn should_retry(err: &ureq::Error) -> bool {
    match err {
        ureq::Error::StatusCode(code) => should_retry_status(*code),
        ureq::Error::Io(e) => matches!(
            e.kind(),
            io::ErrorKind::ConnectionReset
                | io::ErrorKind::ConnectionAborted
                | io::ErrorKind::TimedOut
                | io::ErrorKind::BrokenPipe
                | io::ErrorKind::UnexpectedEof
                | io::ErrorKind::Interrupted
                | io::ErrorKind::NotConnected
        ),
        ureq::Error::ConnectionFailed | ureq::Error::Timeout(_) => true,
        _ => false,
    }
}

fn retry_delay(attempt: u32) -> Duration {
    Duration::from_millis(500 * (1u64 << attempt.min(4)))
}

fn sleep_cancel(total: Duration, cancel: &AtomicBool) -> Result<(), String> {
    let start = Instant::now();
    while start.elapsed() < total {
        if cancel.load(Ordering::Relaxed) {
            return Err("aborted".into());
        }
        let left = total.saturating_sub(start.elapsed());
        thread::sleep(left.min(Duration::from_millis(50)));
    }
    Ok(())
}

fn agent() -> Agent {
    static AGENT: OnceLock<Agent> = OnceLock::new();
    AGENT
        .get_or_init(|| {
            Agent::config_builder()
                .http_status_as_error(false)
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
    let prompt = num(usage, &["prompt_tokens", "input_tokens"]);
    Some(Usage {
        input: prompt.saturating_sub(cache_read.saturating_add(cache_write)),
        output: num(usage, &["completion_tokens", "output_tokens"]),
        cache_read,
        cache_write,
    })
}

fn nonempty(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_caps_completion_tokens() {
        let cfg = Config {
            api_key: "k".into(),
            base_url: "https://api.x.ai/v1".into(),
            model: "grok-4.6".into(),
            provider: "xai".into(),
            window: Some(500_000),
            api: Api::Completions,
        };
        let body: Value = serde_json::from_str(&completions_body(&cfg, &[])).unwrap();
        assert_eq!(body["max_tokens"], MAX_TOKENS);
        assert_eq!(body["stream"], true);
        assert_eq!(body["model"], "grok-4.6");
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
        };
        let body: Value = serde_json::from_str(&completions_body(&cfg, &[])).unwrap();
        assert_eq!(body["max_completion_tokens"], MAX_TOKENS);
        assert_eq!(body["reasoning_effort"], "none");
        assert!(body.get("max_tokens").is_none());
    }

    #[test]
    fn parse_usage_splits_cache_out_of_prompt() {
        let value = json!({
            "usage": {
                "prompt_tokens": 1000,
                "completion_tokens": 20,
                "prompt_tokens_details": { "cached_tokens": 800 }
            }
        });
        let u = parse_usage(&value).unwrap();
        assert_eq!(u.input, 200);
        assert_eq!(u.cache_read, 800);
        assert_eq!(u.output, 20);
        assert_eq!(u.prompt(), 1000);
    }

    #[test]
    fn retries_transient_status_not_auth() {
        assert!(should_retry_status(429));
        assert!(should_retry_status(503));
        assert!(should_retry_status(408));
        assert!(!should_retry_status(401));
        assert!(!should_retry_status(400));
    }

    #[test]
    fn error_response_uses_json_message() {
        assert_eq!(
            error_response(400, r#"{"error":{"message":"bad model"}}"#),
            "bad model"
        );
        assert_eq!(error_response(400, "not json"), "http status: 400");
    }

    #[test]
    fn retry_delay_grows_then_caps() {
        assert_eq!(retry_delay(0), Duration::from_millis(500));
        assert_eq!(retry_delay(1), Duration::from_millis(1000));
        assert_eq!(retry_delay(2), Duration::from_millis(2000));
        assert_eq!(retry_delay(4), Duration::from_millis(8000));
        assert_eq!(retry_delay(9), Duration::from_millis(8000));
    }

    fn sample_cfg() -> Config {
        Config {
            api_key: "k".into(),
            base_url: "https://api.openai.com/v1".into(),
            model: "gpt-5".into(),
            provider: "openai".into(),
            window: None,
            api: Api::Responses,
        }
    }

    #[test]
    fn responses_body_uses_input_and_flat_tools() {
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
        let body: Value = serde_json::from_str(&responses_body(&sample_cfg(), &messages)).unwrap();
        assert_eq!(body["model"], "gpt-5");
        assert_eq!(body["stream"], true);
        assert_eq!(body["store"], false);
        assert!(body.get("max_output_tokens").is_none());
        assert!(body.get("messages").is_none());
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["name"], "read");
        assert!(body["tools"][0].get("function").is_none());
        assert_eq!(body["input"].as_array().unwrap().len(), 4);
        assert!(body["input"][0].get("type").is_none());
        assert_eq!(body["input"][0]["role"], "user");
        assert_eq!(body["input"][0]["content"][0]["type"], "input_text");
        assert_eq!(body["input"][1]["type"], "message");
        assert_eq!(body["input"][1]["role"], "assistant");
        assert_eq!(body["input"][1]["status"], "completed");
        assert_eq!(body["input"][1]["id"], "msg_lunar_1");
        assert_eq!(body["input"][1]["content"][0]["annotations"], json!([]));
        assert_eq!(body["input"][2]["type"], "function_call");
        assert_eq!(body["input"][2]["call_id"], "call_1");
        assert!(body["input"][2].get("id").is_none());
        assert_eq!(body["input"][3]["type"], "function_call_output");
        assert_eq!(body["input"][3]["call_id"], "call_1");
    }

    #[test]
    fn responses_text_delta_and_function_call() {
        let (tx, rx) = mpsc::channel();
        let mut calls = BTreeMap::new();
        let mut truncated = false;
        apply_responses_event(
            &json!({"type":"response.output_text.delta","delta":"hello"}),
            &tx,
            &mut calls,
            &mut truncated,
        );
        apply_responses_event(
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
        apply_responses_event(
            &json!({
                "type": "response.function_call_arguments.delta",
                "output_index": 1,
                "delta": "{\"path\":"
            }),
            &tx,
            &mut calls,
            &mut truncated,
        );
        apply_responses_event(
            &json!({
                "type": "response.function_call_arguments.done",
                "output_index": 1,
                "call_id": "call_1",
                "arguments": "{\"path\":\"a\"}"
            }),
            &tx,
            &mut calls,
            &mut truncated,
        );
        apply_responses_event(
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
        assert!(responses_finished(&json!({"type":"response.completed"})));
    }
}
