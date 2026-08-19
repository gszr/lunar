mod complete;
mod splash;

use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::{DefaultTerminal, Frame};

use complete::{ChatMessage, Config, StreamEvent};

struct App {
    input: String,
    notice: Option<String>,
    messages: Vec<Message>,
    config: Option<Config>,
    stream_rx: Option<Receiver<StreamEvent>>,
    cancel: Option<Arc<AtomicBool>>,
    quit: bool,
}

struct Message {
    role: Role,
    text: String,
}

enum Role {
    User,
    Assistant,
}

fn main() -> io::Result<()> {
    let mut terminal = ratatui::init();
    let mut app = App {
        input: String::new(),
        notice: None,
        messages: Vec::new(),
        config: Config::from_env(),
        stream_rx: None,
        cancel: None,
        quit: false,
    };
    let result = run(&mut terminal, &mut app);
    ratatui::restore();
    result
}

fn run(terminal: &mut DefaultTerminal, app: &mut App) -> io::Result<()> {
    while !app.quit {
        drain_stream(app);
        terminal.draw(|frame| draw(frame, app))?;
        let wait = if app.stream_rx.is_some() {
            Duration::from_millis(16)
        } else {
            Duration::from_secs(3600)
        };
        if event::poll(wait)? {
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind == KeyEventKind::Press {
                on_key(app, key);
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
    while let Ok(ev) = rx.try_recv() {
        match ev {
            StreamEvent::Delta(text) => {
                if let Some(last) = app.messages.last_mut()
                    && matches!(last.role, Role::Assistant)
                {
                    last.text.push_str(&text);
                }
            }
            other => {
                end = Some(other);
                break;
            }
        }
    }
    if let Some(end) = end {
        finish_stream(app, end);
    }
}

fn finish_stream(app: &mut App, end: StreamEvent) {
    app.stream_rx = None;
    app.cancel = None;
    match end {
        StreamEvent::Done => {
            if matches!(
                app.messages.last(),
                Some(Message {
                    role: Role::Assistant,
                    text,
                }) if text.is_empty()
            ) {
                app.messages.pop();
            }
        }
        StreamEvent::Failed(err) => {
            if matches!(
                app.messages.last(),
                Some(Message {
                    role: Role::Assistant,
                    text,
                }) if text.is_empty()
            ) {
                app.messages.pop();
            }
            app.notice = Some(err);
        }
        StreamEvent::Delta(_) => {}
    }
}

fn on_key(app: &mut App, key: KeyEvent) {
    match (key.modifiers, key.code) {
        (KeyModifiers::CONTROL, KeyCode::Char('c')) => app.quit = true,
        (_, KeyCode::Esc) => {
            if let Some(flag) = &app.cancel {
                flag.store(true, Ordering::Relaxed);
            } else {
                app.input.clear();
            }
        }
        (_, KeyCode::Backspace) => {
            app.input.pop();
        }
        (_, KeyCode::Enter) => submit(app),
        (_, KeyCode::Char(c)) => app.input.push(c),
        _ => {}
    }
}

fn submit(app: &mut App) {
    if app.stream_rx.is_some() {
        return;
    }
    let line = app.input.trim().to_string();
    app.input.clear();
    if line.is_empty() {
        return;
    }
    match line.as_str() {
        "/quit" | "/q" => app.quit = true,
        "/help" => {
            app.notice = Some("/quit  /help    esc abort/clear    ctrl+c quits".into());
        }
        cmd if cmd.starts_with('/') => {
            app.notice = Some(format!("unknown command: {cmd}"));
        }
        _ => send_prompt(app, line),
    }
}

fn send_prompt(app: &mut App, line: String) {
    let Some(cfg) = app.config.clone() else {
        app.notice = Some("no model configured".into());
        return;
    };
    app.notice = None;
    app.messages.push(Message {
        role: Role::User,
        text: line,
    });
    app.messages.push(Message {
        role: Role::Assistant,
        text: String::new(),
    });

    let history: Vec<ChatMessage> = app
        .messages
        .iter()
        .filter(|m| !m.text.is_empty() || matches!(m.role, Role::User))
        .map(|m| ChatMessage {
            role: match m.role {
                Role::User => "user",
                Role::Assistant => "assistant",
            },
            content: m.text.clone(),
        })
        .collect();

    let cancel = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    app.cancel = Some(cancel.clone());
    app.stream_rx = Some(rx);
    std::thread::spawn(move || complete::stream(cfg, history, cancel, tx));
}

fn draw(frame: &mut Frame, app: &App) {
    let working = u16::from(app.stream_rx.is_some());
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(0),
            Constraint::Length(working),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(frame.area());

    draw_header(frame, chunks[0], app);
    draw_messages(frame, chunks[1], app);
    if working == 1 {
        draw_working(frame, chunks[2]);
        draw_editor(frame, chunks[3], app);
        draw_footer(frame, chunks[4], app);
    } else {
        draw_editor(frame, chunks[3], app);
        draw_footer(frame, chunks[4], app);
    }
}

const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

fn draw_working(frame: &mut Frame, area: Rect) {
    static ORIGIN: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let origin = ORIGIN.get_or_init(Instant::now);
    let frame_i = (origin.elapsed().as_millis() / 80) as usize % SPINNER.len();
    let line = Line::from(vec![
        Span::styled(SPINNER[frame_i], Style::default().fg(splash::GOLD)),
        Span::styled(" Thinking...", Style::default().fg(splash::ASH)),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn draw_header(frame: &mut Frame, area: Rect, app: &App) {
    let version = env!("CARGO_PKG_VERSION");
    let status = match &app.config {
        Some(cfg) => cfg.model.as_str(),
        None => "no model configured",
    };
    let header = Paragraph::new(vec![
        Line::from(vec![
            Span::styled("lunar", Style::default().fg(splash::GOLD)),
            Span::styled(format!("  {version}"), Style::default().fg(splash::DUST)),
        ]),
        Line::from(Span::styled(status, Style::default().fg(splash::ASH))),
    ]);
    frame.render_widget(header, area);
}

fn draw_messages(frame: &mut Frame, area: Rect, app: &App) {
    if app.messages.is_empty() {
        draw_splash(frame, area, app);
        return;
    }
    let width = area.width.max(1) as usize;
    let mut lines: Vec<Line> = Vec::new();
    for msg in &app.messages {
        if matches!(msg.role, Role::Assistant) && msg.text.is_empty() {
            continue;
        }
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
        match msg.role {
            Role::User => lines.extend(user_bar(&msg.text, width)),
            Role::Assistant => {
                let style = Style::default().fg(splash::BONE);
                for wrapped in wrap(&msg.text, width) {
                    lines.push(Line::from(Span::styled(wrapped, style)));
                }
            }
        }
    }
    if let Some(notice) = &app.notice {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            notice.as_str(),
            Style::default().fg(splash::GOLD),
        )));
    }
    let skip = lines.len().saturating_sub(area.height as usize);
    frame.render_widget(Paragraph::new(lines[skip..].to_vec()), area);
}

fn draw_splash(frame: &mut Frame, area: Rect, app: &App) {
    let art = splash::lines();
    let art_h = art.len() as u16;
    let title = Line::from(Span::styled("lunar", Style::default().fg(splash::GOLD)));
    let hint = Line::from(Span::styled(
        "a coding harness",
        Style::default().fg(splash::ASH),
    ));
    let notice = app.notice.as_deref().unwrap_or("");
    let extra = 3 + u16::from(!notice.is_empty());
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
    lines.push(hint.centered());
    if !notice.is_empty() {
        lines.push(Line::from(Span::styled(notice, Style::default().fg(splash::GOLD))).centered());
    }
    frame.render_widget(
        Paragraph::new(lines).alignment(Alignment::Left),
        splash_area,
    );
}

fn draw_editor(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(splash::DUST));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let prompt = Paragraph::new(Line::from(Span::styled(
        app.input.as_str(),
        Style::default().fg(splash::BONE),
    )));
    frame.render_widget(prompt, inner);

    let cursor_x = inner.x + app.input.chars().count() as u16;
    if cursor_x < inner.x + inner.width && inner.height > 0 {
        frame.set_cursor_position((cursor_x, inner.y));
    }
}

fn draw_footer(frame: &mut Frame, area: Rect, app: &App) {
    let cwd = cwd_label();
    let left = Span::styled(cwd, Style::default().fg(splash::ASH));
    let right = match &app.config {
        Some(cfg) => Span::styled(cfg.model.as_str(), Style::default().fg(splash::DUST)),
        None => Span::styled("no model", Style::default().fg(splash::DUST)),
    };
    let gap = area
        .width
        .saturating_sub((left.content.len() + right.content.len()) as u16);
    let line = Line::from(vec![left, Span::raw(" ".repeat(gap as usize)), right]);
    frame.render_widget(Paragraph::new(line), area);
}

fn user_bar(text: &str, width: usize) -> Vec<Line<'static>> {
    let style = Style::default()
        .fg(splash::BONE)
        .bg(splash::BAR)
        .add_modifier(Modifier::BOLD);
    wrap(text, width)
        .into_iter()
        .map(|s| {
            let pad = width.saturating_sub(s.chars().count());
            Line::from(Span::styled(format!("{s}{}", " ".repeat(pad)), style))
        })
        .collect()
}

fn wrap(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![text.to_string()];
    }
    let mut out = Vec::new();
    for paragraph in text.split('\n') {
        if paragraph.is_empty() {
            out.push(String::new());
            continue;
        }
        let mut line = String::new();
        for word in paragraph.split(' ') {
            if line.is_empty() {
                line.push_str(word);
            } else if line.chars().count() + 1 + word.chars().count() <= width {
                line.push(' ');
                line.push_str(word);
            } else {
                out.push(std::mem::take(&mut line));
                line.push_str(word);
            }
        }
        if !line.is_empty() {
            out.push(line);
        }
    }
    out
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
