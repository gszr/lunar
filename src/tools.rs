//! Built-in tools. Pi-shaped: read, write, edit, bash.

use std::fmt::Write as FmtWrite;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const DEFAULT_READ_LIMIT: usize = 2000;
const DEFAULT_BASH_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_BASH_BYTES: usize = 64 * 1024;

pub fn definitions() -> Value {
    json!([
        {
            "type": "function",
            "function": {
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
            }
        },
        {
            "type": "function",
            "function": {
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
            }
        },
        {
            "type": "function",
            "function": {
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
            }
        },
        {
            "type": "function",
            "function": {
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
            }
        }
    ])
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
    for (i, line) in lines[start..end].iter().enumerate() {
        let _ = writeln!(&mut out, "{:>6}|{line}", start + i + 1);
    }
    if end < lines.len() {
        let _ = writeln!(
            &mut out,
            "… {} more lines (use offset/limit to continue)",
            lines.len() - end
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

    let mut child = match Command::new("bash")
        .arg("-lc")
        .arg(command)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
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
            let _ = child.kill();
            let _ = child.wait();
            let _ = out_h.join();
            let _ = err_h.join();
            return ToolOut {
                title,
                content: "aborted".into(),
            };
        }
        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
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
    let mut content = String::new();
    if !stdout.is_empty() {
        content.push_str(&truncate(&stdout));
    }
    if !stderr.is_empty() {
        if !content.is_empty() {
            content.push('\n');
        }
        content.push_str("stderr:\n");
        content.push_str(&truncate(&stderr));
    }
    if !status.success() {
        if !content.is_empty() {
            content.push('\n');
        }
        content.push_str(&format!("exit {status}"));
    }
    if content.is_empty() {
        content = "(no output)".into();
    }
    ToolOut { title, content }
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

fn truncate(text: &str) -> String {
    let bytes = text.as_bytes();
    if bytes.len() <= MAX_BASH_BYTES {
        return text.to_string();
    }
    let start = bytes.len() - MAX_BASH_BYTES;
    let start = text
        .char_indices()
        .find(|(i, _)| *i >= start)
        .map(|(i, _)| i)
        .unwrap_or(start);
    format!("…(truncated)\n{}", &text[start..])
}
