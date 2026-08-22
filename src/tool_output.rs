//! Bounded tool output retained under `$LUNAR_HOME/tool-output`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub(crate) const MAX_BYTES: usize = 50 * 1024;
pub(crate) const MAX_LINES: usize = 2000;
const RETENTION: Duration = Duration::from_secs(7 * 24 * 60 * 60);
static NEXT_ID: AtomicU64 = AtomicU64::new(0);

pub(crate) fn save(content: &str) -> io::Result<PathBuf> {
    save_in(&crate::mission::home().join("tool-output"), content)
}

fn save_in(dir: &Path, content: &str) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let path = dir.join(format!("tool-{stamp}-{}-{id}", std::process::id()));
    fs::write(&path, content)?;
    Ok(path)
}

pub(crate) fn cleanup() {
    cleanup_in(
        &crate::mission::home().join("tool-output"),
        SystemTime::now(),
    );
}

fn cleanup_in(dir: &Path, now: SystemTime) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let Ok(modified) = meta.modified() else {
            continue;
        };
        if now
            .duration_since(modified)
            .is_ok_and(|age| age > RETENTION)
        {
            let _ = fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
fn set_modified(path: &Path, time: SystemTime) {
    let Ok(file) = fs::File::open(path) else {
        return;
    };
    let _ = file.set_modified(time);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "lunar-{name}-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn saves_full_output() {
        let dir = temp_dir("tool-output-save");
        let path = save_in(&dir, "full output").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "full output");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn cleanup_removes_only_old_files() {
        let dir = temp_dir("tool-output-cleanup");
        let old = save_in(&dir, "old").unwrap();
        let recent = save_in(&dir, "recent").unwrap();
        let now = SystemTime::now();
        set_modified(&old, now - RETENTION - Duration::from_secs(1));
        set_modified(&recent, now);
        cleanup_in(&dir, now);
        assert!(!old.exists());
        assert!(recent.exists());
        let _ = fs::remove_dir_all(dir);
    }
}
