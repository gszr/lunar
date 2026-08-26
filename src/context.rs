//! Model-visible context assembly and preview.

use std::fmt::Write as _;

use crate::app::{Message, Role};
use crate::prompt;
use crate::protocol::ChatMessage;

/// Build the semantic message history sent to either model protocol.
pub(crate) fn history(preamble: Option<&str>, messages: &[Message]) -> Vec<ChatMessage> {
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

/// Summarize the context known before the next user message.
pub(crate) fn summary(messages: &[Message]) -> String {
    let (mut out, preamble_tokens) = prompt::summary();
    let mut users = 0;
    let mut assistants = 0;
    let mut tool_calls = 0;
    let mut tool_results = 0;
    let mut user_chars = 0;
    let mut assistant_chars = 0;
    let mut tool_call_chars = 0;
    let mut tool_result_chars = 0;
    for message in messages {
        match message.role {
            Role::User => {
                users += 1;
                user_chars += message.text.chars().count();
            }
            Role::Assistant => {
                assistants +=
                    usize::from(!message.text.is_empty() || !message.tool_calls.is_empty());
                assistant_chars += message.text.chars().count();
                tool_calls += message.tool_calls.len();
                tool_call_chars += message
                    .tool_calls
                    .iter()
                    .map(|call| call.arguments.chars().count())
                    .sum::<usize>();
            }
            Role::Tool => {
                tool_results += 1;
                tool_result_chars += message.text.chars().count();
            }
        }
    }
    let user_tokens = user_chars.div_ceil(4);
    let assistant_tokens = assistant_chars.div_ceil(4);
    let tool_call_tokens = tool_call_chars.div_ceil(4);
    let tool_result_tokens = tool_result_chars.div_ceil(4);
    let total_tokens = user_tokens + assistant_tokens + tool_call_tokens + tool_result_tokens;
    let _ = write!(
        out,
        "\n\nhistory  ~{total_tokens} tokens\n  user messages      {users}  ~{user_tokens} tokens\n  assistant messages {assistants}  ~{assistant_tokens} tokens\n  tool calls         {tool_calls}  ~{tool_call_tokens} tokens\n  tool results       {tool_results}  ~{tool_result_tokens} tokens\n\ntotal  ~{} tokens",
        preamble_tokens + total_tokens
    );
    out
}

/// Display the complete context known before the next user message.
pub(crate) fn raw(messages: &[Message]) -> String {
    let preamble = prompt::preamble();
    let history = history(preamble.as_deref(), messages);
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

        let summary = summary(&messages);
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

        let history = history(Some("instructions"), &messages);
        assert_eq!(history.len(), 4);
        assert!(matches!(&history[0], ChatMessage::User(text) if text == "instructions"));
        assert!(matches!(&history[1], ChatMessage::User(text) if text == "question"));
        assert!(matches!(
            &history[2],
            ChatMessage::Assistant { content, tool_calls }
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
