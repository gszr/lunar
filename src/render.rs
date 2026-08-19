//! Transcript paint. User bar, tool cards, light markdown.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::splash;

const TOOL_PREVIEW: usize = 8;

pub fn user_bar(text: &str, width: usize) -> Vec<Line<'static>> {
    wrap(text, width)
        .into_iter()
        .map(|s| fill(&s, width, splash::BONE, splash::BAR))
        .collect()
}

const THINK_LINES: usize = 3;

pub fn thinking_preview(text: &str, width: usize) -> Vec<Line<'static>> {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut wrapped = wrap(&flat, width);
    if wrapped.len() > THINK_LINES {
        wrapped.truncate(THINK_LINES);
        if let Some(last) = wrapped.last_mut() {
            const ELLIP: &str = "...";
            let max = width.saturating_sub(ELLIP.len());
            let cut: String = last.chars().take(max).collect();
            *last = format!("{cut}{ELLIP}");
        }
    }
    let style = Style::default()
        .fg(splash::ASH)
        .add_modifier(Modifier::ITALIC);
    wrapped
        .into_iter()
        .map(|s| Line::from(Span::styled(s, style)))
        .collect()
}

pub fn assistant(text: &str, width: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut fence: Option<Vec<&str>> = None;
    for raw in text.split('\n') {
        if raw.trim_start().starts_with("```") {
            match fence.take() {
                Some(block) => lines.extend(code_block(&block, width)),
                None => fence = Some(Vec::new()),
            }
            continue;
        }
        if let Some(block) = fence.as_mut() {
            block.push(raw);
            continue;
        }
        if let Some(rest) = raw.strip_prefix('#') {
            let heading = rest.trim_start_matches('#').trim();
            for s in wrap(heading, width) {
                lines.push(Line::from(Span::styled(
                    s,
                    Style::default().fg(splash::GOLD),
                )));
            }
        } else {
            for s in wrap(raw, width) {
                lines.push(Line::from(Span::styled(
                    s,
                    Style::default().fg(splash::BONE),
                )));
            }
        }
    }
    if let Some(block) = fence {
        lines.extend(code_block(&block, width));
    }
    lines
}

pub fn tool_card(title: &str, body: &str, width: usize) -> Vec<Line<'static>> {
    let (name, rest) = title.split_once(' ').unwrap_or((title, ""));
    let mut lines = Vec::new();
    lines.push(title_line(name, rest, width));

    let source: Vec<&str> = body.lines().collect();
    let preview = source.len().min(TOOL_PREVIEW);
    for line in &source[..preview] {
        for s in wrap(line, width) {
            lines.push(fill(&s, width, splash::TOOL_FG, splash::TOOL_BG));
        }
    }
    if source.len() > TOOL_PREVIEW {
        let more = format!("… {} more lines", source.len() - TOOL_PREVIEW);
        lines.push(fill(&more, width, splash::ASH, splash::TOOL_BG));
    }
    lines
}

fn title_line(name: &str, rest: &str, width: usize) -> Line<'static> {
    let mut spans = vec![Span::styled(
        name.to_string(),
        Style::default().fg(splash::TOOL_NAME),
    )];
    if !rest.is_empty() {
        spans.push(Span::styled(
            format!(" {rest}"),
            Style::default().fg(splash::ASH).bg(splash::TOOL_BG),
        ));
    }
    spans[0].style = spans[0].style.bg(splash::TOOL_BG);
    pad_spans(spans, width, splash::TOOL_BG)
}

fn code_block(block: &[&str], width: usize) -> Vec<Line<'static>> {
    if block.is_empty() {
        return vec![fill("", width, splash::CODE_FG, splash::CODE_BG)];
    }
    block
        .iter()
        .flat_map(|line| wrap(line, width))
        .map(|s| fill(&s, width, splash::CODE_FG, splash::CODE_BG))
        .collect()
}

fn fill(text: &str, width: usize, fg: Color, bg: Color) -> Line<'static> {
    let pad = width.saturating_sub(text.chars().count());
    Line::from(Span::styled(
        format!("{text}{}", " ".repeat(pad)),
        Style::default().fg(fg).bg(bg),
    ))
}

fn pad_spans(mut spans: Vec<Span<'static>>, width: usize, bg: Color) -> Line<'static> {
    let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    let pad = width.saturating_sub(used);
    if pad > 0 {
        spans.push(Span::styled(" ".repeat(pad), Style::default().bg(bg)));
    }
    Line::from(spans)
}

pub fn wrap(text: &str, width: usize) -> Vec<String> {
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
