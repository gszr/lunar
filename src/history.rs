//! Global submitted-line history under $LUNAR_HOME/history.jsonl.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};

use serde_json::{Value, json};

use crate::mission;

pub fn load() -> io::Result<Vec<String>> {
    let path = mission::home().join("history.jsonl");
    let Ok(file) = File::open(path) else {
        return Ok(Vec::new());
    };
    let mut lines = Vec::new();
    for line in BufReader::new(file).lines() {
        let line = line?;
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if let Some(text) = value.get("text").and_then(Value::as_str) {
            lines.push(text.to_string());
        }
    }
    Ok(lines)
}

pub fn append(text: &str) -> io::Result<()> {
    let home = mission::home();
    fs::create_dir_all(&home)?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(home.join("history.jsonl"))?;
    writeln!(file, "{}", json!({ "text": text }))
}
