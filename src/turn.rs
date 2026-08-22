//! Model turn streaming, persistence, and tool orchestration.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, TryRecvError};

use crate::app::{App, Message, Role};
use crate::protocol::{self, ChatMessage, Config, StreamEvent, ToolCall, ToolResult};
use crate::transcript::jump_to_tail;
use crate::{mission, prompt, tools};

const MAX_ROUNDS: u32 = 50;

pub(crate) fn drain_stream(app: &mut App) {
    let Some(rx) = app.stream_rx.as_ref() else {
        return;
    };
    let mut end: Option<StreamEvent> = None;
    loop {
        match rx.try_recv() {
            Ok(StreamEvent::Delta(text)) => {
                if let Some(last) = app.messages.last_mut()
                    && matches!(last.role, Role::Assistant)
                {
                    last.text.push_str(&text);
                }
            }
            Ok(StreamEvent::Think(text)) => {
                if let Some(last) = app.messages.last_mut()
                    && matches!(last.role, Role::Assistant)
                {
                    last.thinking.push_str(&text);
                }
            }
            Ok(StreamEvent::Usage(usage)) => {
                app.usage.add(usage);
                app.last_prompt = usage.prompt();
            }
            Ok(other) => {
                end = Some(other);
                break;
            }
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                end = Some(StreamEvent::Failed("stream ended".into()));
                break;
            }
        }
    }
    if let Some(end) = end {
        finish_stream(app, end);
    }
}

pub(crate) fn finish_stream(app: &mut App, end: StreamEvent) {
    match end {
        StreamEvent::Tools { calls, truncated } => begin_tools(app, calls, truncated),
        StreamEvent::ToolResults(results) => apply_tool_results(app, results),
        StreamEvent::Done => {
            persist_last_assistant(app);
            app.stream_rx = None;
            app.cancel = None;
            pop_empty_assistant(app);
        }
        StreamEvent::Failed(err) => {
            persist_last_assistant(app);
            app.stream_rx = None;
            app.cancel = None;
            pop_empty_assistant(app);
            app.notice = Some(err);
        }
        StreamEvent::Delta(_) | StreamEvent::Think(_) | StreamEvent::Usage(_) => {}
    }
}

pub(crate) fn abort_turn(app: &mut App) {
    if let Some(flag) = &app.cancel {
        flag.store(true, Ordering::Relaxed);
    }
    persist_last_assistant(app);
    app.stream_rx = None;
    app.cancel = None;
    pop_empty_assistant(app);
    app.notice = Some("aborted".into());
}

pub(crate) fn pop_empty_assistant(app: &mut App) {
    if matches!(
        app.messages.last(),
        Some(Message {
            role: Role::Assistant,
            text,
            thinking,
            tool_calls,
            ..
        }) if text.is_empty() && thinking.is_empty() && tool_calls.is_empty()
    ) {
        app.messages.pop();
    }
}

pub(crate) fn begin_tools(app: &mut App, calls: Vec<ToolCall>, truncated: bool) {
    if let Some(last) = app.messages.last_mut()
        && matches!(last.role, Role::Assistant)
    {
        last.tool_calls = calls.clone();
    }
    persist_last_assistant(app);
    if truncated {
        apply_tool_results(app, skipped_truncated(&calls));
        return;
    }
    let Some(cancel) = app.cancel.clone() else {
        return;
    };
    let (tx, rx) = mpsc::channel();
    app.stream_rx = Some(rx);
    std::thread::spawn(move || {
        let results = run_tools_parallel(&calls, &cancel);
        if cancel.load(Ordering::Relaxed) {
            let _ = tx.send(StreamEvent::Failed("aborted".into()));
            return;
        }
        let _ = tx.send(StreamEvent::ToolResults(results));
    });
}

pub(crate) fn skipped_truncated(calls: &[ToolCall]) -> Vec<ToolResult> {
    calls
        .iter()
        .map(|call| ToolResult {
            id: call.id.clone(),
            title: call.name.clone(),
            content: "not executed: hit the output token limit; arguments may be truncated. Re-issue the call.".into(),
        })
        .collect()
}

pub(crate) fn run_tools_parallel(calls: &[ToolCall], cancel: &AtomicBool) -> Vec<ToolResult> {
    std::thread::scope(|s| {
        let handles: Vec<_> = calls
            .iter()
            .map(|call| {
                s.spawn(move || {
                    if cancel.load(Ordering::Relaxed) {
                        return ToolResult {
                            id: call.id.clone(),
                            title: call.name.clone(),
                            content: "aborted".into(),
                        };
                    }
                    let out = tools::run(&call.name, &call.arguments, cancel);
                    ToolResult {
                        id: call.id.clone(),
                        title: out.title,
                        content: out.content,
                    }
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| {
                h.join().unwrap_or_else(|_| ToolResult {
                    id: String::new(),
                    title: "tool".into(),
                    content: "tool panicked".into(),
                })
            })
            .collect()
    })
}

pub(crate) fn apply_tool_results(app: &mut App, results: Vec<ToolResult>) {
    if app
        .cancel
        .as_ref()
        .is_some_and(|c| c.load(Ordering::Relaxed))
    {
        app.stream_rx = None;
        app.cancel = None;
        return;
    }
    for result in results {
        persist_value(
            app,
            &mission::tool_line(&result.id, &result.title, &result.content),
        );
        app.messages
            .push(Message::tool(result.id, result.title, result.content));
    }
    app.rounds += 1;
    if app.rounds >= MAX_ROUNDS {
        app.stream_rx = None;
        app.cancel = None;
        app.notice = Some(format!(
            "tool-round limit reached ({MAX_ROUNDS}); submit \"continue\" to proceed"
        ));
        return;
    }
    continue_turn(app);
}

pub(crate) fn persist_value(app: &mut App, value: &serde_json::Value) {
    if app.mission.is_none() {
        let name = value
            .get("text")
            .and_then(serde_json::Value::as_str)
            .map(mission::semantic_name)
            .unwrap_or_else(|| "Untitled Mission".into());
        match mission::create(&name) {
            Ok(m) => app.mission = Some(m),
            Err(err) => {
                app.notice = Some(format!("mission: {err}"));
                return;
            }
        }
    }
    if let Some(m) = &app.mission
        && let Err(err) = mission::append(m, value)
    {
        app.notice = Some(format!("mission: {err}"));
    }
}

pub(crate) fn persist_last_assistant(app: &mut App) {
    let Some(last) = app.messages.last() else {
        return;
    };
    if !matches!(last.role, Role::Assistant) {
        return;
    }
    if last.text.is_empty() && last.tool_calls.is_empty() {
        return;
    }
    persist_value(app, &mission::assistant_line(&last.text, &last.tool_calls));
}

pub(crate) fn send_prompt(app: &mut App, line: String) {
    if app.config.is_none() {
        app.notice = Some("no model configured".into());
        return;
    }
    if let Some(window) = app.config.as_ref().and_then(Config::context_window)
        && window > 0
        && app.last_prompt >= window
    {
        app.notice = Some("context window full".into());
        return;
    }
    app.notice = None;
    app.rounds = 0;
    app.preamble = prompt::preamble();
    persist_value(app, &mission::user_line(&line));
    app.messages.push(Message::user(line));
    jump_to_tail(app);
    let cancel = Arc::new(AtomicBool::new(false));
    app.cancel = Some(cancel);
    continue_turn(app);
}

pub(crate) fn continue_turn(app: &mut App) {
    let Some(cfg) = app.config.clone() else {
        return;
    };
    let Some(cancel) = app.cancel.clone() else {
        return;
    };
    app.messages.push(Message::assistant());
    let history = api_history(app.preamble.as_deref(), &app.messages);
    let (tx, rx) = mpsc::channel();
    app.stream_rx = Some(rx);
    std::thread::spawn(move || protocol::stream(cfg, history, cancel, tx));
}

pub(crate) fn api_history(preamble: Option<&str>, messages: &[Message]) -> Vec<ChatMessage> {
    let mut out = Vec::new();
    if let Some(text) = preamble {
        out.push(ChatMessage::User(text.to_string()));
    }
    out.extend(
        messages
            .iter()
            .filter(|m| {
                !m.text.is_empty() || matches!(m.role, Role::User) || !m.tool_calls.is_empty()
            })
            .map(|m| match m.role {
                Role::User => ChatMessage::User(m.text.clone()),
                Role::Assistant => ChatMessage::Assistant {
                    content: m.text.clone(),
                    tool_calls: m.tool_calls.clone(),
                },
                Role::Tool => ChatMessage::Tool {
                    id: m.tool_id.clone(),
                    content: m.text.clone(),
                },
            }),
    );
    out
}
