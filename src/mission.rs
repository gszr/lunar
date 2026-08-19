//! Linear jsonl missions under $LUNAR_HOME/missions/ as YYYY-MM-DD-N.jsonl.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::complete::ToolCall;

pub struct Mission {
    pub path: PathBuf,
    pub id: String,
    pub name: Option<String>,
}

#[derive(Clone)]
pub struct Meta {
    pub path: PathBuf,
    pub id: String,
    pub name: Option<String>,
    pub cwd: Option<String>,
}

pub enum Saved {
    User(String),
    Assistant {
        text: String,
        tool_calls: Vec<ToolCall>,
    },
    Tool {
        id: String,
        title: String,
        content: String,
    },
}

pub fn home() -> PathBuf {
    if let Ok(dir) = std::env::var("LUNAR_HOME")
        && !dir.is_empty()
    {
        return PathBuf::from(dir);
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".lunar")
}

pub fn create() -> io::Result<Mission> {
    let cwd = std::env::current_dir()?;
    let dir = home().join("missions");
    fs::create_dir_all(&dir)?;
    let id = next_id(&dir)?;
    let path = dir.join(format!("{id}.jsonl"));
    let mission = Mission {
        path,
        id: id.clone(),
        name: None,
    };
    append(
        &mission,
        &json!({
            "type": "header",
            "id": id,
            "cwd": cwd.to_string_lossy(),
        }),
    )?;
    Ok(mission)
}

pub fn append(mission: &Mission, value: &Value) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&mission.path)?;
    writeln!(file, "{value}")?;
    Ok(())
}

pub fn list() -> io::Result<Vec<Meta>> {
    let cwd = std::env::current_dir()?;
    let dir = home().join("missions");
    let mut items = Vec::new();
    let Ok(entries) = fs::read_dir(&dir) else {
        return Ok(items);
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        if let Ok(meta) = read_meta(&path)
            && meta.cwd.as_deref() == Some(cwd.to_string_lossy().as_ref())
        {
            items.push(meta);
        }
    }
    items.sort_by(|a, b| match (parse_id(&a.id), parse_id(&b.id)) {
        (Some(x), Some(y)) => y.cmp(&x),
        _ => b.id.cmp(&a.id),
    });
    Ok(items)
}

pub fn load(path: &Path) -> io::Result<(Mission, Vec<Saved>)> {
    let file = File::open(path)?;
    let mut id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("mission")
        .to_string();
    let mut name = None;
    let mut saved = Vec::new();
    for line in BufReader::new(file).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(&line)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        match value.get("type").and_then(Value::as_str) {
            Some("header") => {
                if let Some(h) = value.get("id").and_then(Value::as_str) {
                    id = h.to_string();
                }
            }
            Some("name") => {
                name = value
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::to_string);
            }
            Some("user") => {
                if let Some(text) = value.get("text").and_then(Value::as_str) {
                    saved.push(Saved::User(text.to_string()));
                }
            }
            Some("assistant") => {
                saved.push(Saved::Assistant {
                    text: value
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    tool_calls: parse_tool_calls(&value["tool_calls"]),
                });
            }
            Some("tool") => {
                saved.push(Saved::Tool {
                    id: value
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    title: value
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or("tool")
                        .to_string(),
                    content: value
                        .get("content")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                });
            }
            _ => {}
        }
    }
    Ok((
        Mission {
            path: path.to_path_buf(),
            id,
            name,
        },
        saved,
    ))
}

pub fn user_line(text: &str) -> Value {
    json!({ "type": "user", "text": text })
}

pub fn assistant_line(text: &str, tool_calls: &[ToolCall]) -> Value {
    json!({
        "type": "assistant",
        "text": text,
        "tool_calls": tool_calls.iter().map(|c| json!({
            "id": c.id,
            "name": c.name,
            "arguments": c.arguments,
        })).collect::<Vec<_>>(),
    })
}

pub fn tool_line(id: &str, title: &str, content: &str) -> Value {
    json!({ "type": "tool", "id": id, "title": title, "content": content })
}

fn read_meta(path: &Path) -> io::Result<Meta> {
    let file = File::open(path)?;
    let mut id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("mission")
        .to_string();
    let mut name = None;
    let mut cwd = None;
    for line in BufReader::new(file).lines() {
        let line = line?;
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        match value.get("type").and_then(Value::as_str) {
            Some("header") => {
                if let Some(h) = value.get("id").and_then(Value::as_str) {
                    id = h.to_string();
                }
                cwd = value.get("cwd").and_then(Value::as_str).map(str::to_string);
            }
            Some("name") => {
                name = value
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::to_string);
            }
            Some("user" | "assistant" | "tool") => break,
            _ => {}
        }
    }
    Ok(Meta {
        path: path.to_path_buf(),
        id,
        name,
        cwd,
    })
}

fn parse_tool_calls(value: &Value) -> Vec<ToolCall> {
    let Some(items) = value.as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .map(|v| ToolCall {
            id: v
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            name: v
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            arguments: v
                .get("arguments")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        })
        .collect()
}

impl Mission {
    pub fn label(&self) -> String {
        format!("mission: {}", self.display_name())
    }

    fn display_name(&self) -> String {
        self.name
            .clone()
            .unwrap_or_else(|| format!("{}.jsonl", self.id))
    }
}

impl Meta {
    pub fn label(&self) -> String {
        format!("mission: {}", self.display_name())
    }

    fn display_name(&self) -> String {
        self.name
            .clone()
            .unwrap_or_else(|| format!("{}.jsonl", self.id))
    }
}

fn next_id(dir: &Path) -> io::Result<String> {
    let today = today();
    let mut max = 0u32;
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if let Some((date, n)) = parse_id(stem)
                && date == today
            {
                max = max.max(n);
            }
        }
    }
    Ok(format!("{today}-{}", max + 1))
}

fn parse_id(id: &str) -> Option<(&str, u32)> {
    let (date, n) = id.rsplit_once('-')?;
    if date.len() != 10 {
        return None;
    }
    Some((date, n.parse().ok()?))
}

fn today() -> String {
    let output = std::process::Command::new("date").arg("+%Y-%m-%d").output();
    match output {
        Ok(out) => {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if s.len() == 10 {
                s
            } else {
                "1970-01-01".into()
            }
        }
        Err(_) => "1970-01-01".into(),
    }
}
