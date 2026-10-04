//! `@` file references. Search a directory and insert the path; do not read file bodies.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub(crate) const MAX_VISIBLE: usize = 5;
const MAX_MATCHES: usize = 100;
const MAX_VISITED: usize = 4000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FileMatch {
    pub path: String,
    pub kind: String,
    pub modified: SystemTime,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Row {
    Header(String),
    File(FileMatch),
}

/// `@` token at a boundary, up to the cursor. `None` when this is not a reference.
pub(crate) fn at_token(input: &str, cursor: usize) -> Option<(usize, &str)> {
    let cursor = cursor.min(input.len());
    let before = &input[..cursor];
    let start = before
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_whitespace())
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(0);
    let token = &input[start..cursor];
    let query = token.strip_prefix('@')?;
    if query.chars().any(char::is_whitespace) {
        return None;
    }
    Some((start, query))
}

pub(crate) fn kind_of(path: &str) -> String {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() => ext.to_ascii_lowercase(),
        _ => "file".into(),
    }
}

pub(crate) fn search(root: &Path, query: &str) -> Vec<FileMatch> {
    search_from(root, home_dir().as_deref(), query)
}

fn search_from(root: &Path, home: Option<&Path>, query: &str) -> Vec<FileMatch> {
    let Some(scope) = scope_for(root, home, query) else {
        return Vec::new();
    };
    let mut found = walk(&scope);
    for item in &mut found {
        item.kind = kind_of(&item.path);
    }
    found.sort_by(|a, b| {
        b.modified
            .cmp(&a.modified)
            .then_with(|| a.path.cmp(&b.path))
    });
    found.truncate(MAX_MATCHES);
    found
}

struct Scope {
    walk: PathBuf,
    filter: String,
    prefix: String,
    show_hidden: bool,
    /// `~/` and `/` are a directory. Do not search until the user continues.
    listing: bool,
}

fn scope_for(root: &Path, home: Option<&Path>, query: &str) -> Option<Scope> {
    let show_hidden = wants_hidden(query);
    if query == "~" || query.starts_with("~/") {
        let home = home?;
        let rest = if query == "~" { "" } else { &query[2..] };
        return Some(narrow(home, rest, "~/", show_hidden, true));
    }
    if let Some(rest) = query.strip_prefix('/') {
        return Some(narrow(Path::new("/"), rest, "/", show_hidden, true));
    }
    if query == ".." || query.starts_with("../") {
        return Some(narrow(root, query, "", show_hidden, false));
    }
    Some(Scope {
        walk: root.to_path_buf(),
        filter: query.to_ascii_lowercase(),
        prefix: String::new(),
        show_hidden,
        listing: false,
    })
}

/// Walk the longest existing directory in `rest`, and filter by what remains.
fn narrow(base: &Path, rest: &str, prefix: &str, show_hidden: bool, listing: bool) -> Scope {
    let parts: Vec<&str> = if rest.is_empty() {
        Vec::new()
    } else {
        rest.split('/').filter(|part| !part.is_empty()).collect()
    };
    let mut walk = base.to_path_buf();
    let mut consumed = 0;
    for (i, part) in parts.iter().enumerate() {
        let next = walk.join(part);
        if next.is_dir() {
            walk = next;
            consumed = i + 1;
        } else {
            break;
        }
    }
    let extra = parts[..consumed].join("/");
    Scope {
        walk,
        filter: parts[consumed..].join("/").to_ascii_lowercase(),
        prefix: join_prefix(prefix, &extra),
        show_hidden,
        listing,
    }
}

fn join_prefix(prefix: &str, extra: &str) -> String {
    if extra.is_empty() {
        return prefix.to_string();
    }
    let mut out = prefix.to_string();
    if !out.is_empty() && !out.ends_with('/') {
        out.push('/');
    }
    out.push_str(extra);
    out.push('/');
    out
}

fn wants_hidden(query: &str) -> bool {
    query.split('/').any(|part| {
        let part = part.strip_prefix('~').unwrap_or(part);
        part.starts_with('.') && part != "." && part != ".."
    })
}

fn walk(scope: &Scope) -> Vec<FileMatch> {
    let mut found = Vec::new();
    let mut stack = vec![scope.walk.clone()];
    let mut seen = std::collections::HashSet::new();
    let mut visited = 0;
    while let Some(dir) = stack.pop() {
        if visited >= MAX_VISITED {
            break;
        }
        let canon = dir.canonicalize().unwrap_or_else(|_| dir.clone());
        if !seen.insert(canon) {
            continue;
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        let mut dirs = Vec::new();
        for entry in entries.flatten() {
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            if name == ".git" || name == "target" {
                continue;
            }
            if !scope.show_hidden && name.starts_with('.') {
                continue;
            }
            let path = entry.path();
            if path.is_dir() {
                if !scope.listing {
                    dirs.push(path);
                }
                continue;
            }
            if !path.is_file() {
                continue;
            }
            visited += 1;
            let Some(relative) = path.strip_prefix(&scope.walk).ok() else {
                continue;
            };
            let relative = relative.to_string_lossy().replace('\\', "/");
            if !scope.filter.is_empty() && !relative.to_ascii_lowercase().contains(&scope.filter) {
                continue;
            }
            let modified = entry
                .metadata()
                .and_then(|meta| meta.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            found.push(FileMatch {
                path: format!("{}{relative}", scope.prefix),
                kind: String::new(),
                modified,
            });
        }
        stack.extend(dirs);
    }
    found
}

pub(crate) fn rows(found: &[FileMatch]) -> Vec<Row> {
    let mut kinds: Vec<&str> = Vec::new();
    for item in found {
        if !kinds.contains(&item.kind.as_str()) {
            kinds.push(&item.kind);
        }
    }
    let mut rows = Vec::new();
    for kind in kinds {
        rows.push(Row::Header(kind.to_string()));
        for item in found.iter().filter(|item| item.kind == kind) {
            rows.push(Row::File(item.clone()));
        }
    }
    rows
}

pub(crate) fn file_count(rows: &[Row]) -> usize {
    rows.iter()
        .filter(|row| matches!(row, Row::File(_)))
        .count()
}

pub(crate) fn clamp_selected(selected: usize, rows: &[Row]) -> usize {
    if file_count(rows) == 0 {
        return 0;
    }
    let mut seen = 0;
    let mut last = 0;
    for (index, row) in rows.iter().enumerate() {
        if matches!(row, Row::File(_)) {
            if seen == selected {
                return index;
            }
            last = index;
            seen += 1;
        }
    }
    last
}

pub(crate) fn cycle(selected: usize, rows: &[Row], delta: isize) -> usize {
    let files: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter_map(|(index, row)| matches!(row, Row::File(_)).then_some(index))
        .collect();
    if files.is_empty() {
        return 0;
    }
    let current = files
        .iter()
        .position(|index| *index == selected)
        .unwrap_or(0);
    let n = files.len() as isize;
    let next = current as isize + delta;
    files[((next % n + n) % n) as usize]
}

/// Five-row window that keeps the selection on screen, header included.
pub(crate) fn visible(rows: &[Row], selected: usize) -> (usize, &[Row]) {
    if rows.len() <= MAX_VISIBLE {
        return (0, rows);
    }
    let selected = selected.min(rows.len().saturating_sub(1));
    let mut start = selected
        .saturating_sub(MAX_VISIBLE / 2)
        .min(rows.len() - MAX_VISIBLE);
    if start > 0
        && matches!(rows[start], Row::File(_))
        && matches!(rows[start - 1], Row::Header(_))
        && selected < start + MAX_VISIBLE - 1
    {
        start -= 1;
    }
    (start, &rows[start..start + MAX_VISIBLE])
}

pub(crate) fn existing_paths(input: &str, root: &Path) -> Vec<(usize, usize)> {
    existing_paths_in(input, root, home_dir().as_deref())
}

fn existing_paths_in(input: &str, root: &Path, home: Option<&Path>) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut start = 0;
    for (index, c) in input.char_indices() {
        if c.is_whitespace() {
            consider(&mut spans, input, root, home, start, index);
            start = index + c.len_utf8();
        }
    }
    consider(&mut spans, input, root, home, start, input.len());
    spans
}

fn consider(
    spans: &mut Vec<(usize, usize)>,
    input: &str,
    root: &Path,
    home: Option<&Path>,
    start: usize,
    end: usize,
) {
    if start >= end || end > input.len() {
        return;
    }
    let token = &input[start..end];
    if token.is_empty() || token.contains('\u{0}') {
        return;
    }
    if resolve_file(root, home, token).is_some_and(|path| path.is_file()) {
        spans.push((start, end));
    }
}

fn resolve_file(root: &Path, home: Option<&Path>, token: &str) -> Option<PathBuf> {
    if token == "~" || token.starts_with("~/") {
        let home = home?;
        return Some(if token == "~" {
            home.to_path_buf()
        } else {
            home.join(&token[2..])
        });
    }
    Some(root.join(token))
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

pub(crate) fn cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use std::thread;
    use std::time::Duration;

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "lunar-files-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let mut file = File::create(path).unwrap();
        file.write_all(body.as_bytes()).unwrap();
    }

    #[test]
    fn at_token_requires_a_boundary() {
        assert_eq!(at_token("@src", 4), Some((0, "src")));
        assert_eq!(at_token("see @src/event", 14), Some((4, "src/event")));
        assert_eq!(at_token("email@host", 10), None);
        assert_eq!(at_token("@src/event.rs next", 4), Some((0, "src")));
        assert_eq!(at_token(" @", 2), Some((1, "")));
        assert_eq!(at_token("nope", 4), None);
    }

    #[test]
    fn kind_uses_the_last_extension() {
        assert_eq!(kind_of("src/event.rs"), "rs");
        assert_eq!(kind_of("README"), "file");
        assert_eq!(kind_of(".gitignore"), "file");
        assert_eq!(kind_of("archive.tar.gz"), "gz");
    }

    #[test]
    fn search_groups_by_newest_kind_then_mtime() {
        let root = scratch();
        touch(&root.join("old.md"), "old");
        thread::sleep(Duration::from_millis(20));
        touch(&root.join("src/new.rs"), "new");
        thread::sleep(Duration::from_millis(20));
        touch(&root.join("src/newer.rs"), "newer");
        touch(&root.join(".hidden.rs"), "hidden");
        touch(&root.join("target/skip.rs"), "skip");
        fs::create_dir_all(root.join(".git")).unwrap();
        touch(&root.join(".git/config"), "git");

        let found = search(&root, "src");
        let kinds: Vec<_> = rows(&found)
            .into_iter()
            .map(|row| match row {
                Row::Header(kind) => format!("#{kind}"),
                Row::File(file) => file.path,
            })
            .collect();
        assert_eq!(
            kinds,
            vec![
                "#rs".to_string(),
                "src/newer.rs".into(),
                "src/new.rs".into(),
            ]
        );
        assert!(
            search(&root, ".hidden")
                .iter()
                .any(|f| f.path == ".hidden.rs")
        );
        assert!(search(&root, "skip").is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cycle_skips_headers_and_window_keeps_selection() {
        let files = vec![
            FileMatch {
                path: "a.rs".into(),
                kind: "rs".into(),
                modified: SystemTime::UNIX_EPOCH,
            },
            FileMatch {
                path: "b.md".into(),
                kind: "md".into(),
                modified: SystemTime::UNIX_EPOCH,
            },
        ];
        let listed = rows(&files);
        assert!(matches!(listed[0], Row::Header(_)));
        assert_eq!(cycle(1, &listed, -1), 3);
        assert_eq!(cycle(3, &listed, 1), 1);
        let long: Vec<_> = (0..8)
            .map(|i| FileMatch {
                path: format!("f{i}.rs"),
                kind: "rs".into(),
                modified: SystemTime::UNIX_EPOCH,
            })
            .collect();
        let long_rows = rows(&long);
        let (start, view) = visible(&long_rows, long_rows.len() - 1);
        assert_eq!(view.len(), MAX_VISIBLE);
        assert_eq!(start + view.len(), long_rows.len());
    }

    #[test]
    fn tilde_absolute_and_parent_search_that_directory() {
        let root = scratch();
        let home = scratch();
        let elsewhere = scratch();
        touch(&home.join("notes.md"), "home");
        touch(&home.join("proj/lib.rs"), "deep");
        touch(&elsewhere.join("other.rs"), "abs");
        touch(&root.join("here.rs"), "here");
        fs::create_dir_all(root.join("nested")).unwrap();

        let home_hits = search_from(&root, Some(&home), "~/notes");
        assert_eq!(home_hits.len(), 1);
        assert_eq!(home_hits[0].path, "~/notes.md");
        assert!(
            search_from(&root, Some(&home), "~/")
                .iter()
                .all(|f| !f.path.contains("proj/"))
        );

        let abs = elsewhere.to_string_lossy().replace('\\', "/");
        let abs_hits = search_from(&root, Some(&home), &format!("{abs}/other"));
        assert_eq!(abs_hits.len(), 1);
        assert!(abs_hits[0].path.ends_with("/other.rs"));
        assert!(abs_hits[0].path.starts_with('/'));

        let parent = search_from(&root.join("nested"), Some(&home), "../here");
        assert!(parent.iter().any(|f| f.path == "../here.rs"));

        let input = format!("see {abs}/other.rs ~/notes.md missing.rs");
        let spans = existing_paths_in(&input, &root, Some(&home));
        assert_eq!(spans.len(), 2);
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(home).unwrap();
        fs::remove_dir_all(elsewhere).unwrap();
    }

    #[test]
    fn existing_paths_mark_only_real_files() {
        let root = scratch();
        touch(&root.join("src/event.rs"), "x");
        let input = "see src/event.rs and missing.rs";
        let spans = existing_paths(input, &root);
        assert_eq!(spans, vec![(4, 16)]);
        fs::remove_dir_all(root).unwrap();
    }
}
