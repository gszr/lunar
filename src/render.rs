//! Transcript paint. User bar, tool cards, light markdown.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::splash;

const TOOL_PREVIEW: usize = 8;

pub fn user_bar(text: &str, width: usize) -> Vec<Line<'static>> {
    let inner = width.saturating_sub(2);
    let mut lines = vec![fill("", width, splash::USER, splash::BAR)];
    for s in wrap(text, inner) {
        lines.push(fill(&format!(" {s}"), width, splash::USER, splash::BAR));
    }
    if lines.len() == 1 {
        lines.push(fill(" ", width, splash::USER, splash::BAR));
    }
    lines.push(fill("", width, splash::USER, splash::BAR));
    lines
}

const THINK_LINES: usize = 3;

pub fn thinking_preview(text: &str, width: usize) -> Vec<Line<'static>> {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let wrapped = wrap(&flat, width);
    let truncated = wrapped.len() > THINK_LINES;
    let mut visible = if truncated {
        wrapped[wrapped.len() - THINK_LINES..].to_vec()
    } else {
        wrapped
    };
    if truncated && let Some(first) = visible.first_mut() {
        const ELLIP: &str = "...";
        let max = width.saturating_sub(ELLIP.len());
        let cut: String = first.chars().take(max).collect();
        *first = format!("{ELLIP}{cut}");
    }
    let style = Style::default()
        .fg(splash::ASH)
        .add_modifier(Modifier::ITALIC);
    visible
        .into_iter()
        .map(|s| Line::from(Span::styled(s, style)))
        .collect()
}

pub fn assistant(text: &str, width: usize) -> Vec<Line<'static>> {
    Markdown::new(width).render(text)
}

struct Markdown {
    width: usize,
    lines: Vec<Line<'static>>,
    spans: Vec<Span<'static>>,
    style: Style,
    quote_depth: usize,
    list: Vec<Option<u64>>,
    link: Vec<String>,
    code: bool,
}

impl Markdown {
    fn new(width: usize) -> Self {
        Self {
            width,
            lines: Vec::new(),
            spans: Vec::new(),
            style: Style::default().fg(splash::BONE),
            quote_depth: 0,
            list: Vec::new(),
            link: Vec::new(),
            code: false,
        }
    }

    fn render(mut self, text: &str) -> Vec<Line<'static>> {
        let parser = Parser::new_ext(text, Options::ENABLE_STRIKETHROUGH);
        for event in parser {
            match event {
                Event::Start(tag) => self.start(tag),
                Event::End(tag) => self.end(tag),
                Event::Text(text) => self.text(&text),
                Event::Code(text) => self.spans.push(Span::styled(
                    text.into_string(),
                    self.style.fg(splash::CODE_FG).bg(splash::CODE_BG),
                )),
                Event::SoftBreak => self.text(" "),
                Event::HardBreak => self.flush(),
                Event::Rule => {
                    self.flush();
                    self.text(&"─".repeat(self.width));
                    self.flush();
                }
                _ => {}
            }
        }
        self.flush();
        self.lines
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Heading { .. } => self.style = self.style.fg(splash::GOLD).bold(),
            Tag::Emphasis => self.style = self.style.italic(),
            Tag::Strong => self.style = self.style.bold(),
            Tag::Strikethrough => self.style = self.style.crossed_out(),
            Tag::BlockQuote(_) => self.quote_depth += 1,
            Tag::List(start) => self.list.push(start),
            Tag::Item => {
                self.flush();
                let marker = match self.list.last_mut() {
                    Some(Some(n)) => {
                        let marker = format!("{n}. ");
                        *n += 1;
                        marker
                    }
                    _ => "• ".into(),
                };
                self.text(&format!(
                    "{}{}",
                    "  ".repeat(self.list.len().saturating_sub(1)),
                    marker
                ));
            }
            Tag::CodeBlock(_) => {
                self.flush();
                self.code = true;
            }
            Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. } => {
                self.link.push(dest_url.into_string())
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Heading(_) => {
                self.flush();
                self.style = Style::default().fg(splash::BONE);
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                self.style = Style::default().fg(splash::BONE)
            }
            TagEnd::Paragraph | TagEnd::Item => self.flush(),
            TagEnd::BlockQuote(_) => {
                self.flush();
                self.quote_depth = self.quote_depth.saturating_sub(1);
            }
            TagEnd::List(_) => {
                self.flush();
                self.list.pop();
            }
            TagEnd::CodeBlock => {
                self.flush();
                self.code = false;
            }
            TagEnd::Link | TagEnd::Image => {
                if let Some(url) = self.link.pop() {
                    self.spans.push(Span::styled(
                        format!(" ({url})"),
                        Style::default().fg(splash::ASH),
                    ));
                }
            }
            _ => {}
        }
    }

    fn text(&mut self, text: &str) {
        let style = if self.code {
            self.style.fg(splash::CODE_FG).bg(splash::CODE_BG)
        } else if !self.link.is_empty() {
            self.style.fg(splash::GOLD).underlined()
        } else {
            self.style
        };
        self.spans.push(Span::styled(text.to_string(), style));
    }

    fn flush(&mut self) {
        if self.spans.is_empty() {
            return;
        }
        let prefix = if self.quote_depth > 0 { "│ " } else { "" };
        let mut pending = vec![Span::styled(
            prefix.to_string(),
            Style::default().fg(splash::ASH),
        )];
        let mut used = prefix.chars().count();
        for span in std::mem::take(&mut self.spans) {
            for word in span.content.split_inclusive(' ') {
                let len = word.chars().count();
                if used > prefix.len() && used + len > self.width {
                    self.push_line(std::mem::take(&mut pending));
                    pending.push(Span::styled(
                        prefix.to_string(),
                        Style::default().fg(splash::ASH),
                    ));
                    used = prefix.chars().count();
                }
                pending.push(Span::styled(word.to_string(), span.style));
                used += len;
            }
        }
        if pending.len() > 1 {
            self.push_line(pending);
        }
    }

    fn push_line(&mut self, mut spans: Vec<Span<'static>>) {
        if self.code {
            let used: usize = spans.iter().map(|span| span.content.chars().count()).sum();
            spans.push(Span::styled(
                " ".repeat(self.width.saturating_sub(used)),
                Style::default().bg(splash::CODE_BG),
            ));
        }
        self.lines.push(Line::from(spans));
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn line_text(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn assistant_renders_common_mark() {
        let lines = assistant("# Gold\n\n**bold** and `code`\n\n- one\n- two", 40);
        let got: Vec<String> = lines.iter().map(line_text).collect();
        assert_eq!(got, vec!["Gold", "bold and code", "• one", "• two"]);
        assert!(
            lines[0].spans[1]
                .style
                .add_modifier
                .contains(Modifier::BOLD)
        );
        assert!(
            lines[1]
                .spans
                .iter()
                .any(|span| span.style.bg == Some(splash::CODE_BG))
        );
    }

    #[test]
    fn assistant_shows_link_destination() {
        let lines = assistant("read [the docs](https://example.com)", 80);
        assert_eq!(line_text(&lines[0]), "read the docs (https://example.com)");
    }

    #[test]
    fn thinking_preview_keeps_short_text() {
        let lines = thinking_preview("hello world", 20);
        assert_eq!(lines.len(), 1);
        assert_eq!(line_text(&lines[0]), "hello world");
    }

    #[test]
    fn thinking_preview_shows_the_tail() {
        let text = "alpha beta gamma delta epsilon";
        let lines = thinking_preview(text, 5);
        let got: Vec<String> = lines.iter().map(line_text).collect();
        assert_eq!(got, vec!["...ga", "delta", "epsilon"]);
    }
}
