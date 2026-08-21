//! Transcript paint cache and viewport scrolling.

use ratatui::crossterm::event::{MouseEvent, MouseEventKind};
use ratatui::text::Line;

use crate::app::{App, Message, Mode, Role};
use crate::render;
use crate::view::notice_lines;

pub(crate) fn invalidate_paint(app: &mut App) {
    app.paint_frozen.clear();
    app.paint_upto = 0;
    app.paint_prev_tool = false;
}

pub(crate) fn freeze_end(app: &App) -> usize {
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

pub(crate) fn painted_lines(app: &mut App, width: usize) -> Vec<Line<'static>> {
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

pub(crate) fn paint_slice(
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

pub(crate) const WHEEL_LINES: usize = 3;

pub(crate) fn page_delta(app: &App) -> isize {
    app.transcript_h.saturating_sub(1).max(1) as isize
}

pub(crate) fn jump_to_tail(app: &mut App) {
    app.follow = true;
}

pub(crate) fn scroll_home(app: &mut App) {
    if !matches!(app.mode, Mode::Chat) || app.messages.is_empty() {
        return;
    }
    app.scroll = 0;
    app.follow = false;
}

pub(crate) fn scroll_by(app: &mut App, delta: isize) {
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

pub(crate) fn on_mouse(app: &mut App, mouse: MouseEvent) {
    match mouse.kind {
        MouseEventKind::ScrollUp => scroll_by(app, -(WHEEL_LINES as isize)),
        MouseEventKind::ScrollDown => scroll_by(app, WHEEL_LINES as isize),
        _ => {}
    }
}
