//! Global and CWD context files and skill summaries. Not a system prompt.
//!
//! Loaded from disk at the start of each user turn so edits apply without
//! /reload, then held stable across tool rounds for prefix cache.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

const DEFAULT_BUDGET: u32 = 16_000;
const CONTEXT_FILE: &str = "CONTEXT.md";

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
    load(&cwd, global_agents().as_deref()).map(|loaded| loaded.text)
}

pub fn budget_warning() -> Option<String> {
    let cwd = std::env::current_dir().ok()?;
    let loaded = load(&cwd, global_agents().as_deref())?;
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

pub fn summary() -> (String, usize) {
    match std::env::current_dir() {
        Ok(cwd) => summary_in(&cwd, global_agents().as_deref()),
        Err(_) => ("no cwd".into(), 0),
    }
}

fn summary_in(cwd: &Path, global: Option<&Path>) -> (String, usize) {
    let files = load_files(cwd, global);
    let skills = load_skills(cwd, global);
    if files.is_empty() && skills.is_empty() {
        return ("no preamble".into(), 0);
    }
    let text = render(&files, &skills);
    let tokens = estimate_tokens(&text);
    let budget = budget_tokens();
    let mut out = format!("preamble  ~{tokens} / {budget} tokens");
    if !files.is_empty() {
        out.push_str("\n  files");
        for (name, body) in &files {
            let _ = write!(out, "\n    {name}  {} lines", body.lines().count());
        }
    }
    if !skills.is_empty() {
        out.push_str("\n  skill summaries");
        for skill in &skills {
            let _ = write!(out, "\n    {}  (`{}`)", skill.name, skill.path);
            if !skill.description.is_empty() {
                let _ = write!(out, "\n      {}", skill.description);
            }
        }
    }
    (out, tokens as usize)
}

fn load(cwd: &Path, global: Option<&Path>) -> Option<Loaded> {
    let files = load_files(cwd, global);
    let skills = load_skills(cwd, global);
    if files.is_empty() && skills.is_empty() {
        return None;
    }
    let text = render(&files, &skills);
    let tokens = estimate_tokens(&text);
    Some(Loaded { text, tokens })
}

fn global_agents() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .map(|home| home.join(".agents"))
}

fn load_files(cwd: &Path, global: Option<&Path>) -> Vec<(String, String)> {
    let mut files = Vec::new();
    let project_agents = cwd.join("AGENTS.md");
    let agents = if project_agents.is_file() {
        Some(("AGENTS.md".to_string(), project_agents))
    } else {
        global.map(|root| ("~/.agents/AGENTS.md".to_string(), root.join("AGENTS.md")))
    };
    if let Some((name, path)) = agents
        && let Ok(body) = fs::read_to_string(path)
        && !body.trim().is_empty()
    {
        files.push((name, body.trim().to_string()));
    }
    if let Ok(body) = fs::read_to_string(cwd.join(CONTEXT_FILE))
        && !body.trim().is_empty()
    {
        files.push((CONTEXT_FILE.to_string(), body.trim().to_string()));
    }
    files
}

fn load_skills(cwd: &Path, global: Option<&Path>) -> Vec<Skill> {
    let mut skills = BTreeMap::new();
    if let Some(root) = global {
        load_skills_from(root, "~/.agents/skills", &mut skills);
    }
    load_skills_from(&cwd.join(".agents"), ".agents/skills", &mut skills);
    let mut skills: Vec<_> = skills.into_values().collect();
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    skills
}

fn load_skills_from(root: &Path, display_root: &str, skills: &mut BTreeMap<String, Skill>) {
    let entries = match fs::read_dir(root.join("skills")) {
        Ok(entries) => entries,
        Err(_) => return,
    };
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
        let rel = format!("{display_root}/{dir_name}/SKILL.md");
        skills.insert(
            dir_name.clone(),
            Skill {
                name: fm_name.unwrap_or(dir_name),
                description: fm_desc.unwrap_or_default(),
                path: rel,
            },
        );
    }
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
        assert!(load(&dir, None).is_none());
    }

    #[test]
    fn agents_and_context_go_in_full() {
        let dir = scratch();
        fs::write(dir.join("AGENTS.md"), "be brief").unwrap();
        fs::write(dir.join("CONTEXT.md"), "repo facts").unwrap();
        let text = load(&dir, None).unwrap().text;
        assert!(text.contains("# AGENTS.md"));
        assert!(text.contains("be brief"));
        assert!(text.contains("# CONTEXT.md"));
        assert!(text.contains("repo facts"));
    }

    #[test]
    fn project_agents_overrides_global_agents() {
        let cwd = scratch();
        let global = scratch();
        fs::write(global.join("AGENTS.md"), "global rules").unwrap();

        let text = load(&cwd, Some(&global)).unwrap().text;
        assert!(text.contains("global rules"));

        fs::write(cwd.join("AGENTS.md"), "project rules").unwrap();
        let text = load(&cwd, Some(&global)).unwrap().text;
        assert!(text.contains("project rules"));
        assert!(!text.contains("global rules"));
    }

    #[test]
    fn project_skills_override_global_by_directory_name() {
        let cwd = scratch();
        let global = scratch();
        let global_review = global.join("skills/review");
        let global_ship = global.join("skills/ship");
        let project_review = cwd.join(".agents/skills/review");
        fs::create_dir_all(&global_review).unwrap();
        fs::create_dir_all(&global_ship).unwrap();
        fs::create_dir_all(&project_review).unwrap();
        fs::write(
            global_review.join("SKILL.md"),
            "---\nname: global-review\ndescription: Global review.\n---\n",
        )
        .unwrap();
        fs::write(
            global_ship.join("SKILL.md"),
            "---\nname: ship\ndescription: Ship globally.\n---\n",
        )
        .unwrap();
        fs::write(
            project_review.join("SKILL.md"),
            "---\nname: project-review\ndescription: Project review.\n---\n",
        )
        .unwrap();

        let text = load(&cwd, Some(&global)).unwrap().text;
        assert!(text.contains("project-review: Project review."));
        assert!(text.contains(".agents/skills/review/SKILL.md"));
        assert!(!text.contains("global-review"));
        assert!(text.contains("ship: Ship globally."));
        assert!(text.contains("~/.agents/skills/ship/SKILL.md"));
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
        let text = load(&dir, None).unwrap().text;
        assert!(text.contains("# Skills"));
        assert!(text.contains("review: Review a diff."));
        assert!(text.contains(".agents/skills/review/SKILL.md"));
        assert!(!text.contains("Do not paste this body"));
    }

    #[test]
    fn summary_nests_skill_description_under_name() {
        let dir = scratch();
        let skill = dir.join(".agents").join("skills").join("review");
        fs::create_dir_all(&skill).unwrap();
        fs::write(
            skill.join("SKILL.md"),
            "---\nname: review\ndescription: Review a diff.\n---\n",
        )
        .unwrap();

        let (summary, _) = summary_in(&dir, None);
        assert!(
            summary
                .contains("    review  (`.agents/skills/review/SKILL.md`)\n      Review a diff.")
        );
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
        let text = load(&dir, None).unwrap().text;
        assert!(text.contains("- notes (`.agents/skills/notes/SKILL.md`)"));
        assert!(!text.contains("just a body"));
    }
}
