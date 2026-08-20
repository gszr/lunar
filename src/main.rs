mod commands;
mod complete;
mod history;
mod lua;
mod mission;
mod prompt;
mod render;
mod splash;
mod tools;

use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, KeyboardEnhancementFlags, MouseEvent, MouseEventKind,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use ratatui::crossterm::execute;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::{DefaultTerminal, Frame};

use complete::{ChatMessage, Config, StreamEvent, ToolCall, ToolResult, Usage};

const MAX_ROUNDS: u32 = 20;

struct App {
    input: String,
    cursor: usize,
    notice: Option<String>,
    messages: Vec<Message>,
    config: Option<Config>,
    startup_config: Option<Config>,
    models: Vec<lua::ModelChoice>,
    stream_rx: Option<Receiver<StreamEvent>>,
    cancel: Option<Arc<AtomicBool>>,
    rounds: u32,
    usage: Usage,
    last_prompt: u32,
    preamble: Option<String>,
    mission: Option<mission::Mission>,
    mode: Mode,
    complete_sel: usize,
    quit: bool,
    scroll: usize,
    follow: bool,
    transcript_w: u16,
    transcript_h: u16,
    paint_width: usize,
    paint_frozen: Vec<Line<'static>>,
    paint_upto: usize,
    paint_prev_tool: bool,
    history: Vec<String>,
    history_cursor: Option<usize>,
    history_draft: String,
    search: Option<HistorySearch>,
}

struct HistorySearch {
    draft: String,
    draft_cursor: usize,
    query: String,
    matched: Option<usize>,
}

enum Mode {
    Chat,
    Resume {
        items: Vec<mission::Meta>,
        cursor: usize,
    },
    Model {
        items: Vec<lua::ModelChoice>,
        cursor: usize,
    },
}

struct Message {
    role: Role,
    text: String,
    thinking: String,
    tool_calls: Vec<ToolCall>,
    tool_id: String,
    tool_title: String,
}

enum Role {
    User,
    Assistant,
    Tool,
}

impl Message {
    fn user(text: String) -> Self {
        Self {
            role: Role::User,
            text,
            thinking: String::new(),
            tool_calls: Vec::new(),
            tool_id: String::new(),
            tool_title: String::new(),
        }
    }

    fn assistant() -> Self {
        Self {
            role: Role::Assistant,
            text: String::new(),
            thinking: String::new(),
            tool_calls: Vec::new(),
            tool_id: String::new(),
            tool_title: String::new(),
        }
    }

    fn tool(id: String, title: String, content: String) -> Self {
        Self {
            role: Role::Tool,
            text: content,
            thinking: String::new(),
            tool_calls: Vec::new(),
            tool_id: id,
            tool_title: title,
        }
    }
}

fn main() -> io::Result<()> {
    let resume_last = std::env::args()
        .skip(1)
        .any(|a| a == "-c" || a == "--continue");
    let loaded = lua::load();
    let startup_config = loaded.config.clone();
    let mut terminal = ratatui::init();
    enable_enhanced_keys();
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
    let result = run(&mut terminal, &mut app);
    restore_terminal();
    result
}

/// Shift+Enter is `\r` unless the terminal reports modifiers (kitty keyboard protocol).
fn enable_enhanced_keys() {
    let _ = execute!(
        io::stdout(),
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES),
        EnableMouseCapture,
    );
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        pop_enhanced_keys();
        let _ = execute!(io::stdout(), DisableMouseCapture);
        hook(info);
    }));
}

fn pop_enhanced_keys() {
    let _ = execute!(io::stdout(), PopKeyboardEnhancementFlags);
}

fn restore_terminal() {
    pop_enhanced_keys();
    let _ = execute!(io::stdout(), DisableMouseCapture);
    ratatui::restore();
}

fn run(terminal: &mut DefaultTerminal, app: &mut App) -> io::Result<()> {
    while !app.quit {
        drain_stream(app);
        terminal.draw(|frame| draw(frame, app))?;
        let wait = if app.cancel.is_some() {
            Duration::from_millis(16)
        } else {
            Duration::from_secs(3600)
        };
        if event::poll(wait)? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => on_key(app, key),
                Event::Mouse(mouse) => on_mouse(app, mouse),
                _ => {}
            }
        }
    }
    Ok(())
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
        app.notice = Some("stopped after too many tool rounds".into());
        return;
    }
    continue_turn(app);
}

fn on_key(app: &mut App, key: KeyEvent) {
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
                "/quit /new /resume /model /name /session /context /help    tab cycle    shift+enter / ctrl+j newline    esc abort    ctrl+c quits"
                    .into(),
            );
        }
        "/new" => new_mission(app),
        "/resume" => open_resume(app),
        "/model" => open_model(app),
        "/session" => show_session(app),
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

fn show_session(app: &mut App) {
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

const EDITOR_MAX_LINES: u16 = 8;

fn editor_lines(input: &str, width: u16) -> Vec<String> {
    char_wrap(input, width.max(1) as usize)
}

fn char_wrap(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![text.to_string()];
    }
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        if paragraph.is_empty() {
            lines.push(String::new());
            continue;
        }
        let mut current = String::new();
        let mut cols = 0;
        for c in paragraph.chars() {
            if cols == width {
                lines.push(std::mem::take(&mut current));
                cols = 0;
            }
            current.push(c);
            cols += 1;
        }
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn cursor_xy(text: &str, cursor: usize, width: usize) -> (u16, u16) {
    if width == 0 {
        return (0, 0);
    }
    let prefix = &text[..cursor.min(text.len())];
    let lines = char_wrap(prefix, width);
    let row = lines.len().saturating_sub(1) as u16;
    let col = lines.last().map(|l| l.chars().count() as u16).unwrap_or(0);
    if col as usize == width {
        (row + 1, 0)
    } else {
        (row, col)
    }
}

fn editor_height(input: &str, cursor: usize, width: u16) -> u16 {
    let lines = editor_lines(input, width).len() as u16;
    let (row, _) = cursor_xy(input, cursor, width.max(1) as usize);
    lines.max(row + 1).min(EDITOR_MAX_LINES) + 2
}

fn model_picker_height(app: &App) -> u16 {
    match &app.mode {
        Mode::Model { items, .. } => items.len().saturating_add(1).min(u16::MAX as usize) as u16,
        _ => 0,
    }
}

fn draw(frame: &mut Frame, app: &mut App) {
    let working = u16::from(app.cancel.is_some());
    let ed_h = editor_height(app.input.as_str(), app.cursor, frame.area().width)
        .saturating_add(model_picker_height(app));
    let found = if app.search.is_none() {
        commands::matches(&app.input)
    } else {
        Vec::new()
    };
    let selected = commands::clamp_selected(app.complete_sel, found.len());
    let complete_h = if found.is_empty() {
        0
    } else {
        commands::visible(&found, selected).1.len() as u16
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(working),
            Constraint::Length(ed_h),
            Constraint::Length(complete_h),
            Constraint::Length(2),
        ])
        .split(frame.area());

    app.transcript_w = chunks[1].width;
    app.transcript_h = chunks[1].height;
    draw_header(frame, chunks[0], app);
    draw_messages(frame, chunks[1], app);
    if working == 1 {
        draw_working(frame, chunks[2], working_text(app));
    }
    draw_editor(frame, chunks[3], app);
    if complete_h > 0 {
        draw_complete(frame, chunks[4], &found, selected);
    }
    draw_footer(frame, chunks[5], app);
}

const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

fn working_text(app: &App) -> &'static str {
    if matches!(
        app.messages.last(),
        Some(Message {
            role: Role::Assistant,
            tool_calls,
            ..
        }) if !tool_calls.is_empty()
    ) {
        " Running tools..."
    } else {
        " Thinking..."
    }
}

fn draw_working(frame: &mut Frame, area: Rect, text: &'static str) {
    static ORIGIN: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let origin = ORIGIN.get_or_init(Instant::now);
    let frame_i = (origin.elapsed().as_millis() / 80) as usize % SPINNER.len();
    let line = Line::from(vec![
        Span::styled(SPINNER[frame_i], Style::default().fg(splash::GOLD)),
        Span::styled(text, Style::default().fg(splash::ASH)),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn draw_header(frame: &mut Frame, area: Rect, app: &App) {
    let version = env!("CARGO_PKG_VERSION");
    let left = vec![
        Span::styled("lunar", Style::default().fg(splash::GOLD)),
        Span::styled(format!("  {version}"), Style::default().fg(splash::DUST)),
    ];
    let right = match &app.mission {
        Some(m) => Span::styled(m.label(), Style::default().fg(splash::ASH)),
        None => Span::raw(""),
    };
    frame.render_widget(spread(left, right, area.width), area);
}

fn draw_messages(frame: &mut Frame, area: Rect, app: &mut App) {
    if let Mode::Resume { items, cursor } = &app.mode {
        draw_resume(frame, area, items, *cursor);
        return;
    }
    if app.messages.is_empty() {
        if app.notice.as_deref().is_some_and(|n| n.contains('\n')) {
            draw_notice(frame, area, app.notice.as_deref().unwrap());
            return;
        }
        draw_splash(frame, area, app);
        return;
    }
    let width = area.width.max(1) as usize;
    let lines = painted_lines(app, width);
    let height = area.height as usize;
    let max = lines.len().saturating_sub(height);
    if app.follow {
        app.scroll = max;
    } else {
        app.scroll = app.scroll.min(max);
    }
    let start = app.scroll;
    let end = (start + height).min(lines.len());
    frame.render_widget(Paragraph::new(lines[start..end].to_vec()), area);
}

fn invalidate_paint(app: &mut App) {
    app.paint_frozen.clear();
    app.paint_upto = 0;
    app.paint_prev_tool = false;
}

fn freeze_end(app: &App) -> usize {
    let n = app.messages.len();
    if n == 0 {
        return 0;
    }
    if app.cancel.is_some() && matches!(app.messages[n - 1].role, Role::Assistant) {
        n - 1
    } else {
        n
    }
}

fn painted_lines(app: &mut App, width: usize) -> Vec<Line<'static>> {
    if width != app.paint_width {
        app.paint_width = width;
        invalidate_paint(app);
    }
    let end = freeze_end(app);
    if end < app.paint_upto {
        invalidate_paint(app);
    }
    if end > app.paint_upto {
        let (chunk, prev_tool) = paint_slice(
            &app.messages[app.paint_upto..end],
            width,
            !app.paint_frozen.is_empty(),
            app.paint_prev_tool,
        );
        app.paint_frozen.extend(chunk);
        app.paint_prev_tool = prev_tool;
        app.paint_upto = end;
    }
    let mut lines = app.paint_frozen.clone();
    if end < app.messages.len() {
        let (tail, _) = paint_slice(
            &app.messages[end..],
            width,
            !lines.is_empty(),
            app.paint_prev_tool,
        );
        lines.extend(tail);
    }
    if let Some(notice) = &app.notice {
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
        lines.extend(notice_lines(notice));
    }
    lines
}

fn paint_slice(
    messages: &[Message],
    width: usize,
    need_gap: bool,
    mut prev_tool: bool,
) -> (Vec<Line<'static>>, bool) {
    let mut lines = Vec::new();
    let mut gap = need_gap;
    for msg in messages {
        if matches!(msg.role, Role::Assistant) && msg.text.is_empty() && msg.thinking.is_empty() {
            continue;
        }
        let is_tool = matches!(msg.role, Role::Tool);
        if gap && !(is_tool && prev_tool) {
            lines.push(Line::from(""));
        }
        match msg.role {
            Role::User => lines.extend(render::user_bar(&msg.text, width)),
            Role::Assistant => {
                if !msg.thinking.is_empty() {
                    lines.extend(render::thinking_preview(&msg.thinking, width));
                    if !msg.text.is_empty() {
                        lines.push(Line::from(""));
                    }
                }
                if !msg.text.is_empty() {
                    lines.extend(render::assistant(&msg.text, width));
                }
            }
            Role::Tool => lines.extend(render::tool_card(&msg.tool_title, &msg.text, width)),
        }
        prev_tool = is_tool;
        gap = true;
    }
    (lines, prev_tool)
}

const WHEEL_LINES: usize = 3;

fn page_delta(app: &App) -> isize {
    app.transcript_h.saturating_sub(1).max(1) as isize
}

fn jump_to_tail(app: &mut App) {
    app.follow = true;
}

fn scroll_home(app: &mut App) {
    if !matches!(app.mode, Mode::Chat) || app.messages.is_empty() {
        return;
    }
    app.scroll = 0;
    app.follow = false;
}

fn scroll_by(app: &mut App, delta: isize) {
    if !matches!(app.mode, Mode::Chat) || app.messages.is_empty() {
        return;
    }
    let height = app.transcript_h as usize;
    if height == 0 {
        return;
    }
    let width = app.transcript_w.max(1) as usize;
    let max = painted_lines(app, width).len().saturating_sub(height);
    let next = if delta < 0 {
        app.scroll.saturating_sub(delta.unsigned_abs())
    } else {
        app.scroll.saturating_add(delta as usize)
    };
    if next >= max {
        app.scroll = max;
        app.follow = true;
    } else {
        app.scroll = next;
        app.follow = false;
    }
}

fn on_mouse(app: &mut App, mouse: MouseEvent) {
    match mouse.kind {
        MouseEventKind::ScrollUp => scroll_by(app, -(WHEEL_LINES as isize)),
        MouseEventKind::ScrollDown => scroll_by(app, WHEEL_LINES as isize),
        _ => {}
    }
}

fn notice_lines(notice: &str) -> Vec<Line<'static>> {
    notice
        .lines()
        .map(|line| {
            Line::from(Span::styled(
                line.to_string(),
                Style::default().fg(splash::GOLD),
            ))
        })
        .collect()
}

fn draw_notice(frame: &mut Frame, area: Rect, notice: &str) {
    let lines = notice_lines(notice);
    let skip = lines.len().saturating_sub(area.height as usize);
    frame.render_widget(Paragraph::new(lines[skip..].to_vec()), area);
}

fn draw_model(frame: &mut Frame, area: Rect, items: &[lua::ModelChoice], cursor: usize) {
    let mut lines = vec![Line::from(Span::styled(
        "model  j/k  enter  esc",
        Style::default().fg(splash::ASH),
    ))];
    for (i, item) in items.iter().enumerate() {
        let alias = item
            .alias
            .as_deref()
            .map(|a| format!("{a}  "))
            .unwrap_or_default();
        let unavailable = item
            .error
            .as_deref()
            .map(|e| format!("  unavailable: {e}"))
            .unwrap_or_default();
        let style = if i == cursor {
            Style::default().fg(splash::GOLD)
        } else if item.config.is_none() {
            Style::default().fg(splash::DUST)
        } else {
            Style::default().fg(splash::BONE)
        };
        lines.push(Line::from(Span::styled(
            format!("  {} / {}{}{}", item.provider, alias, item.id, unavailable),
            style,
        )));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

fn draw_resume(frame: &mut Frame, area: Rect, items: &[mission::Meta], cursor: usize) {
    let mut lines = vec![Line::from(Span::styled(
        "resume  j/k  enter  esc",
        Style::default().fg(splash::ASH),
    ))];
    for (i, item) in items.iter().enumerate() {
        let label = item.label();
        let style = if i == cursor {
            Style::default().fg(splash::GOLD)
        } else {
            Style::default().fg(splash::BONE)
        };
        lines.push(Line::from(Span::styled(format!("  {label}"), style)));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

fn draw_splash(frame: &mut Frame, area: Rect, app: &App) {
    let art = splash::lines();
    let art_h = art.len() as u16;
    let title = Line::from(Span::styled("lunar", Style::default().fg(splash::GOLD)));
    let tag = Line::from(Span::styled(
        "lua-scriptable",
        Style::default().fg(splash::ASH),
    ));
    let tag2 = Line::from(Span::styled(
        "extensible coding harness",
        Style::default().fg(splash::ASH),
    ));
    let notice = app.notice.as_deref().unwrap_or("");
    let extra = 4 + u16::from(!notice.is_empty());
    let block_h = art_h.saturating_add(extra);
    let art_w = splash::width();
    let splash_area = Rect {
        x: area.x + area.width.saturating_sub(art_w) / 2,
        y: area.y + area.height.saturating_sub(block_h) / 2,
        width: art_w.min(area.width),
        height: block_h.min(area.height),
    };

    let mut lines = art;
    lines.push(Line::from(""));
    lines.push(title.centered());
    lines.push(tag.centered());
    lines.push(tag2.centered());
    if !notice.is_empty() {
        lines.push(Line::from(Span::styled(notice, Style::default().fg(splash::GOLD))).centered());
    }
    frame.render_widget(
        Paragraph::new(lines).alignment(Alignment::Left),
        splash_area,
    );
}

fn draw_complete(
    frame: &mut Frame,
    area: Rect,
    found: &[&'static commands::Command],
    selected: usize,
) {
    let (start, view) = commands::visible(found, selected);
    let name_w = view
        .iter()
        .map(|cmd| cmd.name.len())
        .max()
        .unwrap_or(0)
        .min(16);
    let width = area.width.max(1) as usize;
    let lines: Vec<Line> = view
        .iter()
        .enumerate()
        .map(|(i, cmd)| {
            let sel = start + i == selected;
            let marker = if sel { "→ " } else { "  " };
            let name = format!("/{:<name_w$}", cmd.name);
            let rest = width.saturating_sub(marker.len() + name.len() + 2);
            let desc = if rest == 0 {
                String::new()
            } else {
                let mut d = cmd.description.to_string();
                if d.chars().count() > rest {
                    d = d.chars().take(rest.saturating_sub(1)).collect();
                    d.push('…');
                }
                d
            };
            if sel {
                Line::from(vec![
                    Span::styled(marker, Style::default().fg(splash::GOLD)),
                    Span::styled(name, Style::default().fg(splash::GOLD)),
                    Span::raw("  "),
                    Span::styled(desc, Style::default().fg(splash::ASH)),
                ])
            } else {
                Line::from(vec![
                    Span::styled(marker, Style::default().fg(splash::DUST)),
                    Span::styled(name, Style::default().fg(splash::BONE)),
                    Span::raw("  "),
                    Span::styled(desc, Style::default().fg(splash::DUST)),
                ])
            }
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
}

fn draw_editor(frame: &mut Frame, area: Rect, app: &App) {
    if let Mode::Model { items, cursor } = &app.mode {
        let block = Block::default()
            .borders(Borders::TOP | Borders::BOTTOM)
            .border_style(Style::default().fg(splash::DUST));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if inner.height == 0 {
            return;
        }
        let picker_h = (items.len().saturating_add(1) as u16).min(inner.height);
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(picker_h)])
            .split(inner);
        draw_editor_input(frame, chunks[0], app);
        draw_model(frame, chunks[1], items, *cursor);
        return;
    }
    if let Some(search) = &app.search {
        let result = search
            .matched
            .and_then(|i| app.history.get(i))
            .map(String::as_str)
            .unwrap_or("failing reverse-i-search");
        let line = format!("(reverse-i-search)`{}': {}", search.query, result);
        let block = Block::default()
            .borders(Borders::TOP | Borders::BOTTOM)
            .border_style(Style::default().fg(splash::GOLD));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                line,
                Style::default().fg(splash::BONE),
            ))),
            inner,
        );
        let x = inner.x
            + inner
                .width
                .saturating_sub(1)
                .min((20 + search.query.chars().count()) as u16);
        frame.set_cursor_position((x, inner.y));
        return;
    }
    let block = Block::default()
        .borders(Borders::TOP | Borders::BOTTOM)
        .border_style(Style::default().fg(splash::DUST));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 {
        return;
    }

    draw_editor_input(frame, inner, app);
}

fn draw_editor_input(frame: &mut Frame, area: Rect, app: &App) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let mut lines = editor_lines(app.input.as_str(), area.width);
    let (mut row, col) = cursor_xy(&app.input, app.cursor, area.width as usize);
    if lines.len() > area.height as usize {
        let skip = lines.len() - area.height as usize;
        lines = lines[skip..].to_vec();
        row = row.saturating_sub(skip as u16);
    }
    let styled: Vec<Line> = lines
        .into_iter()
        .map(|s| Line::from(Span::styled(s, Style::default().fg(splash::BONE))))
        .collect();
    frame.render_widget(Paragraph::new(styled), area);

    let cursor_x = area.x + col.min(area.width.saturating_sub(1));
    let cursor_y = area.y + row.min(area.height.saturating_sub(1));
    frame.set_cursor_position((cursor_x, cursor_y));
}

fn draw_footer(frame: &mut Frame, area: Rect, app: &App) {
    let cwd = Line::from(Span::styled(cwd_label(), Style::default().fg(splash::DUST)));
    let stats = stats_line(app);
    let model = match &app.config {
        Some(cfg) => format!("({}) {} • {}", cfg.provider(), cfg.model, "off"),
        None => "no model".into(),
    };
    let stats_row = spread(
        vec![Span::styled(stats, Style::default().fg(splash::DUST))],
        Span::styled(model, Style::default().fg(splash::DUST)),
        area.width,
    );
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);
    frame.render_widget(Paragraph::new(cwd), chunks[0]);
    frame.render_widget(stats_row, chunks[1]);
}

fn stats_line(app: &App) -> String {
    let mut parts = Vec::new();
    if app.usage.input > 0 {
        parts.push(format!("↑{}", format_tokens(app.usage.input)));
    }
    if app.usage.output > 0 {
        parts.push(format!("↓{}", format_tokens(app.usage.output)));
    }
    if app.usage.cache_read > 0 {
        parts.push(format!("R{}", format_tokens(app.usage.cache_read)));
    }
    if app.usage.cache_write > 0 {
        parts.push(format!("W{}", format_tokens(app.usage.cache_write)));
    }
    if let Some(window) = app.config.as_ref().and_then(Config::context_window)
        && window > 0
        && app.last_prompt > 0
    {
        let pct = (f64::from(app.last_prompt) / f64::from(window)) * 100.0;
        parts.push(format!("{pct:.1}%/{}", format_tokens(window)));
    }
    parts.join(" ")
}

fn format_tokens(count: u32) -> String {
    if count < 1000 {
        count.to_string()
    } else if count < 10_000 {
        format!("{:.1}k", f64::from(count) / 1000.0)
    } else if count < 1_000_000 {
        format!("{}k", (count + 500) / 1000)
    } else if count < 10_000_000 {
        format!("{:.1}M", f64::from(count) / 1_000_000.0)
    } else {
        format!("{}M", (count + 500_000) / 1_000_000)
    }
}

fn spread(mut left: Vec<Span<'static>>, right: Span<'static>, width: u16) -> Paragraph<'static> {
    let left_w: usize = left.iter().map(|s| s.content.chars().count()).sum();
    let right_w = right.content.chars().count();
    let gap = width.saturating_sub((left_w + right_w) as u16);
    left.push(Span::raw(" ".repeat(gap as usize)));
    left.push(right);
    Paragraph::new(Line::from(left))
}

#[cfg(test)]
mod tests {
    use super::*;

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
        }
    }

    fn key(modifiers: KeyModifiers, code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, modifiers)
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

fn cwd_label() -> String {
    std::env::current_dir()
        .ok()
        .map(shorten_home)
        .unwrap_or_else(|| ".".into())
}

fn shorten_home(path: PathBuf) -> String {
    match std::env::var_os("HOME") {
        Some(home) => {
            let home = PathBuf::from(home);
            match path.strip_prefix(&home) {
                Ok(rest) => format!("~/{}", rest.display()),
                Err(_) => path.display().to_string(),
            }
        }
        None => path.display().to_string(),
    }
}
