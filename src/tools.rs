//! Built-in tools. Pi-shaped: read, write, edit, bash.

use std::fmt::Write as FmtWrite;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::tool_output::{MAX_BYTES as MAX_TOOL_BYTES, MAX_LINES as MAX_TOOL_LINES};

const DEFAULT_READ_LIMIT: usize = MAX_TOOL_LINES;
const DEFAULT_BASH_TIMEOUT: Duration = Duration::from_secs(60);

pub fn responses_definitions() -> Value {
    Value::Array(
        definitions_list()
            .into_iter()
            .map(|mut definition| {
                definition
                    .as_object_mut()
                    .unwrap()
                    .insert("type".into(), json!("function"));
                definition
            })
            .collect(),
    )
}

pub fn completions_definitions() -> Value {
    Value::Array(
        definitions_list()
            .into_iter()
            .map(|definition| {
                json!({
                    "type": "function",
                    "function": definition,
                })
            })
            .collect(),
    )
}

fn definitions_list() -> Vec<Value> {
    vec![
        json!({
            "name": "read",
            "description": "Read file contents. Use this instead of cat or sed.",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the file to read (relative or absolute)" },
                    "offset": { "type": "integer", "description": "Line number to start reading from (1-indexed)" },
                    "limit": { "type": "integer", "description": "Maximum number of lines to read" }
                },
                "required": ["path"]
            }
        }),
        json!({
            "name": "write",
            "description": "Write content to a file. Creates the file if it doesn't exist, overwrites if it does. Automatically creates parent directories.",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the file to write (relative or absolute)" },
                    "content": { "type": "string", "description": "Content to write to the file" }
                },
                "required": ["path", "content"]
            }
        }),
        json!({
            "name": "edit",
            "description": "Edit a file by replacing one unique string with another.",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the file to edit (relative or absolute)" },
                    "old_string": { "type": "string", "description": "Exact text to find. Must appear exactly once." },
                    "new_string": { "type": "string", "description": "Replacement text." }
                },
                "required": ["path", "old_string", "new_string"]
            }
        }),
        json!({
            "name": "bash",
            "description": "Execute a bash command in the current working directory. Returns stdout and stderr.",
            "parameters": {
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "Bash command to execute" },
                    "timeout": { "type": "integer", "description": "Timeout in seconds (optional)" }
                },
                "required": ["command"]
            }
        }),
    ]
}

pub struct ToolOut {
    pub title: String,
    pub content: String,
}

pub fn run(name: &str, arguments: &str, cancel: &AtomicBool) -> ToolOut {
    if cancel.load(Ordering::Relaxed) {
        return ToolOut {
            title: name.into(),
            content: "aborted".into(),
        };
    }
    let args: Value = match serde_json::from_str(arguments) {
        Ok(v) => v,
        Err(err) => {
            return ToolOut {
                title: name.into(),
                content: format!("invalid arguments: {err}"),
            };
        }
    };
    match name {
        "read" => read(&args),
        "write" => write(&args),
        "edit" => edit(&args),
        "bash" => bash(&args, cancel),
        other => ToolOut {
            title: other.into(),
            content: format!("unknown tool: {other}"),
        },
    }
}

fn read(args: &Value) -> ToolOut {
    let Some(path) = args["path"].as_str() else {
        return ToolOut {
            title: "read".into(),
            content: "missing path".into(),
        };
    };
    let resolved = resolve(path);
    let title = format!("read {}", display(&resolved));
    let raw = match std::fs::read(&resolved) {
        Ok(b) => b,
        Err(err) => {
            return ToolOut {
                title,
                content: err.to_string(),
            };
        }
    };
    if raw.contains(&0) {
        return ToolOut {
            title,
            content: format!("binary file ({} bytes)", raw.len()),
        };
    }
    let Ok(text) = String::from_utf8(raw) else {
        return ToolOut {
            title,
            content: "file is not valid UTF-8".into(),
        };
    };
    let lines: Vec<&str> = text.lines().collect();
    let offset = args["offset"].as_u64().unwrap_or(1).max(1) as usize;
    let limit = args["limit"]
        .as_u64()
        .map(|n| n as usize)
        .unwrap_or(DEFAULT_READ_LIMIT);
    let start = offset.saturating_sub(1).min(lines.len());
    let end = (start + limit).min(lines.len());
    let mut out = String::new();
    let mut used = start;
    for (i, line) in lines[start..end].iter().enumerate() {
        let numbered = format!("{:>6}|{line}\n", start + i + 1);
        if !out.is_empty() && out.len() + numbered.len() > MAX_TOOL_BYTES {
            break;
        }
        if numbered.len() > MAX_TOOL_BYTES && out.is_empty() {
            let take = char_prefix(&numbered, MAX_TOOL_BYTES);
            out.push_str(take);
            out.push('\n');
            used = start + i + 1;
            let _ = writeln!(&mut out, "… truncated at {MAX_TOOL_BYTES} bytes");
            break;
        }
        out.push_str(&numbered);
        used = start + i + 1;
    }
    if used < lines.len() {
        let next = used + 1;
        let _ = writeln!(
            &mut out,
            "… {} more lines (use offset={next} to continue)",
            lines.len() - used
        );
    }
    if out.is_empty() {
        out = "(empty)".into();
    }
    ToolOut {
        title,
        content: out,
    }
}

fn write(args: &Value) -> ToolOut {
    let Some(path) = args["path"].as_str() else {
        return ToolOut {
            title: "write".into(),
            content: "missing path".into(),
        };
    };
    let Some(content) = args["content"].as_str() else {
        return ToolOut {
            title: format!("write {path}"),
            content: "missing content".into(),
        };
    };
    let resolved = resolve(path);
    let title = format!("write {}", display(&resolved));
    if let Some(parent) = resolved.parent()
        && let Err(err) = std::fs::create_dir_all(parent)
    {
        return ToolOut {
            title,
            content: err.to_string(),
        };
    }
    match std::fs::write(&resolved, content) {
        Ok(()) => ToolOut {
            title,
            content: format!("wrote {} bytes", content.len()),
        },
        Err(err) => ToolOut {
            title,
            content: err.to_string(),
        },
    }
}

fn edit(args: &Value) -> ToolOut {
    let Some(path) = args["path"].as_str() else {
        return ToolOut {
            title: "edit".into(),
            content: "missing path".into(),
        };
    };
    let Some(old) = args["old_string"].as_str() else {
        return ToolOut {
            title: format!("edit {path}"),
            content: "missing old_string".into(),
        };
    };
    let Some(new) = args["new_string"].as_str() else {
        return ToolOut {
            title: format!("edit {path}"),
            content: "missing new_string".into(),
        };
    };
    let resolved = resolve(path);
    let title = format!("edit {}", display(&resolved));
    if old.is_empty() {
        return ToolOut {
            title,
            content: "old_string is empty".into(),
        };
    }
    let text = match std::fs::read_to_string(&resolved) {
        Ok(t) => t,
        Err(err) => {
            return ToolOut {
                title,
                content: err.to_string(),
            };
        }
    };
    let matches = text.matches(old).count();
    if matches == 0 {
        return ToolOut {
            title,
            content: "old_string not found".into(),
        };
    }
    if matches > 1 {
        return ToolOut {
            title,
            content: format!("old_string is not unique ({matches} matches)"),
        };
    }
    let updated = text.replacen(old, new, 1);
    match std::fs::write(&resolved, updated) {
        Ok(()) => ToolOut {
            title,
            content: "ok".into(),
        },
        Err(err) => ToolOut {
            title,
            content: err.to_string(),
        },
    }
}

fn bash(args: &Value, cancel: &AtomicBool) -> ToolOut {
    let Some(command) = args["command"].as_str() else {
        return ToolOut {
            title: "bash".into(),
            content: "missing command".into(),
        };
    };
    let title = format!("bash {command}");
    let timeout = args["timeout"]
        .as_u64()
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_BASH_TIMEOUT);

    let mut cmd = Command::new("bash");
    cmd.arg("-lc")
        .arg(command)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        // New session, not just a new group: group leaders still have the
        // glass as controlling tty, so a nested TUI can take raw mode.
        use std::os::unix::process::CommandExt;
        // Safety: setsid is async-signal-safe; this runs between fork and exec.
        unsafe {
            cmd.pre_exec(|| {
                if setsid() == -1 {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
        }
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(err) => {
            return ToolOut {
                title,
                content: err.to_string(),
            };
        }
    };

    let mut stdout = child.stdout.take().expect("piped");
    let mut stderr = child.stderr.take().expect("piped");
    let out_h = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let err_h = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf);
        buf
    });

    let start = Instant::now();
    let status = loop {
        if cancel.load(Ordering::Relaxed) {
            stop_child(&mut child);
            let _ = out_h.join();
            let _ = err_h.join();
            return ToolOut {
                title,
                content: "aborted".into(),
            };
        }
        if start.elapsed() >= timeout {
            stop_child(&mut child);
            let _ = out_h.join();
            let _ = err_h.join();
            return ToolOut {
                title,
                content: format!("timed out after {}s", timeout.as_secs()),
            };
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(err) => {
                let _ = out_h.join();
                let _ = err_h.join();
                return ToolOut {
                    title,
                    content: err.to_string(),
                };
            }
        }
    };

    let stdout = String::from_utf8_lossy(&out_h.join().unwrap_or_default()).into_owned();
    let stderr = String::from_utf8_lossy(&err_h.join().unwrap_or_default()).into_owned();
    let mut full = String::new();
    if !stdout.is_empty() {
        full.push_str(&stdout);
    }
    if !stderr.is_empty() {
        if !full.is_empty() {
            full.push('\n');
        }
        full.push_str("stderr:\n");
        full.push_str(&stderr);
    }
    if !status.success() {
        if !full.is_empty() {
            full.push('\n');
        }
        full.push_str(&format!("exit {status}"));
    }
    let mut content = if full.is_empty() {
        "(no output)".into()
    } else {
        let (bounded, truncated) = truncate_tail(&full);
        if truncated {
            match crate::tool_output::save(&full) {
                Ok(path) => format!(
                    "… output truncated; full content saved to {} …\n{bounded}",
                    path.display()
                ),
                Err(_) => format!("… output truncated …\n{bounded}"),
            }
        } else {
            bounded
        }
    };
    if content.is_empty() {
        content = "(no output)".into();
    }
    ToolOut { title, content }
}

#[cfg(unix)]
fn setsid() -> i32 {
    unsafe extern "C" {
        fn setsid() -> i32;
    }
    unsafe { setsid() }
}

fn stop_child(child: &mut Child) {
    #[cfg(unix)]
    {
        let _ = Command::new("kill")
            .arg("-KILL")
            .arg(format!("-{}", child.id()))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn resolve(path: &str) -> PathBuf {
    let p = Path::new(path);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(p))
            .unwrap_or_else(|_| p.to_path_buf())
    }
}

fn display(path: &Path) -> String {
    match std::env::current_dir() {
        Ok(cwd) => path
            .strip_prefix(&cwd)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| path.display().to_string()),
        Err(_) => path.display().to_string(),
    }
}

fn char_prefix(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn truncate_tail(text: &str) -> (String, bool) {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= MAX_TOOL_LINES && text.len() <= MAX_TOOL_BYTES {
        return (text.to_string(), false);
    }
    let mut kept = Vec::new();
    let mut bytes = 0;
    for line in lines.iter().rev().take(MAX_TOOL_LINES) {
        let needed = line.len() + usize::from(!kept.is_empty());
        if bytes + needed > MAX_TOOL_BYTES {
            break;
        }
        kept.push(*line);
        bytes += needed;
    }
    kept.reverse();
    (kept.join("\n"), true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn protocol_definitions_share_the_same_tools() {
        let completions = completions_definitions();
        let responses = responses_definitions();
        let completions = completions.as_array().unwrap();
        let responses = responses.as_array().unwrap();

        assert_eq!(completions.len(), 4);
        assert_eq!(responses.len(), 4);
        for (completion, response) in completions.iter().zip(responses) {
            assert_eq!(completion["type"], "function");
            assert_eq!(response["type"], "function");
            assert_eq!(completion["function"]["name"], response["name"]);
            assert_eq!(
                completion["function"]["description"],
                response["description"]
            );
            assert_eq!(completion["function"]["parameters"], response["parameters"]);
            assert!(response.get("function").is_none());
        }
    }

    #[test]
    fn bash_stdin_is_not_a_tty() {
        let out = run(
            "bash",
            r#"{"command":"if [ -t 0 ]; then echo stdin_tty; else echo stdin_notty; fi"}"#,
            &AtomicBool::new(false),
        );
        assert!(out.content.contains("stdin_notty"), "{}", out.content);
    }

    #[cfg(unix)]
    #[test]
    fn bash_has_no_controlling_tty() {
        let out = run(
            "bash",
            r#"{"command":"if (exec 3>/dev/tty) 2>/dev/null; then echo has_ctty; else echo no_ctty; fi"}"#,
            &AtomicBool::new(false),
        );
        assert!(out.content.contains("no_ctty"), "{}", out.content);
    }

    #[cfg(unix)]
    #[test]
    fn bash_abort_kills_sleep() {
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let start = Instant::now();
        let handle = thread::spawn(move || run("bash", r#"{"command":"sleep 30"}"#, flag.as_ref()));
        thread::sleep(Duration::from_millis(80));
        cancel.store(true, Ordering::Relaxed);
        let out = handle.join().unwrap();
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "{:?}",
            start.elapsed()
        );
        assert_eq!(out.content, "aborted");
    }

    #[test]
    fn read_caps_a_long_line() {
        let path = std::env::temp_dir().join(format!("lunar-read-cap-{}", std::process::id()));
        std::fs::write(&path, "x".repeat(80_000)).unwrap();
        let out = run(
            "read",
            &format!(r#"{{"path":"{}"}}"#, path.display()),
            &AtomicBool::new(false),
        );
        let _ = std::fs::remove_file(&path);
        assert!(
            out.content.len() < MAX_TOOL_BYTES + 80,
            "{}",
            out.content.len()
        );
        assert!(out.content.contains("truncated") || out.content.contains("more lines"));
    }

    #[test]
    fn bash_keeps_the_tail() {
        let full = (0..2500)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let (bounded, truncated) = truncate_tail(&full);
        assert!(truncated);
        assert!(bounded.ends_with("2499"), "{bounded}");
        assert!(!bounded.starts_with("0\n"), "{bounded}");
        assert!(bounded.lines().count() <= MAX_TOOL_LINES);
        assert!(bounded.len() <= MAX_TOOL_BYTES);
    }
}
