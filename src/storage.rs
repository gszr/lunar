use std::fs;
use std::path::{Path, PathBuf};

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

pub fn control(name: &str) -> PathBuf {
    compatible_path(&home(), "control", name)
}

pub fn recorder(name: &str) -> PathBuf {
    compatible_path(&home(), "recorder", name)
}

fn compatible_path(home: &Path, group: &str, name: &str) -> PathBuf {
    let current = home.join(group).join(name);
    let legacy = home.join(name);
    if current.exists() || !legacy.exists() {
        current
    } else {
        legacy
    }
}

pub fn migrate() -> Vec<String> {
    migrate_in(&home())
}

fn migrate_in(home: &Path) -> Vec<String> {
    let mut errors = Vec::new();
    for (group, names) in [
        ("control", &["init.lua", "lua"][..]),
        (
            "recorder",
            &[
                "auth.json",
                "debug.log",
                "history.jsonl",
                "missions",
                "tool-output",
                "trust.json",
            ][..],
        ),
    ] {
        for name in names {
            let from = home.join(name);
            let to = home.join(group).join(name);
            if !from.exists() {
                continue;
            }
            if let Err(err) = move_compatible(&from, &to) {
                errors.push(format!("{}: {err}", from.display()));
            }
        }
    }
    errors
}

fn move_compatible(from: &Path, to: &Path) -> std::io::Result<()> {
    if !to.exists() {
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent)?;
        }
        return fs::rename(from, to);
    }
    if !from.is_dir() || !to.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        move_compatible(&entry.path(), &to.join(entry.file_name()))?;
    }
    if fs::read_dir(from)?.next().is_none() {
        fs::remove_dir(from)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_ID: AtomicU64 = AtomicU64::new(0);

    fn scratch() -> PathBuf {
        std::env::temp_dir().join(format!(
            "lunar-storage-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn migrates_legacy_files_without_overwriting_current_files() {
        let home = scratch();
        fs::create_dir_all(home.join("control")).unwrap();
        fs::write(home.join("init.lua"), "legacy").unwrap();
        fs::write(home.join("auth.json"), "auth").unwrap();
        fs::create_dir_all(home.join("missions")).unwrap();
        fs::create_dir_all(home.join("recorder/missions")).unwrap();
        fs::write(home.join("missions/legacy.jsonl"), "legacy mission").unwrap();
        fs::write(
            home.join("recorder/missions/current.jsonl"),
            "current mission",
        )
        .unwrap();
        fs::write(home.join("control/init.lua"), "current").unwrap();

        assert!(migrate_in(&home).is_empty());
        assert_eq!(
            fs::read_to_string(home.join("control/init.lua")).unwrap(),
            "current"
        );
        assert_eq!(fs::read_to_string(home.join("init.lua")).unwrap(), "legacy");
        assert_eq!(
            fs::read_to_string(home.join("recorder/auth.json")).unwrap(),
            "auth"
        );
        assert_eq!(
            fs::read_to_string(home.join("recorder/missions/legacy.jsonl")).unwrap(),
            "legacy mission"
        );
        assert_eq!(
            fs::read_to_string(home.join("recorder/missions/current.jsonl")).unwrap(),
            "current mission"
        );

        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn falls_back_to_a_legacy_path() {
        let home = scratch();
        fs::create_dir_all(&home).unwrap();
        fs::write(home.join("history.jsonl"), "history").unwrap();

        assert_eq!(
            compatible_path(&home, "recorder", "history.jsonl"),
            home.join("history.jsonl")
        );
        assert_eq!(
            compatible_path(&home, "recorder", "debug.log"),
            home.join("recorder/debug.log")
        );

        fs::remove_dir_all(home).unwrap();
    }
}
