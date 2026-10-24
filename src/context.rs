//! Model-visible context assembly and preview.

use std::collections::HashSet;
use std::fmt::Write as _;

use crate::app::{Message, Role};
use crate::prompt;
use crate::protocol::ChatMessage;

/// Build the semantic message history sent to either model protocol.
pub(crate) fn history(
    preamble: Option<&str>,
    compaction: Option<(&str, usize)>,
    messages: &[Message],
) -> Vec<ChatMessage> {
    let mut out = Vec::new();
    if let Some(text) = preamble {
        out.push(ChatMessage::User(text.to_string()));
    }
    let start = if let Some((summary, first_kept)) = compaction {
        out.push(ChatMessage::User(format!(
            "The conversation before this point was compacted into this checkpoint:\n\n{summary}"
        )));
        first_kept.min(messages.len())
    } else {
        0
    };
    out.extend(
        messages[start..]
            .iter()
            .filter(|m| {
                !m.aside
                    && !m.debug
                    && (!m.text.is_empty()
                        || matches!(m.role, Role::User | Role::Tool)
                        || !m.thinking.is_empty()
                        || !m.tool_calls.is_empty())
            })
            .map(|m| match m.role {
                Role::User => ChatMessage::User(m.text.clone()),
                Role::Assistant => ChatMessage::Assistant {
                    content: m.text.clone(),
                    thinking: m.thinking.clone(),
                    thinking_signature: m.thinking_signature.clone(),
                    tool_calls: m.tool_calls.clone(),
                },
                Role::Tool => ChatMessage::Tool {
                    id: m.tool_id.clone(),
                    content: m.text.clone(),
                },
            }),
    );
    // An interrupted mission can end a tool round before its results are recorded.
    let mut results = HashSet::new();
    for message in out.iter_mut().rev() {
        match message {
            ChatMessage::Tool { id, .. } => {
                results.insert(id.clone());
            }
            ChatMessage::Assistant { tool_calls, .. } => {
                tool_calls.retain(|call| results.contains(&call.id));
                results.clear();
            }
            ChatMessage::User(_) => results.clear(),
        }
    }
    out.retain(|message| {
        !matches!(message, ChatMessage::Assistant { content, thinking, tool_calls, .. }
            if content.is_empty() && thinking.is_empty() && tool_calls.is_empty())
    });
    out
}

/// Summarize the context known before the next user message.
pub(crate) fn summary(
    messages: &[Message],
    compaction: Option<(&str, usize)>,
    stack: &str,
) -> String {
    let (mut out, preamble_tokens) = prompt::summary(stack);
    let mut users = 0;
    let mut assistants = 0;
    let mut tool_calls = 0;
    let mut tool_results = 0;
    let mut user_chars = 0;
    let mut assistant_chars = 0;
    let mut tool_call_chars = 0;
    let mut tool_result_chars = 0;
    let start = compaction
        .map(|(_, first_kept)| first_kept)
        .unwrap_or(0)
        .min(messages.len());
    for message in history(None, None, &messages[start..]) {
        match message {
            ChatMessage::User(content) => {
                users += 1;
                user_chars += content.chars().count();
            }
            ChatMessage::Assistant {
                content,
                tool_calls: calls,
                ..
            } => {
                assistants += usize::from(!content.is_empty() || !calls.is_empty());
                assistant_chars += content.chars().count();
                tool_calls += calls.len();
                tool_call_chars += calls
                    .iter()
                    .map(|call| call.arguments.chars().count())
                    .sum::<usize>();
            }
            ChatMessage::Tool { content, .. } => {
                tool_results += 1;
                tool_result_chars += content.chars().count();
            }
        }
    }
    let user_tokens = user_chars.div_ceil(4);
    let assistant_tokens = assistant_chars.div_ceil(4);
    let tool_call_tokens = tool_call_chars.div_ceil(4);
    let tool_result_tokens = tool_result_chars.div_ceil(4);
    let summary_tokens = compaction
        .map(|(summary, _)| summary.chars().count().div_ceil(4))
        .unwrap_or(0);
    let total_tokens =
        summary_tokens + user_tokens + assistant_tokens + tool_call_tokens + tool_result_tokens;
    let _ = write!(
        out,
        "\n\ncheckpoint  ~{summary_tokens} tokens\nhistory  ~{} tokens\n  user messages      {users}  ~{user_tokens} tokens\n  assistant messages {assistants}  ~{assistant_tokens} tokens\n  tool calls         {tool_calls}  ~{tool_call_tokens} tokens\n  tool results       {tool_results}  ~{tool_result_tokens} tokens\n\ntotal  ~{} tokens",
        total_tokens - summary_tokens,
        preamble_tokens + total_tokens
    );
    out
}

/// Display the complete context known before the next user message.
pub(crate) fn raw(messages: &[Message], compaction: Option<(&str, usize)>, stack: &str) -> String {
    let preamble = prompt::preamble(stack);
    let history = history(preamble.as_deref(), compaction, messages);
    format_history(history)
}

fn format_history(history: Vec<ChatMessage>) -> String {
    if history.is_empty() {
        return "no prompt context or mission history".into();
    }

    let mut out = String::from("next prompt context (next user message not included)");
    for message in history {
        match message {
            ChatMessage::User(content) => {
                let _ = write!(out, "\n\n[user]\n{content}");
            }
            ChatMessage::Assistant {
                content,
                tool_calls,
                ..
            } => {
                if !content.is_empty() {
                    let _ = write!(out, "\n\n[assistant]\n{content}");
                }
                for call in tool_calls {
                    let _ = write!(
                        out,
                        "\n\n[assistant tool call: {} · {}]\n{}",
                        call.name, call.id, call.arguments
                    );
                }
            }
            ChatMessage::Tool { id, content } => {
                let _ = write!(out, "\n\n[tool result: {id}]\n{content}");
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Message;
    use crate::protocol::ToolCall;

    #[test]
    fn history_excludes_asides_and_debug_cards() {
        let mut answer = Message::aside_assistant();
        answer.text = "side answer".into();
        let messages = vec![
            Message::user("main question".into()),
            Message::aside_user("btw: side question".into()),
            answer,
            Message::debug("debug HTTP 200".into(), "x-test: one".into()),
        ];

        let history = history(None, None, &messages);

        assert_eq!(history.len(), 1);
        assert!(matches!(&history[0], ChatMessage::User(text) if text == "main question"));
    }

    #[test]
    fn summary_counts_history_components_without_contents() {
        let mut assistant = Message::assistant();
        assistant.text = "secret answer".into();
        assistant.tool_calls.push(ToolCall {
            id: "call-1".into(),
            name: "read".into(),
            arguments: "secret arguments".into(),
        });
        let messages = vec![
            Message::user("secret question".into()),
            assistant,
            Message::tool("call-1".into(), "read".into(), "secret result".into()),
        ];

        let summary = summary(&messages, None, "");
        assert!(summary.contains("history  ~"));
        assert!(summary.contains("total  ~"));
        assert!(summary.contains("user messages      1  ~4 tokens"));
        assert!(summary.contains("assistant messages 1  ~4 tokens"));
        assert!(summary.contains("tool calls         1  ~4 tokens"));
        assert!(summary.contains("tool results       1  ~4 tokens"));
        assert!(!summary.contains("secret question"));
        assert!(!summary.contains("secret answer"));
        assert!(!summary.contains("secret arguments"));
        assert!(!summary.contains("secret result"));
    }

    #[test]
    fn raw_shows_every_model_visible_field() {
        let history = vec![
            ChatMessage::User("instructions".into()),
            ChatMessage::Assistant {
                content: "checking".into(),
                thinking: String::new(),
                thinking_signature: String::new(),
                tool_calls: vec![ToolCall {
                    id: "call-1".into(),
                    name: "read".into(),
                    arguments: r#"{"path":"CONTEXT.md"}"#.into(),
                }],
            },
            ChatMessage::Tool {
                id: "call-1".into(),
                content: "contents".into(),
            },
        ];

        let preview = format_history(history);
        assert!(preview.contains("[user]\ninstructions"));
        assert!(preview.contains("[assistant]\nchecking"));
        assert!(preview.contains("[assistant tool call: read · call-1]"));
        assert!(preview.contains(r#"{"path":"CONTEXT.md"}"#));
        assert!(preview.contains("[tool result: call-1]\ncontents"));
    }

    #[test]
    fn history_omits_tool_calls_without_results() {
        let mut interrupted = Message::assistant();
        interrupted.tool_calls.push(ToolCall {
            id: "missing".into(),
            name: "bash".into(),
            arguments: r#"{"command":"echo done"}"#.into(),
        });
        let mut messages = vec![Message::user("question".into()), interrupted];

        let at_end = history(Some("instructions"), None, &messages);
        assert_eq!(at_end.len(), 2);
        assert!(matches!(&at_end[0], ChatMessage::User(text) if text == "instructions"));
        assert!(matches!(&at_end[1], ChatMessage::User(text) if text == "question"));

        messages.push(Message::user("continue".into()));
        let resumed = history(None, None, &messages);
        assert_eq!(resumed.len(), 2);
        assert!(matches!(&resumed[1], ChatMessage::User(text) if text == "continue"));
        assert_eq!(messages[1].tool_calls.len(), 1);
        assert!(!format_history(resumed).contains("missing"));
        let summary = summary(&messages, None, "");
        assert!(summary.contains("assistant messages 0  ~0 tokens"));
        assert!(summary.contains("tool calls         0  ~0 tokens"));
    }

    #[test]
    fn history_keeps_partial_tool_round_and_empty_results() {
        let mut assistant = Message::assistant();
        assistant.text = "checking".into();
        assistant.thinking = "reasoning".into();
        assistant.thinking_signature = "signature".into();
        for id in ["missing", "saved", "empty"] {
            assistant.tool_calls.push(ToolCall {
                id: id.into(),
                name: "bash".into(),
                arguments: "{}".into(),
            });
        }
        let messages = vec![
            Message::user("old question".into()),
            Message::user("question".into()),
            assistant,
            Message::tool("saved".into(), "bash".into(), "done".into()),
            Message::aside_user("side question".into()),
            Message::debug("debug".into(), "headers".into()),
            Message::tool("empty".into(), "bash".into(), String::new()),
            Message::user("continue".into()),
        ];

        let history = history(Some("instructions"), Some(("checkpoint", 1)), &messages);
        assert_eq!(history.len(), 7);
        assert!(matches!(
            &history[3],
            ChatMessage::Assistant { content, thinking, thinking_signature, tool_calls }
                if content == "checking"
                    && thinking == "reasoning"
                    && thinking_signature == "signature"
                    && tool_calls.iter().map(|call| call.id.as_str()).collect::<Vec<_>>()
                        == ["saved", "empty"]
        ));
        assert!(matches!(
            &history[4], ChatMessage::Tool { id, content } if id == "saved" && content == "done"
        ));
        assert!(matches!(
            &history[5], ChatMessage::Tool { id, content } if id == "empty" && content.is_empty()
        ));
        assert_eq!(messages[2].tool_calls.len(), 3);
        let preview = format_history(history);
        assert!(!preview.contains("missing"));
        assert!(!preview.contains("old question"));
        let summary = summary(&messages, Some(("checkpoint", 1)), "");
        assert!(summary.contains("tool calls         2  ~1 tokens"));
        assert!(summary.contains("tool results       2  ~1 tokens"));
    }

    #[test]
    fn history_matches_tool_results_only_in_their_round() {
        let mut interrupted = Message::assistant();
        interrupted.text = "checking".into();
        interrupted.tool_calls.push(ToolCall {
            id: "reused".into(),
            name: "read".into(),
            arguments: "{}".into(),
        });
        let messages = vec![
            interrupted.clone(),
            Message::tool("reused".into(), "read".into(), "first result".into()),
            Message::user("again".into()),
            interrupted.clone(),
            Message::user("continue".into()),
            interrupted,
            Message::tool("reused".into(), "read".into(), "last result".into()),
        ];

        let history = history(None, None, &messages);
        assert_eq!(history.len(), 7);
        for (index, count) in [(0, 1), (3, 0), (5, 1)] {
            assert!(matches!(
                &history[index], ChatMessage::Assistant { content, tool_calls, .. }
                    if content == "checking" && tool_calls.len() == count
            ));
        }
    }

    #[test]
    fn history_keeps_model_message_order_and_tool_data() {
        let mut assistant = Message::assistant();
        assistant.text = "checking".into();
        assistant.tool_calls.push(ToolCall {
            id: "call-1".into(),
            name: "read".into(),
            arguments: r#"{"path":"CONTEXT.md"}"#.into(),
        });
        let messages = vec![
            Message::user("question".into()),
            assistant,
            Message::tool("call-1".into(), "read".into(), "contents".into()),
        ];

        let history = history(Some("instructions"), None, &messages);
        assert_eq!(history.len(), 4);
        assert!(matches!(&history[0], ChatMessage::User(text) if text == "instructions"));
        assert!(matches!(&history[1], ChatMessage::User(text) if text == "question"));
        assert!(matches!(
            &history[2],
            ChatMessage::Assistant { content, tool_calls, .. }
                if content == "checking"
                    && tool_calls[0].id == "call-1"
                    && tool_calls[0].name == "read"
                    && tool_calls[0].arguments == r#"{"path":"CONTEXT.md"}"#
        ));
        assert!(matches!(
            &history[3],
            ChatMessage::Tool { id, content } if id == "call-1" && content == "contents"
        ));
    }
}
