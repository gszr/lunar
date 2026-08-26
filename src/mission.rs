//! Linear jsonl missions under $LUNAR_HOME/recorder/missions/ as YYYY-MM-DD-N.jsonl.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::{Value, json};

use crate::app::Message;
use crate::protocol::{Thinking, ToolCall, Usage};

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
    modified: SystemTime,
}

pub enum Selection {
    Mission(Meta),
    Log(Vec<Meta>),
}

pub fn select(items: &[Meta], selector: &str) -> Selection {
    let stem = selector.strip_suffix(".jsonl").unwrap_or(selector);

    if let Some(meta) = items.iter().find(|meta| meta.id == stem) {
        return Selection::Mission(meta.clone());
    }
    if let Some(meta) = items
        .iter()
        .find(|meta| meta.name.as_deref() == Some(selector))
    {
        return Selection::Mission(meta.clone());
    }
    if is_date(selector) {
        return Selection::Log(
            items
                .iter()
                .filter(|meta| meta.id.starts_with(&format!("{selector}-")))
                .cloned()
                .collect(),
        );
    }
    Selection::Log(items.to_vec())
}

pub fn semantic_name(prompt: &str) -> String {
    let line = prompt
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("");
    let clean: String = line
        .chars()
        .map(|ch| {
            if ch.is_alphanumeric() || ch == '-' || ch == '_' || ch.is_whitespace() {
                ch
            } else {
                ' '
            }
        })
        .collect();
    let words: Vec<&str> = clean.split_whitespace().take(6).collect();
    if words.is_empty() {
        return "Untitled Mission".into();
    }
    let mostly_lowercase = words
        .iter()
        .flat_map(|word| word.chars())
        .filter(|ch| ch.is_alphabetic())
        .all(|ch| ch.is_lowercase());
    let mut name = if mostly_lowercase {
        words
            .iter()
            .map(|word| {
                let mut chars = word.chars();
                chars
                    .next()
                    .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        words.join(" ")
    };
    while name.len() > 48 {
        name.pop();
    }
    name.trim_end().to_string()
}

fn is_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(i, byte)| i == 4 || i == 7 || byte.is_ascii_digit())
}

pub struct Loaded {
    pub mission: Mission,
    pub messages: Vec<Message>,
    pub model: Option<(String, String)>,
    pub thinking: Option<Thinking>,
    pub usage: Usage,
    pub last_prompt: u32,
}

pub fn create(name: &str) -> io::Result<Mission> {
    let cwd = std::env::current_dir()?;
    let dir = crate::storage::recorder("missions");
    fs::create_dir_all(&dir)?;
    let id = next_id(&dir)?;
    let path = dir.join(format!("{id}.jsonl"));
    let mission = Mission {
        path,
        id: id.clone(),
        name: Some(name.to_string()),
    };
    append(
        &mission,
        &json!({
            "type": "header",
            "id": id,
            "cwd": cwd.to_string_lossy(),
            "name": name,
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

pub fn set_name(mission: &mut Mission, name: &str) -> io::Result<()> {
    rewrite_header_name(&mission.path, name)?;
    mission.name = Some(name.to_string());
    Ok(())
}

fn rewrite_header_name(path: &Path, name: &str) -> io::Result<()> {
    let contents = fs::read_to_string(path)?;
    let mut lines = contents.lines();
    let first = lines
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing mission header"))?;
    let mut header: Value = serde_json::from_str(first)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    if header.get("type").and_then(Value::as_str) != Some("header") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "missing mission header",
        ));
    }
    header["name"] = Value::String(name.to_string());

    let temp = path.with_extension("jsonl.tmp");
    let mut file = File::create(&temp)?;
    writeln!(file, "{header}")?;
    for line in lines {
        if serde_json::from_str::<Value>(line)
            .ok()
            .and_then(|value| {
                value
                    .get("type")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .as_deref()
            == Some("name")
        {
            continue;
        }
        writeln!(file, "{line}")?;
    }
    file.sync_all()?;
    fs::rename(temp, path)
}

pub fn list() -> io::Result<Vec<Meta>> {
    let cwd = std::env::current_dir()?;
    let dir = crate::storage::recorder("missions");
    let mut items = Vec::new();
    let Ok(entries) = fs::read_dir(&dir) else {
        return Ok(items);
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        if let Ok(mut meta) = read_meta(&path)
            && meta.cwd.as_deref() == Some(cwd.to_string_lossy().as_ref())
        {
            if meta.name.is_none()
                && let Ok(Some(name)) = first_user_name(&path)
            {
                meta.name = Some(name);
            }
            items.push(meta);
        }
    }
    sort_by_modified(&mut items);
    Ok(items)
}

fn sort_by_modified(items: &mut [Meta]) {
    items.sort_by(|a, b| {
        b.modified
            .cmp(&a.modified)
            .then_with(|| match (parse_id(&a.id), parse_id(&b.id)) {
                (Some(x), Some(y)) => y.cmp(&x),
                _ => b.id.cmp(&a.id),
            })
    });
}

pub fn load(path: &Path) -> io::Result<Loaded> {
    let file = File::open(path)?;
    let mut id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("mission")
        .to_string();
    let mut name = None;
    let mut messages = Vec::new();
    let mut model = None;
    let mut thinking = None;
    let mut usage = Usage::default();
    let mut last_prompt = 0;
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
                name = value
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::to_string);
            }
            Some("name") => {
                name = value
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::to_string);
            }
            Some("model") => {
                if let (Some(provider), Some(id)) = (
                    value.get("provider").and_then(Value::as_str),
                    value.get("id").and_then(Value::as_str),
                ) {
                    model = Some((provider.to_string(), id.to_string()));
                }
            }
            Some("thinking") => {
                if let Some(level) = value
                    .get("level")
                    .and_then(Value::as_str)
                    .and_then(Thinking::parse)
                {
                    thinking = Some(level);
                }
            }
            Some("usage") => {
                let item = Usage {
                    input: number(&value, "input"),
                    output: number(&value, "output"),
                    cache_read: number(&value, "cache_read"),
                    cache_write: number(&value, "cache_write"),
                };
                usage.add(item);
                last_prompt = item.prompt();
            }
            Some("user") => {
                if let Some(text) = value.get("text").and_then(Value::as_str) {
                    messages.push(Message::user(text.to_string()));
                }
            }
            Some("assistant") => {
                let mut message = Message::assistant();
                message.text = value
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                message.tool_calls = parse_tool_calls(&value["tool_calls"]);
                messages.push(message);
            }
            Some("tool") => {
                messages.push(Message::tool(
                    value
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    value
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or("tool")
                        .to_string(),
                    value
                        .get("content")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                ));
            }
            _ => {}
        }
    }
    Ok(Loaded {
        mission: Mission {
            path: path.to_path_buf(),
            id,
            name,
        },
        messages,
        model,
        thinking,
        usage,
        last_prompt,
    })
}

pub fn model_line(provider: &str, id: &str) -> Value {
    json!({ "type": "model", "provider": provider, "id": id })
}

pub fn thinking_line(level: Thinking) -> Value {
    json!({ "type": "thinking", "level": level.as_str() })
}

pub fn usage_line(usage: Usage) -> Value {
    json!({
        "type": "usage",
        "input": usage.input,
        "output": usage.output,
        "cache_read": usage.cache_read,
        "cache_write": usage.cache_write,
    })
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

fn first_user_name(path: &Path) -> io::Result<Option<String>> {
    let file = File::open(path)?;
    for line in BufReader::new(file).lines() {
        let value: Value = match serde_json::from_str(&line?) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if value.get("type").and_then(Value::as_str) == Some("user")
            && let Some(text) = value.get("text").and_then(Value::as_str)
        {
            return Ok(Some(semantic_name(text)));
        }
    }
    Ok(None)
}

fn read_meta(path: &Path) -> io::Result<Meta> {
    let modified = fs::metadata(path)?.modified()?;
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
                name = value
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::to_string);
            }
            Some("name") => {
                name = value
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::to_string);
            }
            Some("user" | "assistant" | "tool" | "model" | "thinking") => break,
            _ => {}
        }
    }
    Ok(Meta {
        path: path.to_path_buf(),
        id,
        name,
        cwd,
        modified,
    })
}

fn number(value: &Value, name: &str) -> u32 {
    value
        .get(name)
        .and_then(Value::as_u64)
        .and_then(|value| value.try_into().ok())
        .unwrap_or(0)
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
        match &self.name {
            Some(name) => format!("{} - {name}", self.id),
            None => self.id.clone(),
        }
    }
}

impl Meta {
    pub fn label(&self) -> String {
        format!("mission: {}", self.display_name())
    }

    fn display_name(&self) -> String {
        match &self.name {
            Some(name) => format!("{} - {name}", self.id),
            None => self.id.clone(),
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(id: &str, name: Option<&str>) -> Meta {
        Meta {
            path: PathBuf::from(format!("{id}.jsonl")),
            id: id.into(),
            name: name.map(str::to_string),
            cwd: Some("/work".into()),
            modified: SystemTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn thinking_round_trips() {
        let dir = std::env::temp_dir().join(format!(
            "lunar-mission-thinking-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("2026-08-19-1.jsonl");
        fs::write(
            &path,
            format!(
                "{}\n{}\n",
                json!({"type":"header","id":"2026-08-19-1","name":"Thinking"}),
                thinking_line(Thinking::High)
            ),
        )
        .unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.thinking, Some(Thinking::High));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn load_replays_runtime_state() {
        let dir = std::env::temp_dir().join(format!(
            "lunar-mission-load-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("2026-08-19-1.jsonl");
        let lines = [
            json!({"type":"header","id":"2026-08-19-1","name":"Loaded"}),
            model_line("xai", "grok-old"),
            thinking_line(Thinking::Low),
            user_line("hello"),
            assistant_line("hi", &[]),
            tool_line("call-1", "read", "contents"),
            usage_line(Usage {
                input: 10,
                output: 2,
                cache_read: 3,
                cache_write: 1,
            }),
            model_line("openai", "gpt-current"),
            thinking_line(Thinking::High),
            usage_line(Usage {
                input: 20,
                output: 4,
                cache_read: 5,
                cache_write: 2,
            }),
        ];
        fs::write(
            &path,
            lines
                .iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n")
                + "\n",
        )
        .unwrap();

        let loaded = load(&path).unwrap();

        assert_eq!(loaded.model, Some(("openai".into(), "gpt-current".into())));
        assert_eq!(loaded.thinking, Some(Thinking::High));
        assert_eq!(loaded.usage.input, 30);
        assert_eq!(loaded.usage.output, 6);
        assert_eq!(loaded.usage.cache_read, 8);
        assert_eq!(loaded.usage.cache_write, 3);
        assert_eq!(loaded.last_prompt, 27);
        assert_eq!(loaded.messages.len(), 3);
        assert_eq!(loaded.messages[0].text, "hello");
        assert_eq!(loaded.messages[1].text, "hi");
        assert_eq!(loaded.messages[2].tool_title, "read");
        assert_eq!(loaded.messages[2].text, "contents");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn missions_sort_by_most_recent_activity() {
        let mut older_id_but_newer_activity = meta("2026-08-19-1", None);
        older_id_but_newer_activity.modified =
            SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(2);
        let mut newer_id_but_older_activity = meta("2026-08-20-1", None);
        newer_id_but_older_activity.modified =
            SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1);
        let mut items = vec![newer_id_but_older_activity, older_id_but_newer_activity];

        sort_by_modified(&mut items);

        assert_eq!(items[0].id, "2026-08-19-1");
    }

    #[test]
    fn semantic_name_is_short_and_meaningful() {
        assert_eq!(
            semantic_name("we need a feature to resume a specific mission."),
            "We Need A Feature To Resume"
        );
        assert_eq!(
            semantic_name("# Fix LUNAR_MODEL handling"),
            "Fix LUNAR_MODEL handling"
        );
        assert_eq!(semantic_name("\n\n!!!"), "Untitled Mission");
    }

    #[test]
    fn session_label_combines_id_and_name() {
        assert_eq!(
            meta("2026-08-19-1", Some("Resume Mission Selector")).label(),
            "mission: 2026-08-19-1 - Resume Mission Selector"
        );
    }

    #[test]
    fn select_accepts_id_and_filename() {
        let items = vec![meta("2026-08-19-1", None)];
        for selector in ["2026-08-19-1", "2026-08-19-1.jsonl"] {
            assert!(matches!(select(&items, selector), Selection::Mission(_)));
        }
    }

    #[test]
    fn exact_label_wins_over_date_and_newest_duplicate_wins() {
        let items = vec![
            meta("2026-08-20-2", Some("2026-08-19")),
            meta("2026-08-20-1", Some("2026-08-19")),
            meta("2026-08-19-1", None),
        ];
        match select(&items, "2026-08-19") {
            Selection::Mission(item) => assert_eq!(item.id, "2026-08-20-2"),
            Selection::Log(_) => panic!("label should win"),
        }
    }

    #[test]
    fn date_filters_log_and_unknown_opens_full_log() {
        let items = vec![
            meta("2026-08-20-1", None),
            meta("2026-08-19-2", None),
            meta("2026-08-19-1", None),
        ];
        match select(&items, "2026-08-19") {
            Selection::Log(items) => assert_eq!(items.len(), 2),
            Selection::Mission(_) => panic!("date should open log"),
        }
        match select(&items, "missing") {
            Selection::Log(log) => assert_eq!(log.len(), items.len()),
            Selection::Mission(_) => panic!("unknown should open log"),
        }
    }
}
