mod app;
mod auth;
mod cli;
mod commands;
mod complete;
mod history;
mod lua;
mod mission;
mod prompt;
mod render;
mod splash;
mod terminal;
mod tools;
mod view;

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, TryRecvError};
use std::time::Duration;

use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use app::{App, AuthEvent, AuthPrompt, HistorySearch, Message, Mode, Role};
use complete::{ChatMessage, Config, StreamEvent, ToolCall, ToolResult, Usage};
use view::{draw, invalidate_paint, jump_to_tail, on_mouse, page_delta, scroll_by, scroll_home};

const MAX_ROUNDS: u32 = 50;

fn main() -> io::Result<()> {
    let resume_last = match cli::parse(std::env::args_os().skip(1)) {
        Ok(cli::Action::Open { continue_last }) => continue_last,
        Ok(cli::Action::Help) => {
            print!("{}", cli::HELP);
            return Ok(());
        }
        Err(message) => {
            eprintln!("lunar: {message}\n\nTry 'lunar --help' for more information.");
            std::process::exit(2);
        }
    };
    let loaded = lua::load();
    let startup_config = loaded.config.clone();
    let mut terminal = terminal::Terminal::init();
    let mut app = App {
        input: String::new(),
        cursor: 0,
        notice: loaded.notice,
        messages: Vec::new(),
        config: loaded.config,
        startup_config,
        models: loaded.models,
        stream_rx: None,
        cancel: None,
        rounds: 0,
        usage: Usage::default(),
        last_prompt: 0,
        preamble: None,
        mission: None,
        mode: Mode::Chat,
        complete_sel: 0,
        quit: false,
        scroll: 0,
        follow: true,
        transcript_w: 0,
        transcript_h: 0,
        paint_width: 0,
        paint_frozen: Vec::new(),
        paint_upto: 0,
        paint_prev_tool: false,
        history: history::load().unwrap_or_default(),
        history_cursor: None,
        history_draft: String::new(),
        search: None,
        auth_rx: None,
        auth_cancel: None,
        auth_prompt: None,
    };
    if resume_last {
        match mission::list()?.into_iter().next() {
            Some(meta) => load_mission(&mut app, &meta.path),
            None => app.notice = Some("no missions in this directory".into()),
        }
    }
    if app.notice.is_none() {
        app.notice = prompt::budget_warning();
    }
    run(terminal.get_mut(), &mut app)
}

fn run(terminal: &mut DefaultTerminal, app: &mut App) -> io::Result<()> {
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

fn drain_auth(app: &mut App) {
    let Some(rx) = app.auth_rx.as_ref() else {
        return;
    };
    let event = match rx.try_recv() {
        Ok(event) => Some(event),
        Err(TryRecvError::Disconnected) => Some(AuthEvent::Failed("login ended".into())),
        Err(TryRecvError::Empty) => None,
    };
    match event {
        Some(AuthEvent::DeviceCode {
            url,
            code,
            browser_opened,
        }) => {
            app.auth_prompt = Some(AuthPrompt {
                url,
                code,
                browser_opened,
            });
        }
        Some(AuthEvent::Done) => {
            app.auth_rx = None;
            app.auth_cancel = None;
            app.auth_prompt = None;
            app.notice = Some("logged in to xAI".into());
            reload_config(app);
        }
        Some(AuthEvent::Failed(err)) => {
            app.auth_rx = None;
            app.auth_cancel = None;
            app.auth_prompt = None;
            app.notice = Some(err);
        }
        None => {}
    }
}

fn drain_stream(app: &mut App) {
    let Some(rx) = app.stream_rx.as_ref() else {
        return;
    };
    let mut end: Option<StreamEvent> = None;
    loop {
        match rx.try_recv() {
            Ok(StreamEvent::Delta(text)) => {
                if let Some(last) = app.messages.last_mut()
                    && matches!(last.role, Role::Assistant)
                {
                    last.text.push_str(&text);
                }
            }
            Ok(StreamEvent::Think(text)) => {
                if let Some(last) = app.messages.last_mut()
                    && matches!(last.role, Role::Assistant)
                {
                    last.thinking.push_str(&text);
                }
            }
            Ok(StreamEvent::Usage(usage)) => {
                app.usage.add(usage);
                app.last_prompt = usage.prompt();
            }
            Ok(other) => {
                end = Some(other);
                break;
            }
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                end = Some(StreamEvent::Failed("stream ended".into()));
                break;
            }
        }
    }
    if let Some(end) = end {
        finish_stream(app, end);
    }
}

fn finish_stream(app: &mut App, end: StreamEvent) {
    match end {
        StreamEvent::Tools { calls, truncated } => begin_tools(app, calls, truncated),
        StreamEvent::ToolResults(results) => apply_tool_results(app, results),
        StreamEvent::Done => {
            persist_last_assistant(app);
            app.stream_rx = None;
            app.cancel = None;
            pop_empty_assistant(app);
        }
        StreamEvent::Failed(err) => {
            persist_last_assistant(app);
            app.stream_rx = None;
            app.cancel = None;
            pop_empty_assistant(app);
            app.notice = Some(err);
        }
        StreamEvent::Delta(_) | StreamEvent::Think(_) | StreamEvent::Usage(_) => {}
    }
}

fn abort_turn(app: &mut App) {
    if let Some(flag) = &app.cancel {
        flag.store(true, Ordering::Relaxed);
    }
    persist_last_assistant(app);
    app.stream_rx = None;
    app.cancel = None;
    pop_empty_assistant(app);
    app.notice = Some("aborted".into());
}

fn pop_empty_assistant(app: &mut App) {
    if matches!(
        app.messages.last(),
        Some(Message {
            role: Role::Assistant,
            text,
            thinking,
            tool_calls,
            ..
        }) if text.is_empty() && thinking.is_empty() && tool_calls.is_empty()
    ) {
        app.messages.pop();
    }
}

fn begin_tools(app: &mut App, calls: Vec<ToolCall>, truncated: bool) {
    if let Some(last) = app.messages.last_mut()
        && matches!(last.role, Role::Assistant)
    {
        last.tool_calls = calls.clone();
    }
    persist_last_assistant(app);
    if truncated {
        apply_tool_results(app, skipped_truncated(&calls));
        return;
    }
    let Some(cancel) = app.cancel.clone() else {
        return;
    };
    let (tx, rx) = mpsc::channel();
    app.stream_rx = Some(rx);
    std::thread::spawn(move || {
        let results = run_tools_parallel(&calls, &cancel);
        if cancel.load(Ordering::Relaxed) {
            let _ = tx.send(StreamEvent::Failed("aborted".into()));
            return;
        }
        let _ = tx.send(StreamEvent::ToolResults(results));
    });
}

fn skipped_truncated(calls: &[ToolCall]) -> Vec<ToolResult> {
    calls
        .iter()
        .map(|call| ToolResult {
            id: call.id.clone(),
            title: call.name.clone(),
            content: "not executed: hit the output token limit; arguments may be truncated. Re-issue the call.".into(),
        })
        .collect()
}

fn run_tools_parallel(calls: &[ToolCall], cancel: &AtomicBool) -> Vec<ToolResult> {
    std::thread::scope(|s| {
        let handles: Vec<_> = calls
            .iter()
            .map(|call| {
                s.spawn(move || {
                    if cancel.load(Ordering::Relaxed) {
                        return ToolResult {
                            id: call.id.clone(),
                            title: call.name.clone(),
                            content: "aborted".into(),
                        };
                    }
                    let out = tools::run(&call.name, &call.arguments, cancel);
                    ToolResult {
                        id: call.id.clone(),
                        title: out.title,
                        content: out.content,
                    }
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| {
                h.join().unwrap_or_else(|_| ToolResult {
                    id: String::new(),
                    title: "tool".into(),
                    content: "tool panicked".into(),
                })
            })
            .collect()
    })
}

fn apply_tool_results(app: &mut App, results: Vec<ToolResult>) {
    if app
        .cancel
        .as_ref()
        .is_some_and(|c| c.load(Ordering::Relaxed))
    {
        app.stream_rx = None;
        app.cancel = None;
        return;
    }
    for result in results {
        persist_value(
            app,
            &mission::tool_line(&result.id, &result.title, &result.content),
        );
        app.messages
            .push(Message::tool(result.id, result.title, result.content));
    }
    app.rounds += 1;
    if app.rounds >= MAX_ROUNDS {
        app.stream_rx = None;
        app.cancel = None;
        app.notice = Some(format!(
            "tool-round limit reached ({MAX_ROUNDS}); submit \"continue\" to proceed"
        ));
        return;
    }
    continue_turn(app);
}

fn on_paste(app: &mut App, text: &str) {
    if !matches!(app.mode, Mode::Chat) || app.search.is_some() {
        return;
    }
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    app.input.insert_str(app.cursor, &text);
    app.cursor += text.len();
    app.complete_sel = 0;
}

fn on_key(app: &mut App, key: KeyEvent) {
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
            KeyCode::Enter => app.mode = Mode::LoginMethod { cursor: 0 },
            KeyCode::Up | KeyCode::Char('k') | KeyCode::Down | KeyCode::Char('j') => {
                app.mode = Mode::LoginProvider { cursor }
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
    if let Mode::Resume { items, cursor } = &app.mode {
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
    if on_complete_key(app, key) {
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

fn start_search(app: &mut App) {
    app.search = Some(HistorySearch {
        draft: app.input.clone(),
        draft_cursor: app.cursor,
        query: String::new(),
        matched: None,
    });
    update_search(app, false);
}

fn update_search(app: &mut App, older: bool) {
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

fn on_search_key(app: &mut App, key: KeyEvent) {
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

fn history_up(app: &mut App) {
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

fn history_down(app: &mut App) {
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

fn reset_history_navigation(app: &mut App) {
    app.history_cursor = None;
    app.history_draft.clear();
}

fn on_complete_key(app: &mut App, key: KeyEvent) -> bool {
    let n = commands::matches(&app.input).len();
    if n == 0 {
        return false;
    }
    match (key.modifiers, key.code) {
        (_, KeyCode::Tab) | (_, KeyCode::Down) | (KeyModifiers::CONTROL, KeyCode::Char('n')) => {
            app.complete_sel = commands::cycle(app.complete_sel, n, 1);
            true
        }
        (_, KeyCode::BackTab) | (_, KeyCode::Up) | (KeyModifiers::CONTROL, KeyCode::Char('p')) => {
            app.complete_sel = commands::cycle(app.complete_sel, n, -1);
            true
        }
        (m, KeyCode::Enter) if m.is_empty() => {
            accept_complete(app);
            true
        }
        _ => false,
    }
}

fn insert_input(app: &mut App, c: char) {
    reset_history_navigation(app);
    app.input.insert(app.cursor, c);
    app.cursor += c.len_utf8();
    app.complete_sel = 0;
}

fn accept_complete(app: &mut App) {
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

fn prev_char(s: &str, cur: usize) -> usize {
    s[..cur.min(s.len())]
        .char_indices()
        .next_back()
        .map(|(i, _)| i)
        .unwrap_or(0)
}

fn next_char(s: &str, cur: usize) -> usize {
    if cur >= s.len() {
        return s.len();
    }
    cur + s[cur..].chars().next().map(char::len_utf8).unwrap_or(0)
}

fn word_left(s: &str, mut cur: usize) -> usize {
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

fn word_right(s: &str, cur: usize) -> usize {
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

fn submit(app: &mut App) {
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
        "/login" | "/login xai" => open_login(app),
        "/logout" | "/logout xai" => logout_xai(app),
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

fn persist_value(app: &mut App, value: &serde_json::Value) {
    if app.mission.is_none() {
        match mission::create() {
            Ok(m) => app.mission = Some(m),
            Err(err) => {
                app.notice = Some(format!("mission: {err}"));
                return;
            }
        }
    }
    if let Some(m) = &app.mission
        && let Err(err) = mission::append(m, value)
    {
        app.notice = Some(format!("mission: {err}"));
    }
}

fn persist_last_assistant(app: &mut App) {
    let Some(last) = app.messages.last() else {
        return;
    };
    if !matches!(last.role, Role::Assistant) {
        return;
    }
    if last.text.is_empty() && last.tool_calls.is_empty() {
        return;
    }
    persist_value(app, &mission::assistant_line(&last.text, &last.tool_calls));
}

fn open_login(app: &mut App) {
    app.mode = Mode::LoginProvider { cursor: 0 };
}

fn start_xai_oauth(app: &mut App) {
    app.mode = Mode::Chat;
    let cancel = Arc::new(AtomicBool::new(false));
    let thread_cancel = cancel.clone();
    let (tx, rx) = mpsc::channel();
    app.auth_cancel = Some(cancel);
    app.auth_rx = Some(rx);
    app.auth_prompt = None;
    app.notice = None;
    std::thread::spawn(move || {
        let result = auth::request_xai_device_code()
            .and_then(|device| {
                let browser_opened = webbrowser::open(&device.verification_uri).is_ok();
                let _ = tx.send(AuthEvent::DeviceCode {
                    url: device.verification_uri.clone(),
                    code: device.user_code.clone(),
                    browser_opened,
                });
                auth::poll_xai(&device, &thread_cancel)
            })
            .and_then(|credential| auth::save_oauth("xai", credential));
        let _ = tx.send(match result {
            Ok(()) => AuthEvent::Done,
            Err(err) => AuthEvent::Failed(err),
        });
    });
}

fn save_api_key(app: &mut App) {
    let key = std::mem::take(&mut app.input);
    app.cursor = 0;
    app.mode = Mode::Chat;
    match auth::save_api_key("xai", &key) {
        Ok(()) => {
            app.notice = Some("saved xAI API key".into());
            reload_config(app);
        }
        Err(err) => app.notice = Some(err),
    }
}

fn logout_xai(app: &mut App) {
    match auth::logout("xai") {
        Ok(true) => {
            app.notice = Some("logged out of xAI".into());
            reload_config(app);
        }
        Ok(false) => app.notice = Some("not logged in to xAI".into()),
        Err(err) => app.notice = Some(err),
    }
}

fn reload_config(app: &mut App) {
    let loaded = lua::load();
    app.config = loaded.config.clone();
    app.startup_config = loaded.config;
    app.models = loaded.models;
    if let Some(notice) = loaded.notice {
        app.notice = Some(notice);
    }
}

fn edit_config(app: &mut App) {
    let editor = std::env::var("VISUAL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            std::env::var("EDITOR")
                .ok()
                .filter(|value| !value.trim().is_empty())
        });
    let Some(editor) = editor else {
        app.notice = Some("set VISUAL or EDITOR to edit init.lua".into());
        return;
    };

    let path = mission::home().join("init.lua");
    if let Some(parent) = path.parent()
        && let Err(err) = std::fs::create_dir_all(parent)
    {
        app.notice = Some(format!("config: {err}"));
        return;
    }
    if !path.exists()
        && let Err(err) = std::fs::File::create(&path)
    {
        app.notice = Some(format!("config: {err}"));
        return;
    }

    terminal::suspend();
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg("exec $0 \"$@\"")
        .arg(editor)
        .arg(&path)
        .status();
    let mut terminal = terminal::resume();

    match status {
        Ok(_) => {
            app.notice = Some("config reloaded".into());
            reload_config(app);
        }
        Err(err) => app.notice = Some(format!("editor: {err}")),
    }
    let _ = terminal.clear();
    let _ = terminal.draw(|frame| draw(frame, app));
}

fn new_mission(app: &mut App) {
    if app.cancel.is_some() {
        return;
    }
    app.messages.clear();
    app.config = app.startup_config.clone();
    invalidate_paint(app);
    app.mission = None;
    app.usage = Usage::default();
    app.last_prompt = 0;
    app.notice = Some("new mission".into());
    jump_to_tail(app);
}

fn name_mission(app: &mut App, name: &str) {
    let name = name.trim();
    if name.is_empty() {
        app.notice = Some("usage: /name <text>".into());
        return;
    }
    persist_value(app, &serde_json::json!({ "type": "name", "name": name }));
    if let Some(m) = &mut app.mission {
        m.name = Some(name.to_string());
    }
}

fn show_mission(app: &mut App) {
    match &app.mission {
        Some(m) => {
            app.notice = Some(format!("{}  {}", m.label(), m.path.display()));
        }
        None => app.notice = Some("no mission yet".into()),
    }
}

fn open_model(app: &mut App) {
    if app.models.is_empty() {
        app.notice = Some("no configured models".into());
        return;
    }
    let cursor = app
        .config
        .as_ref()
        .and_then(|cfg| {
            app.models
                .iter()
                .position(|m| m.provider == cfg.provider && m.id == cfg.model)
        })
        .unwrap_or(0);
    app.mode = Mode::Model {
        items: app.models.clone(),
        cursor,
    };
}

fn select_model(app: &mut App, item: lua::ModelChoice, persist: bool) {
    let Some(config) = item.config else {
        app.notice = Some(item.error.unwrap_or_else(|| "model is unavailable".into()));
        return;
    };
    app.config = Some(config);
    app.notice = Some(format!("model: {} / {}", item.provider, item.id));
    if persist {
        persist_value(app, &mission::model_line(&item.provider, &item.id));
    }
}

fn restore_model(app: &mut App, provider: &str, id: &str) -> bool {
    let choice = app
        .models
        .iter()
        .find(|m| m.provider == provider && m.id == id)
        .cloned();
    if let Some(item) = choice.filter(|m| m.config.is_some()) {
        select_model(app, item, false);
        true
    } else {
        false
    }
}

fn open_resume(app: &mut App) {
    match mission::list() {
        Ok(items) if items.is_empty() => app.notice = Some("no missions in this directory".into()),
        Ok(items) => {
            app.mode = Mode::Resume { items, cursor: 0 };
        }
        Err(err) => app.notice = Some(format!("resume: {err}")),
    }
}

fn resume_prefix(app: &mut App, prefix: &str) {
    let prefix = prefix.trim();
    match mission::list() {
        Ok(items) => {
            if let Some(meta) = items.into_iter().find(|m| {
                m.id.starts_with(prefix) || m.name.as_deref().is_some_and(|n| n.starts_with(prefix))
            }) {
                load_mission(app, &meta.path);
            } else {
                app.notice = Some(format!("no mission matching {prefix}"));
            }
        }
        Err(err) => app.notice = Some(format!("resume: {err}")),
    }
}

fn load_mission(app: &mut App, path: &std::path::Path) {
    match mission::load(path) {
        Ok((loaded, saved)) => {
            app.config = app.startup_config.clone();
            let persisted_model = saved.iter().rev().find_map(|s| match s {
                mission::Saved::Model { provider, id } => Some((provider.clone(), id.clone())),
                _ => None,
            });
            app.messages = saved
                .into_iter()
                .filter_map(|s| match s {
                    mission::Saved::Model { .. } => None,
                    mission::Saved::User(text) => Some(Message::user(text)),
                    mission::Saved::Assistant { text, tool_calls } => {
                        let mut msg = Message::assistant();
                        msg.text = text;
                        msg.tool_calls = tool_calls;
                        Some(msg)
                    }
                    mission::Saved::Tool { id, title, content } => {
                        Some(Message::tool(id, title, content))
                    }
                })
                .collect();
            app.mission = Some(loaded);
            invalidate_paint(app);
            app.usage = Usage::default();
            app.last_prompt = 0;
            app.notice = None;
            if let Some((provider, id)) = persisted_model
                && !restore_model(app, &provider, &id)
            {
                app.notice = Some(format!(
                    "saved model {provider} / {id} is no longer configured; using startup default"
                ));
            }
            jump_to_tail(app);
        }
        Err(err) => app.notice = Some(format!("resume: {err}")),
    }
}

fn send_prompt(app: &mut App, line: String) {
    if app.config.is_none() {
        app.notice = Some("no model configured".into());
        return;
    }
    if let Some(window) = app.config.as_ref().and_then(Config::context_window)
        && window > 0
        && app.last_prompt >= window
    {
        app.notice = Some("context window full".into());
        return;
    }
    app.notice = None;
    app.rounds = 0;
    app.preamble = prompt::preamble();
    persist_value(app, &mission::user_line(&line));
    app.messages.push(Message::user(line));
    jump_to_tail(app);
    let cancel = Arc::new(AtomicBool::new(false));
    app.cancel = Some(cancel);
    continue_turn(app);
}

fn continue_turn(app: &mut App) {
    let Some(cfg) = app.config.clone() else {
        return;
    };
    let Some(cancel) = app.cancel.clone() else {
        return;
    };
    app.messages.push(Message::assistant());
    let history = api_history(app.preamble.as_deref(), &app.messages);
    let (tx, rx) = mpsc::channel();
    app.stream_rx = Some(rx);
    std::thread::spawn(move || complete::stream(cfg, history, cancel, tx));
}

fn api_history(preamble: Option<&str>, messages: &[Message]) -> Vec<ChatMessage> {
    let mut out = Vec::new();
    if let Some(text) = preamble {
        out.push(ChatMessage::User(text.to_string()));
    }
    out.extend(
        messages
            .iter()
            .filter(|m| {
                !m.text.is_empty() || matches!(m.role, Role::User) || !m.tool_calls.is_empty()
            })
            .map(|m| match m.role {
                Role::User => ChatMessage::User(m.text.clone()),
                Role::Assistant => ChatMessage::Assistant {
                    content: m.text.clone(),
                    tool_calls: m.tool_calls.clone(),
                },
                Role::Tool => ChatMessage::Tool {
                    id: m.tool_id.clone(),
                    content: m.text.clone(),
                },
            }),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::{char_wrap, cursor_xy, painted_lines, working_text};
    use ratatui::crossterm::event::{MouseEvent, MouseEventKind};

    fn test_app() -> App {
        App {
            input: String::new(),
            cursor: 0,
            notice: None,
            messages: Vec::new(),
            config: None,
            startup_config: None,
            models: Vec::new(),
            stream_rx: None,
            cancel: None,
            rounds: 0,
            usage: Usage::default(),
            last_prompt: 0,
            preamble: None,
            mission: None,
            mode: Mode::Chat,
            complete_sel: 0,
            quit: false,
            scroll: 0,
            follow: true,
            transcript_w: 0,
            transcript_h: 0,
            paint_width: 0,
            paint_frozen: Vec::new(),
            paint_upto: 0,
            paint_prev_tool: false,
            history: Vec::new(),
            history_cursor: None,
            history_draft: String::new(),
            search: None,
            auth_rx: None,
            auth_cancel: None,
            auth_prompt: None,
        }
    }

    fn key(modifiers: KeyModifiers, code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn multiline_paste_inserts_without_sending() {
        let mut app = test_app();
        app.input = "ab".into();
        app.cursor = 1;
        on_paste(&mut app, "one\r\ntwo\rthree");
        assert_eq!(app.input, "aone\ntwo\nthreeb");
        assert_eq!(app.cursor, 14);
        assert!(app.messages.is_empty());
        assert_eq!(app.notice, None);
    }

    #[test]
    fn shift_enter_inserts_newline() {
        let mut app = test_app();
        app.input = "ab".into();
        app.cursor = 1;
        on_key(&mut app, key(KeyModifiers::SHIFT, KeyCode::Enter));
        assert_eq!(app.input, "a\nb");
        assert_eq!(app.cursor, 2);
        assert!(app.messages.is_empty());
    }

    #[test]
    fn ctrl_j_inserts_newline() {
        let mut app = test_app();
        app.input = "hi".into();
        app.cursor = 2;
        on_key(&mut app, key(KeyModifiers::CONTROL, KeyCode::Char('j')));
        assert_eq!(app.input, "hi\n");
        assert_eq!(app.cursor, 3);
    }

    #[test]
    fn enter_still_sends() {
        let mut app = test_app();
        app.input = "hi".into();
        app.cursor = 2;
        on_key(&mut app, key(KeyModifiers::NONE, KeyCode::Enter));
        assert_eq!(app.input, "");
        assert_eq!(app.notice.as_deref(), Some("no model configured"));
    }

    #[test]
    fn shift_enter_does_not_accept_completion() {
        let mut app = test_app();
        app.input = "/he".into();
        app.cursor = 3;
        on_key(&mut app, key(KeyModifiers::SHIFT, KeyCode::Enter));
        assert_eq!(app.input, "/he\n");
        assert_eq!(app.notice, None);
    }

    #[test]
    fn arrows_walk_history_and_restore_draft() {
        let mut app = test_app();
        app.history = vec!["one".into(), "two".into()];
        app.input = "draft".into();
        app.cursor = 5;
        on_key(&mut app, key(KeyModifiers::NONE, KeyCode::Up));
        assert_eq!(app.input, "two");
        on_key(&mut app, key(KeyModifiers::NONE, KeyCode::Up));
        assert_eq!(app.input, "one");
        on_key(&mut app, key(KeyModifiers::NONE, KeyCode::Down));
        on_key(&mut app, key(KeyModifiers::NONE, KeyCode::Down));
        assert_eq!(app.input, "draft");
    }

    #[test]
    fn reverse_search_cycles_and_escape_restores_draft() {
        let mut app = test_app();
        app.history = vec!["cargo test".into(), "git status".into(), "cargo fmt".into()];
        app.input = "draft".into();
        app.cursor = 5;
        on_key(&mut app, key(KeyModifiers::CONTROL, KeyCode::Char('r')));
        on_key(&mut app, key(KeyModifiers::NONE, KeyCode::Char('c')));
        assert_eq!(app.search.as_ref().unwrap().matched, Some(2));
        on_key(&mut app, key(KeyModifiers::CONTROL, KeyCode::Char('r')));
        assert_eq!(app.search.as_ref().unwrap().matched, Some(0));
        on_key(&mut app, key(KeyModifiers::NONE, KeyCode::Esc));
        assert_eq!(app.input, "draft");
    }

    #[test]
    fn reverse_search_enter_accepts_without_submitting() {
        let mut app = test_app();
        app.history = vec!["cargo test".into()];
        on_key(&mut app, key(KeyModifiers::CONTROL, KeyCode::Char('r')));
        on_key(&mut app, key(KeyModifiers::NONE, KeyCode::Char('t')));
        on_key(&mut app, key(KeyModifiers::NONE, KeyCode::Enter));
        assert_eq!(app.input, "cargo test");
        assert!(app.messages.is_empty());
    }

    #[test]
    fn char_wrap_keeps_hard_newlines() {
        assert_eq!(char_wrap("ab\ncd", 10), vec!["ab", "cd"]);
        assert_eq!(char_wrap("hello\n", 10), vec!["hello", ""]);
        assert_eq!(char_wrap("abcd", 2), vec!["ab", "cd"]);
        assert_eq!(char_wrap("ab\ncdef", 2), vec!["ab", "cd", "ef"]);
    }

    #[test]
    fn cursor_follows_hard_newline() {
        assert_eq!(cursor_xy("ab\ncd", 3, 10), (1, 0));
        assert_eq!(cursor_xy("ab\ncd", 5, 10), (1, 2));
        assert_eq!(cursor_xy("hello", 5, 5), (1, 0));
    }

    #[test]
    fn tools_in_a_round_keep_call_order() {
        let cancel = AtomicBool::new(false);
        let calls = vec![
            ToolCall {
                id: "1".into(),
                name: "read".into(),
                arguments: r#"{"path":"Cargo.toml"}"#.into(),
            },
            ToolCall {
                id: "2".into(),
                name: "read".into(),
                arguments: r#"{"path":"LICENSE"}"#.into(),
            },
        ];
        let results = run_tools_parallel(&calls, &cancel);
        assert_eq!(results[0].id, "1");
        assert_eq!(results[1].id, "2");
        assert!(results[0].content.contains("lunar"));
        assert!(results[1].content.contains("MIT"));
    }

    #[test]
    fn paint_cache_freezes_finished_messages() {
        let mut app = test_app();
        app.messages.push(Message::user("hello".into()));
        let first = painted_lines(&mut app, 40).len();
        assert_eq!(app.paint_upto, 1);
        let frozen = app.paint_frozen.len();
        app.messages.push(Message::user("again".into()));
        let second = painted_lines(&mut app, 40).len();
        assert_eq!(app.paint_upto, 2);
        assert_eq!(app.paint_frozen.len(), frozen + second - first);
    }

    #[test]
    fn truncated_calls_are_not_executed() {
        let calls = vec![ToolCall {
            id: "1".into(),
            name: "bash".into(),
            arguments: r#"{"command":"echo should-not-run"}"#.into(),
        }];
        let results = skipped_truncated(&calls);
        assert_eq!(results[0].id, "1");
        assert!(results[0].content.contains("token limit"));
        assert!(!results[0].content.contains("should-not-run"));
    }

    #[test]
    fn working_text_is_thinking_until_tools() {
        let mut app = test_app();
        app.messages.push(Message::assistant());
        assert_eq!(working_text(&app), " Thinking...");
        app.messages.last_mut().unwrap().tool_calls.push(ToolCall {
            id: "1".into(),
            name: "bash".into(),
            arguments: "{}".into(),
        });
        assert_eq!(working_text(&app), " Running tools...");
    }

    #[test]
    fn esc_aborts_a_turn_immediately() {
        let mut app = test_app();
        app.cancel = Some(Arc::new(AtomicBool::new(false)));
        let (_tx, rx) = mpsc::channel();
        app.stream_rx = Some(rx);
        app.messages.push(Message::assistant());
        on_key(&mut app, key(KeyModifiers::NONE, KeyCode::Esc));
        assert!(app.cancel.is_none());
        assert!(app.stream_rx.is_none());
        assert!(app.messages.is_empty());
        assert_eq!(app.notice.as_deref(), Some("aborted"));
    }

    fn tall_app() -> App {
        let mut app = test_app();
        app.transcript_w = 40;
        app.transcript_h = 8;
        for i in 0..12 {
            app.messages.push(Message::user(format!("line {i}")));
        }
        let max = painted_lines(&mut app, 40).len().saturating_sub(8);
        app.scroll = max;
        app.follow = true;
        app
    }

    #[test]
    fn page_up_leaves_follow() {
        let mut app = tall_app();
        on_key(&mut app, key(KeyModifiers::NONE, KeyCode::PageUp));
        assert!(!app.follow);
        assert!(app.scroll < painted_lines(&mut app, 40).len().saturating_sub(8));
    }

    #[test]
    fn page_down_to_end_follows_again() {
        let mut app = tall_app();
        on_key(&mut app, key(KeyModifiers::NONE, KeyCode::PageUp));
        on_key(&mut app, key(KeyModifiers::NONE, KeyCode::PageDown));
        on_key(&mut app, key(KeyModifiers::NONE, KeyCode::PageDown));
        assert!(app.follow);
    }

    #[test]
    fn ctrl_home_goes_to_top() {
        let mut app = tall_app();
        on_key(&mut app, key(KeyModifiers::CONTROL, KeyCode::Home));
        assert_eq!(app.scroll, 0);
        assert!(!app.follow);
    }

    #[test]
    fn wheel_does_nothing_in_resume() {
        let mut app = tall_app();
        app.mode = Mode::Resume {
            items: Vec::new(),
            cursor: 0,
        };
        let before = app.scroll;
        on_mouse(
            &mut app,
            MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: 0,
                row: 0,
                modifiers: KeyModifiers::NONE,
            },
        );
        assert_eq!(app.scroll, before);
        assert!(app.follow);
    }

    #[test]
    fn submit_jumps_to_tail() {
        let mut app = tall_app();
        app.follow = false;
        app.scroll = 0;
        jump_to_tail(&mut app);
        assert!(app.follow);
    }
}
