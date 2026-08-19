//! CWD context files and skill summaries. Not a system prompt.
//!
//! Loaded from disk on every request so edits apply without /reload.

use std::fmt::Write as _;
use std::fs;
use std::path::Path;

const DEFAULT_BUDGET: u32 = 16_000;
const CONTEXT_FILES: &[&str] = &["AGENTS.md", "CONTEXT.md"];

struct Skill {
    name: String,
    description: String,
    path: String,
}

struct Loaded {
    text: String,
    tokens: u32,
}

pub fn preamble() -> Option<String> {
    let cwd = std::env::current_dir().ok()?;
    load(&cwd).map(|loaded| loaded.text)
}

pub fn budget_warning() -> Option<String> {
    let cwd = std::env::current_dir().ok()?;
    let loaded = load(&cwd)?;
    let budget = budget_tokens();
    if loaded.tokens > budget {
        Some(format!(
            "prompt context ~{} tokens (budget {})",
            loaded.tokens, budget
        ))
    } else {
        None
    }
}

fn load(cwd: &Path) -> Option<Loaded> {
    let files = load_files(cwd);
    let skills = load_skills(cwd);
    if files.is_empty() && skills.is_empty() {
        return None;
    }
    let text = render(&files, &skills);
    let tokens = estimate_tokens(&text);
    Some(Loaded { text, tokens })
}

fn load_files(cwd: &Path) -> Vec<(String, String)> {
    let mut files = Vec::new();
    for name in CONTEXT_FILES {
        let Ok(body) = fs::read_to_string(cwd.join(name)) else {
            continue;
        };
        let body = body.trim();
        if !body.is_empty() {
            files.push(((*name).to_string(), body.to_string()));
        }
    }
    files
}

fn load_skills(cwd: &Path) -> Vec<Skill> {
    let root = cwd.join(".agents").join("skills");
    let entries = match fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };
    let mut skills = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let skill_md = path.join("SKILL.md");
        let Ok(body) = fs::read_to_string(&skill_md) else {
            continue;
        };
        let dir_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("skill")
            .to_string();
        let (fm_name, fm_desc) = parse_frontmatter(&body);
        let rel = format!(".agents/skills/{dir_name}/SKILL.md");
        skills.push(Skill {
            name: fm_name.unwrap_or(dir_name),
            description: fm_desc.unwrap_or_default(),
            path: rel,
        });
    }
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    skills
}

fn render(files: &[(String, String)], skills: &[Skill]) -> String {
    let mut out = String::new();
    for (name, body) in files {
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        let _ = write!(out, "# {name}\n\n{body}");
    }
    if !skills.is_empty() {
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str("# Skills\n\nRead a skill's file before using it.");
        for skill in skills {
            if skill.description.is_empty() {
                let _ = write!(out, "\n- {} (`{}`)", skill.name, skill.path);
            } else {
                let _ = write!(
                    out,
                    "\n- {}: {} (`{}`)",
                    skill.name, skill.description, skill.path
                );
            }
        }
    }
    out
}

fn parse_frontmatter(text: &str) -> (Option<String>, Option<String>) {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let Some(rest) = text.strip_prefix("---") else {
        return (None, None);
    };
    let rest = rest
        .strip_prefix('\n')
        .or_else(|| rest.strip_prefix("\r\n"))
        .unwrap_or(rest);
    let Some(end) = rest.find("\n---") else {
        return (None, None);
    };
    let mut name = None;
    let mut description = None;
    let mut lines = rest[..end].lines().peekable();
    while let Some(line) = lines.next() {
        if let Some(value) = line.strip_prefix("name:") {
            name = nonempty_owned(unquote(value.trim()));
        } else if let Some(value) = line.strip_prefix("description:") {
            let value = value.trim();
            if matches!(value, "" | ">" | "|" | ">-" | "|-") {
                let mut parts = Vec::new();
                while let Some(next) = lines.peek() {
                    if next.starts_with(' ') || next.starts_with('\t') {
                        parts.push(lines.next().unwrap().trim().to_string());
                    } else {
                        break;
                    }
                }
                description = nonempty_owned(parts.join(" "));
            } else {
                description = nonempty_owned(unquote(value));
            }
        }
    }
    (name, description)
}

fn unquote(s: &str) -> String {
    for quote in ['"', '\''] {
        if let Some(inner) = s
            .strip_prefix(quote)
            .and_then(|rest| rest.strip_suffix(quote))
        {
            return inner.to_string();
        }
    }
    s.to_string()
}

fn nonempty_owned(s: String) -> Option<String> {
    if s.is_empty() { None } else { Some(s) }
}

fn estimate_tokens(text: &str) -> u32 {
    (text.chars().count() as u32).div_ceil(4)
}

fn budget_tokens() -> u32 {
    std::env::var("LUNAR_PROMPT_BUDGET")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_BUDGET)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn scratch() -> std::path::PathBuf {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("lunar-prompt-{nanos}-{n}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn empty_cwd_is_none() {
        let dir = scratch();
        assert!(load(&dir).is_none());
    }

    #[test]
    fn agents_and_context_go_in_full() {
        let dir = scratch();
        fs::write(dir.join("AGENTS.md"), "be brief").unwrap();
        fs::write(dir.join("CONTEXT.md"), "repo facts").unwrap();
        let text = load(&dir).unwrap().text;
        assert!(text.contains("# AGENTS.md"));
        assert!(text.contains("be brief"));
        assert!(text.contains("# CONTEXT.md"));
        assert!(text.contains("repo facts"));
    }

    #[test]
    fn skill_summary_uses_frontmatter() {
        let dir = scratch();
        let skill = dir.join(".agents").join("skills").join("review");
        fs::create_dir_all(&skill).unwrap();
        fs::write(
            skill.join("SKILL.md"),
            "---\nname: review\ndescription: Review a diff.\n---\n\nDo not paste this body.\n",
        )
        .unwrap();
        let text = load(&dir).unwrap().text;
        assert!(text.contains("# Skills"));
        assert!(text.contains("review: Review a diff."));
        assert!(text.contains(".agents/skills/review/SKILL.md"));
        assert!(!text.contains("Do not paste this body"));
    }

    #[test]
    fn folded_description() {
        let (name, desc) = parse_frontmatter(
            "---\nname: ship\ndescription: >\n  Cut a release.\n  Tag it.\n---\n",
        );
        assert_eq!(name.as_deref(), Some("ship"));
        assert_eq!(desc.as_deref(), Some("Cut a release. Tag it."));
    }

    #[test]
    fn missing_frontmatter_falls_back_to_dirname() {
        let dir = scratch();
        let skill = dir.join(".agents").join("skills").join("notes");
        fs::create_dir_all(&skill).unwrap();
        fs::write(skill.join("SKILL.md"), "just a body\n").unwrap();
        let text = load(&dir).unwrap().text;
        assert!(text.contains("- notes (`.agents/skills/notes/SKILL.md`)"));
        assert!(!text.contains("just a body"));
    }
}
