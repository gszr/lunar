mod splash;

use std::io;
use std::path::PathBuf;

use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::{DefaultTerminal, Frame};

struct App {
    input: String,
    notice: Option<String>,
    quit: bool,
}

fn main() -> io::Result<()> {
    let mut terminal = ratatui::init();
    let mut app = App {
        input: String::new(),
        notice: None,
        quit: false,
    };
    let result = run(&mut terminal, &mut app);
    ratatui::restore();
    result
}

fn run(terminal: &mut DefaultTerminal, app: &mut App) -> io::Result<()> {
    while !app.quit {
        terminal.draw(|frame| draw(frame, app))?;
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind == KeyEventKind::Press {
            on_key(app, key);
        }
    }
    Ok(())
}

fn on_key(app: &mut App, key: KeyEvent) {
    match (key.modifiers, key.code) {
        (KeyModifiers::CONTROL, KeyCode::Char('c')) => app.quit = true,
        (_, KeyCode::Esc) => app.input.clear(),
        (_, KeyCode::Backspace) => {
            app.input.pop();
        }
        (_, KeyCode::Enter) => submit(app),
        (_, KeyCode::Char(c)) => app.input.push(c),
        _ => {}
    }
}

fn submit(app: &mut App) {
    let line = app.input.trim().to_string();
    app.input.clear();
    if line.is_empty() {
        return;
    }
    match line.as_str() {
        "/quit" | "/q" => app.quit = true,
        "/help" => {
            app.notice = Some("/quit  /help    esc clears    ctrl+c quits".into());
        }
        cmd if cmd.starts_with('/') => {
            app.notice = Some(format!("unknown command: {cmd}"));
        }
        _ => {
            app.notice = Some("no model configured".into());
        }
    }
}

fn draw(frame: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(0),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(frame.area());

    draw_header(frame, chunks[0]);
    draw_messages(frame, chunks[1], app);
    draw_editor(frame, chunks[2], app);
    draw_footer(frame, chunks[3]);
}

fn draw_header(frame: &mut Frame, area: Rect) {
    let version = env!("CARGO_PKG_VERSION");
    let header = Paragraph::new(vec![
        Line::from(vec![
            Span::styled("lunar", Style::default().fg(splash::GOLD)),
            Span::styled(format!("  {version}"), Style::default().fg(splash::DUST)),
        ]),
        Line::from(Span::styled(
            "no model configured",
            Style::default().fg(splash::ASH),
        )),
    ]);
    frame.render_widget(header, area);
}

fn draw_messages(frame: &mut Frame, area: Rect, app: &App) {
    let art = splash::lines();
    let art_h = art.len() as u16;
    let title = Line::from(Span::styled("lunar", Style::default().fg(splash::GOLD)));
    let hint = Line::from(Span::styled(
        "a coding harness",
        Style::default().fg(splash::ASH),
    ));
    let notice = app.notice.as_deref().unwrap_or("");
    let notice_line = Line::from(Span::styled(notice, Style::default().fg(splash::GOLD)));

    // art + blank + title + hint + optional notice
    let extra = 3 + u16::from(!notice.is_empty());
    let block_h = art_h.saturating_add(extra);
    let art_w = splash::width();

    let x = area.x + area.width.saturating_sub(art_w) / 2;
    let y = area.y + area.height.saturating_sub(block_h) / 2;
    let splash_area = Rect {
        x,
        y,
        width: art_w.min(area.width),
        height: block_h.min(area.height),
    };

    let mut lines = art;
    lines.push(Line::from(""));
    lines.push(title.centered());
    lines.push(hint.centered());
    if !notice.is_empty() {
        lines.push(notice_line.centered());
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

    let prompt = Paragraph::new(Line::from(vec![
        Span::styled("❯ ", Style::default().fg(splash::GOLD)),
        Span::styled(app.input.as_str(), Style::default().fg(splash::BONE)),
    ]));
    frame.render_widget(prompt, inner);

    let cursor_x = inner.x + 2 + app.input.chars().count() as u16;
    if cursor_x < inner.x + inner.width && inner.height > 0 {
        frame.set_cursor_position((cursor_x, inner.y));
    }
}

fn draw_footer(frame: &mut Frame, area: Rect) {
    let cwd = cwd_label();
    let left = Span::styled(cwd, Style::default().fg(splash::ASH));
    let right = Span::styled("no model", Style::default().fg(splash::DUST));
    let gap = area
        .width
        .saturating_sub((left.content.len() + right.content.len()) as u16);
    let line = Line::from(vec![left, Span::raw(" ".repeat(gap as usize)), right]);
    frame.render_widget(Paragraph::new(line), area);
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
