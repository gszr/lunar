//! Terminal event loop and input dispatch.

use std::io;
use std::sync::atomic::Ordering;
use std::time::Duration;

use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::actions::{
    drain_auth, edit_config, load_mission, logout_openai, logout_xai, name_mission, new_mission,
    open_login, open_model, open_resume, open_thinking, open_xai_login, resume_prefix,
    save_api_key, select_model, set_thinking, show_mission, start_openai_oauth, start_xai_oauth,
};
use crate::app::{App, Mode};
use crate::history;
use crate::input::{
    history_down, history_up, insert_input, line_down, line_up, next_char, on_complete_key,
    on_search_key, prev_char, reset_history_navigation, start_search, word_left, word_right,
};
use crate::transcript::{jump_to_tail, on_mouse, page_delta, scroll_by, scroll_home};
use crate::turn::{abort_turn, drain_stream, send_prompt};
use crate::view::draw;

pub(crate) fn run(terminal: &mut DefaultTerminal, app: &mut App) -> io::Result<()> {
    while !app.quit {
        drain_stream(app);
        drain_auth(app);
        terminal.draw(|frame| draw(frame, app))?;
        let wait = if app.cancel.is_some() || app.auth_rx.is_some() {
            Duration::from_millis(16)
        } else {
            Duration::from_secs(3600)
        };
        if event::poll(wait)? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => on_key(app, key),
                Event::Paste(text) => on_paste(app, &text),
                Event::Mouse(mouse) => on_mouse(app, mouse),
                _ => {}
            }
        }
    }
    Ok(())
}

pub(crate) fn on_paste(app: &mut App, text: &str) {
    if !matches!(app.mode, Mode::Chat) || app.search.is_some() {
        return;
    }
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    app.input.insert_str(app.cursor, &text);
    app.cursor += text.len();
    app.complete_sel = 0;
}

pub(crate) fn on_key(app: &mut App, key: KeyEvent) {
    if app.auth_rx.is_some() {
        if key.code == KeyCode::Esc
            && let Some(cancel) = &app.auth_cancel
        {
            cancel.store(true, Ordering::Relaxed);
        }
        return;
    }
    match &app.mode {
        Mode::LoginProvider { cursor } => on_login_provider_key(app, key, *cursor),
        Mode::LoginMethod { cursor } => on_login_method_key(app, key, *cursor),
        Mode::ApiKey => on_api_key(app, key),
        Mode::Context { .. } => on_context_key(app, key),
        Mode::Thinking { cursor } => on_thinking_key(app, key, *cursor),
        Mode::Model { items, cursor } => {
            let len = items.len();
            let cursor = *cursor;
            on_model_key(app, key, len, cursor);
        }
        Mode::Resume { items, cursor, .. } => {
            let len = items.len();
            let cursor = *cursor;
            on_resume_key(app, key, len, cursor);
        }
        Mode::Chat => on_chat_key(app, key),
    }
}

fn on_login_provider_key(app: &mut App, key: KeyEvent, cursor: usize) {
    match key.code {
        KeyCode::Esc => app.mode = Mode::Chat,
        KeyCode::Enter if cursor == 0 => open_xai_login(app),
        KeyCode::Enter => start_openai_oauth(app),
        KeyCode::Up | KeyCode::Char('k') => {
            app.mode = Mode::LoginProvider {
                cursor: cursor.saturating_sub(1),
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.mode = Mode::LoginProvider {
                cursor: (cursor + 1).min(1),
            }
        }
        _ => {}
    }
}

fn on_login_method_key(app: &mut App, key: KeyEvent, cursor: usize) {
    match key.code {
        KeyCode::Esc => app.mode = Mode::Chat,
        KeyCode::Up | KeyCode::Char('k') => {
            app.mode = Mode::LoginMethod {
                cursor: cursor.saturating_sub(1),
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.mode = Mode::LoginMethod {
                cursor: (cursor + 1).min(1),
            }
        }
        KeyCode::Enter if cursor == 0 => start_xai_oauth(app),
        KeyCode::Enter => {
            app.mode = Mode::ApiKey;
            app.input.clear();
            app.cursor = 0;
        }
        _ => {}
    }
}

fn on_context_key(app: &mut App, key: KeyEvent) {
    let page = app.transcript_h.saturating_sub(1).max(1) as usize;
    let Some((text, scroll)) = (match &mut app.mode {
        Mode::Context { text, scroll } => Some((text, scroll)),
        _ => None,
    }) else {
        return;
    };
    let width = app.transcript_w.max(1) as usize;
    let max = crate::view::context_lines(text, width)
        .len()
        .saturating_sub(app.transcript_h as usize);
    match (key.modifiers, key.code) {
        (_, KeyCode::Esc | KeyCode::Char('q')) => app.mode = Mode::Chat,
        (_, KeyCode::PageUp) => *scroll = scroll.saturating_sub(page),
        (_, KeyCode::PageDown) => *scroll = scroll.saturating_add(page).min(max),
        (KeyModifiers::CONTROL, KeyCode::Home) => *scroll = 0,
        (KeyModifiers::CONTROL, KeyCode::End) => *scroll = max,
        (_, KeyCode::Up | KeyCode::Char('k')) => *scroll = scroll.saturating_sub(1),
        (_, KeyCode::Down | KeyCode::Char('j')) => *scroll = scroll.saturating_add(1).min(max),
        _ => {}
    }
}

fn on_api_key(app: &mut App, key: KeyEvent) {
    match (key.modifiers, key.code) {
        (_, KeyCode::Esc) => {
            app.mode = Mode::Chat;
            app.input.clear();
            app.cursor = 0;
        }
        (_, KeyCode::Enter) => save_api_key(app),
        (_, KeyCode::Backspace) => {
            let from = prev_char(&app.input, app.cursor);
            app.input.replace_range(from..app.cursor, "");
            app.cursor = from;
        }
        (m, KeyCode::Char(c)) if m.is_empty() || m == KeyModifiers::SHIFT => insert_input(app, c),
        _ => {}
    }
}

fn on_thinking_key(app: &mut App, key: KeyEvent, cursor: usize) {
    match key.code {
        KeyCode::Esc => app.mode = Mode::Chat,
        KeyCode::Left | KeyCode::Up | KeyCode::Char('h' | 'k') => {
            app.mode = Mode::Thinking {
                cursor: cursor.saturating_sub(1),
            };
        }
        KeyCode::Right | KeyCode::Down | KeyCode::Char('l' | 'j') => {
            app.mode = Mode::Thinking {
                cursor: (cursor + 1).min(3),
            };
        }
        KeyCode::Enter => {
            let level = [
                crate::protocol::Thinking::Off,
                crate::protocol::Thinking::Low,
                crate::protocol::Thinking::Medium,
                crate::protocol::Thinking::High,
            ][cursor];
            app.mode = Mode::Chat;
            set_thinking(app, level);
            app.notice = Some(format!("thinking: {}", level.as_str()));
        }
        _ => {}
    }
}

fn on_model_key(app: &mut App, key: KeyEvent, len: usize, cursor: usize) {
    match key.code {
        KeyCode::Esc => app.mode = Mode::Chat,
        KeyCode::Up | KeyCode::Char('k') => {
            if let Mode::Model { cursor, .. } = &mut app.mode {
                *cursor = cursor.saturating_sub(1);
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if let Mode::Model { cursor, .. } = &mut app.mode {
                *cursor = (*cursor + 1).min(len.saturating_sub(1));
            }
        }
        KeyCode::Enter => {
            let item = match &app.mode {
                Mode::Model { items, .. } => items.get(cursor).cloned(),
                _ => None,
            };
            if let Some(item) = item {
                app.mode = Mode::Chat;
                select_model(app, item, true);
            }
        }
        _ => {}
    }
}

fn on_resume_key(app: &mut App, key: KeyEvent, len: usize, cursor: usize) {
    match key.code {
        KeyCode::Esc => app.mode = Mode::Chat,
        KeyCode::Up | KeyCode::Char('k') => {
            if let Mode::Resume { cursor, .. } = &mut app.mode {
                *cursor = cursor.saturating_sub(1);
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if let Mode::Resume { cursor, .. } = &mut app.mode {
                *cursor = (*cursor + 1).min(len.saturating_sub(1));
            }
        }
        KeyCode::Enter => {
            let meta = match &app.mode {
                Mode::Resume { items, .. } => items.get(cursor).cloned(),
                _ => None,
            };
            if let Some(meta) = meta {
                app.mode = Mode::Chat;
                load_mission(app, &meta.path);
            }
        }
        _ => {}
    }
}

fn on_chat_key(app: &mut App, key: KeyEvent) {
    if app.search.is_some() {
        on_search_key(app, key);
        return;
    }
    if key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Char('r') {
        start_search(app);
        return;
    }
    if on_complete_key(app, key, submit) {
        return;
    }
    match (key.modifiers, key.code) {
        (KeyModifiers::CONTROL, KeyCode::Char('c')) => app.quit = true,
        (_, KeyCode::PageUp) => scroll_by(app, -page_delta(app)),
        (_, KeyCode::PageDown) => scroll_by(app, page_delta(app)),
        (KeyModifiers::CONTROL, KeyCode::Home) => scroll_home(app),
        (KeyModifiers::CONTROL, KeyCode::End) => jump_to_tail(app),
        (_, KeyCode::Esc) => {
            if app.cancel.is_some() {
                abort_turn(app);
            } else {
                app.input.clear();
                app.cursor = 0;
                app.complete_sel = 0;
            }
        }
        (KeyModifiers::CONTROL, KeyCode::Char('a')) => app.cursor = 0,
        (KeyModifiers::CONTROL, KeyCode::Char('e')) => app.cursor = app.input.len(),
        (m, KeyCode::Left)
            if m.contains(KeyModifiers::CONTROL) || m.contains(KeyModifiers::ALT) =>
        {
            app.cursor = word_left(&app.input, app.cursor);
        }
        (m, KeyCode::Right)
            if m.contains(KeyModifiers::CONTROL) || m.contains(KeyModifiers::ALT) =>
        {
            app.cursor = word_right(&app.input, app.cursor);
        }
        (KeyModifiers::CONTROL, KeyCode::Char('b')) | (_, KeyCode::Left) => {
            app.cursor = prev_char(&app.input, app.cursor);
        }
        (KeyModifiers::CONTROL, KeyCode::Char('f')) | (_, KeyCode::Right) => {
            app.cursor = next_char(&app.input, app.cursor);
        }
        (KeyModifiers::ALT, KeyCode::Char('b')) => {
            app.cursor = word_left(&app.input, app.cursor);
        }
        (KeyModifiers::ALT, KeyCode::Char('f')) => {
            app.cursor = word_right(&app.input, app.cursor);
        }
        (KeyModifiers::CONTROL, KeyCode::Char('w')) | (KeyModifiers::ALT, KeyCode::Backspace) => {
            let from = word_left(&app.input, app.cursor);
            app.input.replace_range(from..app.cursor, "");
            app.cursor = from;
            app.complete_sel = 0;
        }
        (KeyModifiers::ALT, KeyCode::Char('d')) => {
            let to = word_right(&app.input, app.cursor);
            app.input.replace_range(app.cursor..to, "");
            app.complete_sel = 0;
        }
        (KeyModifiers::CONTROL, KeyCode::Char('u')) => {
            app.input.replace_range(..app.cursor, "");
            app.cursor = 0;
            app.complete_sel = 0;
        }
        (KeyModifiers::CONTROL, KeyCode::Char('k')) => {
            app.input.truncate(app.cursor);
            app.complete_sel = 0;
        }
        (KeyModifiers::CONTROL, KeyCode::Char('d')) | (_, KeyCode::Delete) => {
            let to = next_char(&app.input, app.cursor);
            app.input.replace_range(app.cursor..to, "");
            app.complete_sel = 0;
        }
        (_, KeyCode::Home) => app.cursor = 0,
        (_, KeyCode::End) => app.cursor = app.input.len(),
        (_, KeyCode::Up) if app.input.contains('\n') => {
            app.cursor = line_up(&app.input, app.cursor);
        }
        (_, KeyCode::Down) if app.input.contains('\n') => {
            app.cursor = line_down(&app.input, app.cursor);
        }
        (_, KeyCode::Up) => history_up(app),
        (_, KeyCode::Down) => history_down(app),
        (_, KeyCode::Backspace) => {
            let from = prev_char(&app.input, app.cursor);
            app.input.replace_range(from..app.cursor, "");
            app.cursor = from;
            app.complete_sel = 0;
        }
        (m, KeyCode::Enter)
            if m.contains(KeyModifiers::SHIFT) || m.contains(KeyModifiers::CONTROL) =>
        {
            insert_input(app, '\n');
        }
        (KeyModifiers::CONTROL, KeyCode::Char('j')) | (_, KeyCode::Char('\n')) => {
            insert_input(app, '\n');
        }
        (_, KeyCode::Enter) => submit(app),
        (m, KeyCode::Char(c)) if m.is_empty() || m == KeyModifiers::SHIFT => {
            insert_input(app, c);
        }
        _ => {}
    }
}

pub(crate) fn submit(app: &mut App) {
    if app.cancel.is_some() {
        return;
    }
    let line = app.input.trim().to_string();
    app.input.clear();
    app.cursor = 0;
    if line.is_empty() {
        return;
    }
    app.history.push(line.clone());
    if let Err(err) = history::append(&line) {
        app.notice = Some(format!("history: {err}"));
    }
    reset_history_navigation(app);
    match line.as_str() {
        "/quit" | "/q" => app.quit = true,
        "/help" => app.notice = Some(crate::commands::help()),
        "/config" => edit_config(app),
        "/new" => new_mission(app),
        "/login" => open_login(app),
        "/login xai" => open_xai_login(app),
        "/login openai" => start_openai_oauth(app),
        "/logout" => app.notice = Some("usage: /logout xai|openai".into()),
        "/logout xai" => logout_xai(app),
        "/logout openai" => logout_openai(app),
        "/resume" => open_resume(app),
        "/model" => open_model(app),
        "/thinking" => open_thinking(app),
        cmd if let Some(raw) = cmd.strip_prefix("/thinking ") => {
            match crate::protocol::Thinking::parse(raw.trim()) {
                Some(level) => {
                    if set_thinking(app, level) {
                        app.notice = Some(format!("thinking: {}", level.as_str()));
                    } else {
                        app.notice = Some("no model configured".into());
                    }
                }
                None => app.notice = Some("usage: /thinking off|low|medium|high".into()),
            }
        }
        "/mission" => show_mission(app),
        "/context" | "/context raw" => {
            let text = if line == "/context raw" {
                crate::context::raw(&app.messages)
            } else {
                crate::context::summary(&app.messages)
            };
            app.mode = Mode::Context { text, scroll: 0 };
        }
        cmd if let Some(name) = cmd.strip_prefix("/name ") => name_mission(app, name),
        cmd if let Some(prefix) = cmd.strip_prefix("/resume ") => resume_prefix(app, prefix),
        cmd if cmd.starts_with('/') => {
            app.notice = Some(format!("unknown command: {cmd}"));
        }
        _ => send_prompt(app, line),
    }
}
