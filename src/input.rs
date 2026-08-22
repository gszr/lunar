//! Editor input, history navigation, search, and command completion.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{App, HistorySearch};
use crate::commands;

pub(crate) fn start_search(app: &mut App) {
    app.search = Some(HistorySearch {
        draft: app.input.clone(),
        draft_cursor: app.cursor,
        query: String::new(),
        matched: None,
    });
    update_search(app, false);
}

pub(crate) fn update_search(app: &mut App, older: bool) {
    let Some(search) = &app.search else { return };
    let end = if older {
        search.matched.unwrap_or(app.history.len())
    } else {
        app.history.len()
    };
    let query = search.query.clone();
    let found = (0..end).rev().find(|&i| app.history[i].contains(&query));
    if let Some(search) = &mut app.search {
        search.matched = found;
    }
}

pub(crate) fn on_search_key(app: &mut App, key: KeyEvent) {
    match (key.modifiers, key.code) {
        (_, KeyCode::Esc) => {
            let search = app.search.take().unwrap();
            app.input = search.draft;
            app.cursor = search.draft_cursor;
        }
        (_, KeyCode::Enter) => {
            let search = app.search.take().unwrap();
            if let Some(i) = search.matched {
                app.input = app.history[i].clone();
                app.cursor = app.input.len();
            } else {
                app.input = search.draft;
                app.cursor = search.draft_cursor;
            }
            app.history_cursor = None;
        }
        (KeyModifiers::CONTROL, KeyCode::Char('r')) => update_search(app, true),
        (_, KeyCode::Backspace) => {
            if let Some(search) = &mut app.search {
                search.query.pop();
            }
            update_search(app, false);
        }
        (m, KeyCode::Char(c)) if m.is_empty() || m == KeyModifiers::SHIFT => {
            if let Some(search) = &mut app.search {
                search.query.push(c);
            }
            update_search(app, false);
        }
        _ => {}
    }
}

pub(crate) fn history_up(app: &mut App) {
    if app.history.is_empty() {
        return;
    }
    let next = match app.history_cursor {
        None => {
            app.history_draft = app.input.clone();
            app.history.len() - 1
        }
        Some(i) => i.saturating_sub(1),
    };
    app.history_cursor = Some(next);
    app.input = app.history[next].clone();
    app.cursor = app.input.len();
}

pub(crate) fn history_down(app: &mut App) {
    let Some(i) = app.history_cursor else { return };
    if i + 1 < app.history.len() {
        app.history_cursor = Some(i + 1);
        app.input = app.history[i + 1].clone();
    } else {
        app.history_cursor = None;
        app.input = std::mem::take(&mut app.history_draft);
    }
    app.cursor = app.input.len();
}

pub(crate) fn reset_history_navigation(app: &mut App) {
    app.history_cursor = None;
    app.history_draft.clear();
}

pub(crate) fn on_complete_key(app: &mut App, key: KeyEvent, submit: fn(&mut App)) -> bool {
    let n = commands::matches(&app.input).len();
    if n == 0 {
        return false;
    }
    match (key.modifiers, key.code) {
        (_, KeyCode::Tab) | (KeyModifiers::CONTROL, KeyCode::Char('n')) => {
            app.complete_sel = commands::cycle(app.complete_sel, n, 1);
            true
        }
        (_, KeyCode::Down) if n > 1 => {
            app.complete_sel = commands::cycle(app.complete_sel, n, 1);
            true
        }
        (_, KeyCode::BackTab) | (KeyModifiers::CONTROL, KeyCode::Char('p')) => {
            app.complete_sel = commands::cycle(app.complete_sel, n, -1);
            true
        }
        (_, KeyCode::Up) if n > 1 => {
            app.complete_sel = commands::cycle(app.complete_sel, n, -1);
            true
        }
        (m, KeyCode::Enter) if m.is_empty() => {
            accept_complete(app, submit);
            true
        }
        _ => false,
    }
}

pub(crate) fn insert_input(app: &mut App, c: char) {
    reset_history_navigation(app);
    app.input.insert(app.cursor, c);
    app.cursor += c.len_utf8();
    app.complete_sel = 0;
}

pub(crate) fn accept_complete(app: &mut App, submit: fn(&mut App)) {
    let found = commands::matches(&app.input);
    let Some(cmd) = found.get(app.complete_sel) else {
        submit(app);
        return;
    };
    if cmd.name == "name" {
        app.input = commands::apply(cmd.name);
        app.cursor = app.input.len();
        app.complete_sel = 0;
        return;
    }
    app.input = format!("/{}", cmd.name);
    app.cursor = app.input.len();
    app.complete_sel = 0;
    submit(app);
}

pub(crate) fn prev_char(s: &str, cur: usize) -> usize {
    s[..cur.min(s.len())]
        .char_indices()
        .next_back()
        .map(|(i, _)| i)
        .unwrap_or(0)
}

pub(crate) fn next_char(s: &str, cur: usize) -> usize {
    if cur >= s.len() {
        return s.len();
    }
    cur + s[cur..].chars().next().map(char::len_utf8).unwrap_or(0)
}

pub(crate) fn word_left(s: &str, mut cur: usize) -> usize {
    cur = cur.min(s.len());
    while let Some((i, c)) = s[..cur].char_indices().next_back() {
        if !c.is_whitespace() {
            break;
        }
        cur = i;
    }
    while let Some((i, c)) = s[..cur].char_indices().next_back() {
        if c.is_whitespace() {
            break;
        }
        cur = i;
    }
    cur
}

pub(crate) fn word_right(s: &str, cur: usize) -> usize {
    let cur = cur.min(s.len());
    let rest = &s[cur..];
    let mut i = 0;
    let mut in_word = false;
    for (off, c) in rest.char_indices() {
        if !in_word {
            if !c.is_whitespace() {
                in_word = true;
            }
            i = off + c.len_utf8();
        } else if c.is_whitespace() {
            return cur + off;
        } else {
            i = off + c.len_utf8();
        }
    }
    cur + i
}
