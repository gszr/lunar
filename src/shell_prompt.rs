//! Explicit shell commands expanded into user prompts before sending.

use std::sync::atomic::{AtomicBool, Ordering};

fn command(line: &str) -> Option<&str> {
    line.trim_start()
        .strip_prefix('!')
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

pub(crate) fn contains_commands(prompt: &str) -> bool {
    prompt.lines().any(|line| command(line).is_some())
}

pub(crate) fn expand(prompt: &str, cancel: &AtomicBool) -> Option<String> {
    let mut expanded = String::new();
    for line in prompt.split_inclusive('\n') {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        if let Some(command) = command(line) {
            let args = serde_json::json!({ "command": command }).to_string();
            let output = crate::tools::run("bash", &args, cancel);
            if cancel.load(Ordering::Relaxed) {
                return None;
            }
            let longest_run = output
                .content
                .split(|c| c != '`')
                .map(str::len)
                .max()
                .unwrap_or(0);
            let fence = "`".repeat(3.max(longest_run + 1));
            expanded.push_str(&format!(
                "User ran `{command}` command:\n\n{fence}text\n{}",
                output.content
            ));
            if !output.content.ends_with('\n') {
                expanded.push('\n');
            }
            expanded.push_str(&fence);
            if line.ends_with('\n') {
                expanded.push('\n');
            }
        } else {
            expanded.push_str(line);
        }
    }
    Some(expanded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_commands_in_place_without_expanding_their_output() {
        let cancel = AtomicBool::new(false);
        let result = expand(
            "Explain:\n  ! printf '! echo untouched'\nThen:\n! printf second\nDone.",
            &cancel,
        )
        .unwrap();
        assert_eq!(
            result,
            "Explain:\nUser ran `printf '! echo untouched'` command:\n\n```text\n! echo untouched\n```\nThen:\nUser ran `printf second` command:\n\n```text\nsecond\n```\nDone."
        );
    }

    #[test]
    fn output_backticks_cannot_close_the_fence() {
        let result = expand("! printf '````\\n'", &AtomicBool::new(false)).unwrap();
        assert!(result.ends_with("`````text\n````\n`````"));
    }

    #[test]
    fn preserves_plain_text_and_empty_bang() {
        let prompt = "Hello! inline ! echo no\n!\n";
        assert!(!contains_commands(prompt));
        assert_eq!(expand(prompt, &AtomicBool::new(false)).unwrap(), prompt);
    }

    #[test]
    fn includes_command_failures() {
        let result = expand("! printf failure >&2; exit 7", &AtomicBool::new(false)).unwrap();
        assert!(result.contains("stderr:\nfailure"));
        assert!(result.contains("exit status: 7"));
    }

    #[test]
    fn cancellation_does_not_return_a_prompt() {
        assert!(expand("! touch should-not-exist", &AtomicBool::new(true)).is_none());
    }
}
