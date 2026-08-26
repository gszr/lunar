//! Ratatui view, transcript viewport, and editor layout.

use std::path::PathBuf;
use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::app::{App, Message, Mode, Role};
use crate::protocol::Config;
use crate::{commands, lua, mission, splash, transcript};

pub(crate) const EDITOR_MAX_LINES: u16 = 8;

pub(crate) fn editor_lines(input: &str, width: u16) -> Vec<String> {
    char_wrap(input, width.max(1) as usize)
}

pub(crate) fn char_wrap(text: &str, width: usize) -> Vec<String> {
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

pub(crate) fn cursor_xy(text: &str, cursor: usize, width: usize) -> (u16, u16) {
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

pub(crate) fn editor_height(input: &str, cursor: usize, width: u16) -> u16 {
    let lines = editor_lines(input, width).len() as u16;
    let (row, _) = cursor_xy(input, cursor, width.max(1) as usize);
    lines.max(row + 1).min(EDITOR_MAX_LINES) + 2
}

pub(crate) fn auth_editor_height(app: &App, width: u16) -> Option<u16> {
    app.auth_rx.as_ref()?;
    let text = auth_prompt_text(app);
    Some((char_wrap(&text, width.max(1) as usize).len() as u16).min(EDITOR_MAX_LINES) + 2)
}

pub(crate) fn auth_prompt_text(app: &App) -> String {
    let brand = app.auth_brand.unwrap_or("xAI");
    match &app.auth_prompt {
        Some(prompt) if prompt.browser_opened => format!(
            "Sign in to {brand}\nOpen: {}\nCode: {}\nWaiting for authorization…  Esc cancels",
            prompt.url, prompt.code
        ),
        Some(prompt) => format!(
            "Sign in to {brand}\nOpen: {}\nCode: {}\nCouldn’t open your browser. Copy and paste the URL above, then enter the displayed code.\nWaiting for authorization…  Esc cancels",
            prompt.url, prompt.code
        ),
        None => format!("Sign in to {brand}\nRequesting device code…  Esc cancels"),
    }
}

pub(crate) fn model_picker_height(app: &App) -> u16 {
    match &app.mode {
        Mode::Model { items, .. } => items.len().saturating_add(1).min(u16::MAX as usize) as u16,
        Mode::LoginProvider { .. } => 3,
        Mode::LoginMethod { .. } => 3,
        Mode::Thinking { .. } => 1,
        _ => 0,
    }
}

pub(crate) fn draw(frame: &mut Frame, app: &mut App) {
    let working = u16::from(app.cancel.is_some());
    let ed_h = auth_editor_height(app, frame.area().width).unwrap_or_else(|| {
        editor_height(app.input.as_str(), app.cursor, frame.area().width)
            .saturating_add(model_picker_height(app))
    });
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

pub(crate) const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub(crate) fn working_text(app: &App) -> &'static str {
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

pub(crate) fn draw_working(frame: &mut Frame, area: Rect, text: &'static str) {
    static ORIGIN: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let origin = ORIGIN.get_or_init(Instant::now);
    let frame_i = (origin.elapsed().as_millis() / 80) as usize % SPINNER.len();
    let line = Line::from(vec![
        Span::styled(SPINNER[frame_i], Style::default().fg(splash::GOLD)),
        Span::styled(text, Style::default().fg(splash::ASH)),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

pub(crate) fn draw_header(frame: &mut Frame, area: Rect, app: &App) {
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

pub(crate) fn draw_messages(frame: &mut Frame, area: Rect, app: &mut App) {
    if let Mode::Context { text, scroll } = &app.mode {
        draw_context(frame, area, text, *scroll);
        return;
    }
    if let Mode::Resume {
        items,
        cursor,
        title,
    } = &app.mode
    {
        draw_resume(frame, area, items, *cursor, title);
        return;
    }
    if app.messages.is_empty()
        && !app
            .notice
            .as_deref()
            .is_some_and(|notice| notice.contains('\n'))
    {
        draw_splash(frame, area, app);
        return;
    }
    let width = area.width.max(1) as usize;
    let lines = transcript::painted_lines(app, width);
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

pub(crate) fn context_lines(text: &str, width: usize) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(Span::styled(
        "context  page up/down · j/k · esc/q close",
        Style::default().fg(splash::ASH),
    ))];
    lines.extend(notice_lines(text, width));
    lines
}

fn draw_context(frame: &mut Frame, area: Rect, text: &str, scroll: usize) {
    let lines = context_lines(text, area.width.max(1) as usize);
    let max = lines.len().saturating_sub(area.height as usize);
    let start = scroll.min(max);
    let end = (start + area.height as usize).min(lines.len());
    frame.render_widget(Paragraph::new(lines[start..end].to_vec()), area);
}

pub(crate) fn notice_lines(notice: &str, width: usize) -> Vec<Line<'static>> {
    notice
        .lines()
        .flat_map(|line| char_wrap(line, width.max(1)))
        .map(|line| Line::from(Span::styled(line, Style::default().fg(splash::GOLD))))
        .collect()
}

pub(crate) fn draw_model(frame: &mut Frame, area: Rect, items: &[lua::ModelChoice], cursor: usize) {
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

pub(crate) fn draw_resume(
    frame: &mut Frame,
    area: Rect,
    items: &[mission::Meta],
    cursor: usize,
    title: &str,
) {
    let mut lines = vec![Line::from(Span::styled(
        format!("{title}  j/k  enter  esc"),
        Style::default().fg(splash::ASH),
    ))];
    let visible = area.height.saturating_sub(1) as usize;
    let start = if visible == 0 {
        0
    } else {
        cursor.saturating_add(1).saturating_sub(visible)
    };
    for (offset, item) in items.iter().skip(start).take(visible).enumerate() {
        let i = start + offset;
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

pub(crate) fn draw_splash(frame: &mut Frame, area: Rect, app: &App) {
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
    let notice_lines = notice_lines(notice, area.width.max(1) as usize);
    let extra = 4usize.saturating_add(notice_lines.len());
    let block_h = art_h.saturating_add(extra.min(u16::MAX as usize) as u16);
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
    lines.extend(notice_lines.into_iter().map(Line::centered));
    frame.render_widget(
        Paragraph::new(lines).alignment(Alignment::Left),
        splash_area,
    );
}

pub(crate) fn draw_complete(
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

pub(crate) fn draw_editor(frame: &mut Frame, area: Rect, app: &App) {
    if app.auth_rx.is_some() {
        draw_auth_editor(frame, area, app);
        return;
    }
    if let Mode::LoginProvider { cursor } = &app.mode {
        draw_picker_editor(
            frame,
            area,
            app,
            "login  j/k  enter  esc",
            &["xAI", "OpenAI"],
            *cursor,
        );
        return;
    }
    if let Mode::LoginMethod { cursor } = &app.mode {
        draw_picker_editor(
            frame,
            area,
            app,
            "xAI login  j/k  enter  esc",
            &["Use a subscription", "Use an API key"],
            *cursor,
        );
        return;
    }
    if let Mode::Thinking { cursor } = app.mode {
        draw_thinking_editor(frame, area, app, cursor);
        return;
    }
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

pub(crate) fn draw_thinking_editor(frame: &mut Frame, area: Rect, app: &App, cursor: usize) {
    let block = Block::default()
        .borders(Borders::TOP | Borders::BOTTOM)
        .border_style(Style::default().fg(splash::DUST));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inner);
    draw_editor_input(frame, chunks[0], app);
    let mut spans = vec![Span::styled("<-  ", Style::default().fg(splash::ASH))];
    for (i, level) in ["off", "low", "medium", "high"].iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("   "));
        }
        spans.push(Span::styled(
            *level,
            Style::default().fg(if i == cursor {
                splash::GOLD
            } else {
                splash::BONE
            }),
        ));
    }
    spans.push(Span::styled("  ->", Style::default().fg(splash::ASH)));
    frame.render_widget(Paragraph::new(Line::from(spans)), chunks[1]);
}

pub(crate) fn draw_auth_editor(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::TOP | Borders::BOTTOM)
        .border_style(Style::default().fg(splash::GOLD));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let lines: Vec<Line> = char_wrap(&auth_prompt_text(app), inner.width.max(1) as usize)
        .into_iter()
        .map(|line| Line::from(Span::styled(line, Style::default().fg(splash::BONE))))
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

pub(crate) fn draw_picker_editor(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    title: &str,
    items: &[&str],
    cursor: usize,
) {
    let block = Block::default()
        .borders(Borders::TOP | Borders::BOTTOM)
        .border_style(Style::default().fg(splash::DUST));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let picker_h = (items.len() + 1) as u16;
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(picker_h.min(inner.height)),
        ])
        .split(inner);
    draw_editor_input(frame, chunks[0], app);
    let mut lines = vec![Line::from(Span::styled(
        title.to_string(),
        Style::default().fg(splash::ASH),
    ))];
    for (i, item) in items.iter().enumerate() {
        let style = Style::default().fg(if i == cursor {
            splash::GOLD
        } else {
            splash::BONE
        });
        lines.push(Line::from(Span::styled(format!("  {item}"), style)));
    }
    frame.render_widget(Paragraph::new(lines), chunks[1]);
}

pub(crate) fn draw_editor_input(frame: &mut Frame, area: Rect, app: &App) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let shown = if matches!(app.mode, Mode::ApiKey) {
        "•".repeat(app.input.chars().count())
    } else {
        app.input.clone()
    };
    let mut lines = editor_lines(&shown, area.width);
    let shown_cursor = if matches!(app.mode, Mode::ApiKey) {
        shown.len()
    } else {
        app.cursor
    };
    let (mut row, col) = cursor_xy(&shown, shown_cursor, area.width as usize);
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

pub(crate) fn draw_footer(frame: &mut Frame, area: Rect, app: &App) {
    let cwd = Line::from(Span::styled(cwd_label(), Style::default().fg(splash::DUST)));
    let stats = stats_line(app);
    let model = match &app.config {
        Some(cfg) => format!(
            "({}) {} • {}",
            cfg.provider(),
            cfg.model,
            cfg.thinking.as_str()
        ),
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

pub(crate) fn stats_line(app: &App) -> String {
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

pub(crate) fn format_tokens(count: u32) -> String {
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

pub(crate) fn spread(
    mut left: Vec<Span<'static>>,
    right: Span<'static>,
    width: u16,
) -> Paragraph<'static> {
    let left_w: usize = left.iter().map(|s| s.content.chars().count()).sum();
    let right_w = right.content.chars().count();
    let gap = width.saturating_sub((left_w + right_w) as u16);
    left.push(Span::raw(" ".repeat(gap as usize)));
    left.push(right);
    Paragraph::new(Line::from(left))
}

pub(crate) fn cwd_label() -> String {
    std::env::current_dir()
        .ok()
        .map(shorten_home)
        .unwrap_or_else(|| ".".into())
}

pub(crate) fn shorten_home(path: PathBuf) -> String {
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
