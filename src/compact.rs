//! Pi-style manual context compaction.

use std::fmt::Write as _;

use crate::app::{Message, Role};

pub(crate) const KEEP_RECENT_TOKENS: usize = 20_000;
const TOOL_RESULT_MAX_CHARS: usize = 2_000;

pub(crate) struct Preparation {
    pub(crate) prompt: String,
    pub(crate) first_kept: usize,
    pub(crate) tokens_before: u32,
}

pub(crate) fn prepare(
    messages: &[Message],
    boundary_start: usize,
    previous_summary: Option<&str>,
    instructions: Option<&str>,
) -> Option<Preparation> {
    if boundary_start >= messages.len() {
        return None;
    }
    let first_kept = cut_point(messages, boundary_start, KEEP_RECENT_TOKENS);
    if first_kept <= boundary_start {
        return None;
    }

    let conversation = serialize(&messages[boundary_start..first_kept]);
    if conversation.is_empty() {
        return None;
    }
    let task = if previous_summary.is_some() {
        UPDATE_PROMPT
    } else {
        INITIAL_PROMPT
    };
    let mut prompt = format!("<conversation>\n{conversation}\n</conversation>\n\n");
    if let Some(summary) = previous_summary {
        let _ = write!(
            prompt,
            "<previous-summary>\n{summary}\n</previous-summary>\n\n"
        );
    }
    prompt.push_str(task);
    if let Some(instructions) = instructions.filter(|text| !text.trim().is_empty()) {
        let _ = write!(prompt, "\n\nAdditional focus: {}", instructions.trim());
    }

    let tokens_before = estimate_slice(&messages[boundary_start..])
        + previous_summary.map(estimate_text).unwrap_or(0);
    Some(Preparation {
        prompt,
        first_kept,
        tokens_before: tokens_before.min(u32::MAX as usize) as u32,
    })
}

fn cut_point(messages: &[Message], start: usize, keep_tokens: usize) -> usize {
    let valid: Vec<usize> = (start..messages.len())
        .filter(|&i| !matches!(messages[i].role, Role::Tool))
        .collect();
    let Some(&first) = valid.first() else {
        return start;
    };
    let mut accumulated = 0;
    let mut cut = first;
    for i in (start..messages.len()).rev() {
        accumulated += estimate_message(&messages[i]);
        if accumulated >= keep_tokens {
            cut = valid
                .iter()
                .copied()
                .find(|&candidate| candidate >= i)
                .unwrap_or(first);
            break;
        }
    }
    cut
}

pub(crate) fn estimate_context(summary: &str, messages: &[Message]) -> u32 {
    (estimate_text(summary) + estimate_slice(messages)).min(u32::MAX as usize) as u32
}

fn estimate_slice(messages: &[Message]) -> usize {
    messages.iter().map(estimate_message).sum()
}

fn estimate_message(message: &Message) -> usize {
    let chars = message.text.chars().count()
        + message
            .tool_calls
            .iter()
            .map(|call| call.name.chars().count() + call.arguments.chars().count())
            .sum::<usize>();
    chars.div_ceil(4) + 4
}

fn estimate_text(text: &str) -> usize {
    text.chars().count().div_ceil(4)
}

fn serialize(messages: &[Message]) -> String {
    let mut parts = Vec::new();
    for message in messages {
        match message.role {
            Role::User => parts.push(format!("[User]: {}", message.text)),
            Role::Assistant => {
                if !message.text.is_empty() {
                    parts.push(format!("[Assistant]: {}", message.text));
                }
                if !message.tool_calls.is_empty() {
                    let calls = message
                        .tool_calls
                        .iter()
                        .map(|call| format!("{}({})", call.name, call.arguments))
                        .collect::<Vec<_>>()
                        .join("; ");
                    parts.push(format!("[Assistant tool calls]: {calls}"));
                }
            }
            Role::Tool => {
                let text = truncate_chars(&message.text, TOOL_RESULT_MAX_CHARS);
                parts.push(format!("[Tool result: {}]: {text}", message.tool_title));
            }
        }
    }
    parts.join("\n\n")
}

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let kept: String = text.chars().take(max).collect();
    format!("{kept}\n\n[... tool result truncated]")
}

const INITIAL_PROMPT: &str = r#"Create a structured context checkpoint summary that another LLM will use to continue the work.
Do not continue the conversation or answer questions from it. Output only this format:

## Goal
[What the user is trying to accomplish]

## Constraints & Preferences
- [Requirements and preferences, or "(none)"]

## Progress
### Done
- [x] [Completed work]
### In Progress
- [ ] [Current work]
### Blocked
- [Blockers, if any]

## Key Decisions
- **[Decision]**: [Rationale]

## Next Steps
1. [What should happen next]

## Critical Context
- [Details needed to continue, or "(none)"]

Keep each section concise. Preserve exact file paths, function names, commands, and error messages."#;

const UPDATE_PROMPT: &str = r#"Update the existing structured summary with the new conversation messages.
Do not continue the conversation or answer questions from it. Output only the same structured format.
Preserve still-relevant goals, constraints, decisions, exact file paths, function names, commands, and error messages. Add new progress and context, move completed work to Done, remove resolved blockers, and update Next Steps. Keep each section concise."#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::ToolCall;

    #[test]
    fn keeps_recent_messages_and_never_starts_at_a_tool_result() {
        let mut messages = vec![Message::user("old".repeat(10_000))];
        let mut assistant = Message::assistant();
        assistant.tool_calls.push(ToolCall {
            id: "1".into(),
            name: "read".into(),
            arguments: "{}".into(),
        });
        messages.push(assistant);
        messages.push(Message::tool(
            "1".into(),
            "read".into(),
            "x".repeat(100_000),
        ));
        messages.push(Message::user("recent".into()));

        let prepared = prepare(&messages, 0, None, None).unwrap();
        assert_eq!(prepared.first_kept, 3);
        assert!(!matches!(messages[prepared.first_kept].role, Role::Tool));
        assert!(prepared.prompt.contains("[User]: old"));
    }

    #[test]
    fn repeated_compaction_updates_previous_summary() {
        let messages = vec![
            Message::user("boundary".into()),
            Message::user("older".into()),
            Message::user("middle".repeat(20_000)),
            Message::user("new".repeat(20_000)),
        ];
        let prepared = prepare(
            &messages,
            1,
            Some("previous checkpoint"),
            Some("focus on tests"),
        )
        .unwrap();
        assert!(
            prepared
                .prompt
                .contains("<previous-summary>\nprevious checkpoint")
        );
        assert!(prepared.prompt.contains("Additional focus: focus on tests"));
    }
}
