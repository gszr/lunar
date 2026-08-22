//! Append-only model HTTP debug log under `$LUNAR_HOME/debug.log`.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

static LOG: OnceLock<Mutex<std::fs::File>> = OnceLock::new();

pub(crate) fn enable() -> io::Result<()> {
    let home = crate::mission::home();
    fs::create_dir_all(&home)?;
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(home.join("debug.log"))?;
    LOG.set(Mutex::new(file))
        .map_err(|_| io::Error::new(io::ErrorKind::AlreadyExists, "debug log already enabled"))
}

pub(crate) fn event(kind: &str, fields: Value) {
    let Some(log) = LOG.get() else {
        return;
    };
    let mut value = json!({
        "time_ms": SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
        "event": kind,
    });
    if let (Some(dst), Some(src)) = (value.as_object_mut(), fields.as_object()) {
        dst.extend(src.clone());
    }
    if let Ok(mut file) = log.lock() {
        let _ = serde_json::to_writer(&mut *file, &value);
        let _ = file.write_all(b"\n");
    }
}
