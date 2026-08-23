//! Terminal event loop and input dispatch.

use std::io;
use std::sync::atomic::Ordering;
use std::time::Duration;

use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::actions::{
    drain_auth, edit_config, load_mission, logout_openai, logout_xai, name_mission, new_mission,
    open_login, open_model, open_resume, open_xai_login, resume_prefix, save_api_key, select_model,
    show_mission, start_openai_oauth, start_xai_oauth,
};
use crate::app::{App, Mode};
use crate::history;
use crate::input::{
    history_down, history_up, insert_input, next_char, on_complete_key, on_search_key, prev_char,
    reset_history_navigation, start_search, word_left, word_right,
};
use crate::prompt;
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
    if let Mode::LoginProvider { cursor } = app.mode {
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
        return;
    }
    if let Mode::LoginMethod { cursor } = app.mode {
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
        return;
    }
    if matches!(app.mode, Mode::ApiKey) {
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
            (m, KeyCode::Char(c)) if m.is_empty() || m == KeyModifiers::SHIFT => {
                insert_input(app, c)
            }
            _ => {}
        }
        return;
    }
    if let Mode::Model { items, cursor } = &app.mode {
        let len = items.len();
        let cursor = *cursor;
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
                if let Some(item) = items.get(cursor).cloned() {
                    app.mode = Mode::Chat;
                    select_model(app, item, true);
                }
            }
            _ => {}
        }
        return;
    }
    if let Mode::Resume { items, cursor, .. } = &app.mode {
        let len = items.len();
        let cursor = *cursor;
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
                if let Some(meta) = items.get(cursor).cloned() {
                    app.mode = Mode::Chat;
                    load_mission(app, &meta.path);
                }
            }
            _ => {}
        }
        return;
    }
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
        "/help" => {
            app.notice = Some(
                "/quit /new /resume /model /config /login /logout /name /mission /context /help    tab cycle    shift+enter / ctrl+j newline    esc abort    ctrl+c quits"
                    .into(),
            );
        }
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
        "/mission" => show_mission(app),
        "/context" => app.notice = Some(prompt::summary()),
        cmd if let Some(name) = cmd.strip_prefix("/name ") => name_mission(app, name),
        cmd if let Some(prefix) = cmd.strip_prefix("/resume ") => resume_prefix(app, prefix),
        cmd if cmd.starts_with('/') => {
            app.notice = Some(format!("unknown command: {cmd}"));
        }
        _ => send_prompt(app, line),
    }
}
