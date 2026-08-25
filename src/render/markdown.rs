//! Terminal CommonMark rendering.

use pulldown_cmark::{Alignment, CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthChar;

use super::display_width;
use crate::splash;

pub(super) fn render(text: &str, width: usize) -> Vec<Line<'static>> {
    Markdown::new(width).render(text)
}

fn options() -> Options {
    Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS
}

pub(super) fn plain(text: &str) -> String {
    let mut plain = String::new();
    for event in Parser::new_ext(text, options()) {
        match event {
            Event::Text(text) | Event::Code(text) => plain.push_str(&text),
            Event::TaskListMarker(done) => plain.push_str(if done { "[x] " } else { "[ ] " }),
            Event::SoftBreak | Event::HardBreak => plain.push(' '),
            Event::End(TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::TableCell) => {
                plain.push(' ')
            }
            Event::Rule => plain.push_str(" — "),
            _ => {}
        }
    }
    plain
}

struct Markdown {
    width: usize,
    lines: Vec<Line<'static>>,
    spans: Vec<Span<'static>>,
    heading: bool,
    emphasis: usize,
    strong: usize,
    strike: usize,
    quote_depth: usize,
    list: Vec<Option<u64>>,
    link: Vec<String>,
    image_depth: usize,
    code: bool,
    json: bool,
    table: Option<Table>,
}

struct Table {
    alignments: Vec<Alignment>,
    rows: Vec<Vec<Vec<Span<'static>>>>,
    row: Vec<Vec<Span<'static>>>,
}

impl Markdown {
    fn new(width: usize) -> Self {
        Self {
            width,
            lines: Vec::new(),
            spans: Vec::new(),
            heading: false,
            emphasis: 0,
            strong: 0,
            strike: 0,
            quote_depth: 0,
            list: Vec::new(),
            link: Vec::new(),
            image_depth: 0,
            code: false,
            json: false,
            table: None,
        }
    }

    fn render(mut self, text: &str) -> Vec<Line<'static>> {
        for event in Parser::new_ext(text, options()) {
            match event {
                Event::Start(tag) => self.start(tag),
                Event::End(tag) => self.end(tag),
                Event::Text(text) if self.code => {
                    if self.json {
                        self.json_text(&text);
                    } else {
                        self.code_text(&text);
                    }
                }
                Event::Text(text) => self.text(&text),
                Event::Code(text) => {
                    let style = self.style().fg(splash::CODE_FG).bg(splash::CODE_BG);
                    self.spans.push(Span::styled(text.into_string(), style));
                }
                Event::TaskListMarker(done) => self.text(if done { "[x] " } else { "[ ] " }),
                Event::SoftBreak => self.text(" "),
                Event::HardBreak => self.flush(),
                Event::Rule => {
                    self.flush();
                    self.text(&"─".repeat(self.width));
                    self.flush();
                }
                Event::Html(_) | Event::InlineHtml(_) => {}
                Event::FootnoteReference(label) => self.text(&format!("[{label}]")),
                Event::InlineMath(text) | Event::DisplayMath(text) => self.text(&text),
            }
        }
        self.flush();
        self.lines
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Heading { .. } => self.heading = true,
            Tag::Emphasis => self.emphasis += 1,
            Tag::Strong => self.strong += 1,
            Tag::Strikethrough => self.strike += 1,
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
            Tag::CodeBlock(kind) => {
                self.flush();
                self.code = true;
                self.json =
                    matches!(kind, CodeBlockKind::Fenced(language) if language.as_ref() == "json");
            }
            Tag::Link { dest_url, .. } => self.link.push(dest_url.into_string()),
            Tag::Image { dest_url, .. } => {
                self.link.push(dest_url.into_string());
                self.image_depth += 1;
            }
            Tag::Table(alignments) => {
                self.flush();
                self.table = Some(Table {
                    alignments,
                    rows: Vec::new(),
                    row: Vec::new(),
                });
            }
            Tag::TableRow => {
                if let Some(table) = &mut self.table {
                    table.row.clear();
                }
            }
            Tag::TableCell => self.spans.clear(),
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Heading(_) => {
                self.flush();
                self.heading = false;
            }
            TagEnd::Emphasis => self.emphasis = self.emphasis.saturating_sub(1),
            TagEnd::Strong => self.strong = self.strong.saturating_sub(1),
            TagEnd::Strikethrough => self.strike = self.strike.saturating_sub(1),
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
                self.json = false;
            }
            TagEnd::Link | TagEnd::Image => {
                if let Some(url) = self.link.pop() {
                    self.spans.push(Span::styled(
                        format!(" ({url})"),
                        Style::default().fg(splash::ASH),
                    ));
                }
                if matches!(tag, TagEnd::Image) {
                    self.image_depth = self.image_depth.saturating_sub(1);
                }
            }
            TagEnd::TableCell => {
                let cell = std::mem::take(&mut self.spans);
                if let Some(table) = &mut self.table {
                    table.row.push(cell);
                }
            }
            TagEnd::TableRow | TagEnd::TableHead => {
                if let Some(table) = &mut self.table
                    && !table.row.is_empty()
                {
                    table.rows.push(std::mem::take(&mut table.row));
                }
            }
            TagEnd::Table => {
                if let Some(table) = self.table.take() {
                    self.render_table(table);
                }
            }
            _ => {}
        }
    }

    fn style(&self) -> Style {
        let mut style = Style::default().fg(if self.heading {
            splash::GOLD
        } else {
            splash::BONE
        });
        if self.heading || self.strong > 0 {
            style = style.bold();
        }
        if self.emphasis > 0 {
            style = style.italic();
        }
        if self.strike > 0 {
            style = style.crossed_out();
        }
        if !self.link.is_empty() && self.image_depth == 0 {
            style = style.fg(splash::GOLD).underlined();
        }
        style
    }

    fn json_text(&mut self, text: &str) {
        let formatted = serde_json::from_str::<serde_json::Value>(text)
            .and_then(|value| serde_json::to_string_pretty(&value))
            .unwrap_or_else(|_| text.to_string());
        self.code_text(&formatted);
    }

    fn code_text(&mut self, text: &str) {
        let style = self.style().fg(splash::CODE_FG).bg(splash::CODE_BG);
        for line in text.lines() {
            if line.is_empty() {
                self.push_line(Vec::new());
                continue;
            }
            let mut rest = line;
            while !rest.is_empty() {
                let (part, tail) = split_width(rest, self.width.max(1));
                self.push_line(vec![Span::styled(part.to_string(), style)]);
                rest = tail;
            }
        }
    }

    fn text(&mut self, text: &str) {
        let style = if self.code {
            self.style().fg(splash::CODE_FG).bg(splash::CODE_BG)
        } else {
            self.style()
        };
        self.spans.push(Span::styled(text.to_string(), style));
    }

    fn flush(&mut self) {
        if self.spans.is_empty() || self.table.is_some() {
            return;
        }
        let prefix = "│ ".repeat(self.quote_depth);
        let width = self.width.saturating_sub(display_width(&prefix)).max(1);
        let wrapped = wrap_spans(std::mem::take(&mut self.spans), width);
        for mut spans in wrapped {
            spans.insert(
                0,
                Span::styled(prefix.clone(), Style::default().fg(splash::ASH)),
            );
            self.push_line(spans);
        }
    }

    fn push_line(&mut self, mut spans: Vec<Span<'static>>) {
        if self.code {
            let used: usize = spans.iter().map(|span| display_width(&span.content)).sum();
            spans.push(Span::styled(
                " ".repeat(self.width.saturating_sub(used)),
                Style::default().bg(splash::CODE_BG),
            ));
        }
        self.lines.push(Line::from(spans));
    }

    fn render_table(&mut self, table: Table) {
        if table.rows.is_empty() {
            return;
        }
        let columns = table
            .rows
            .iter()
            .map(Vec::len)
            .max()
            .unwrap_or(0)
            .max(table.alignments.len());
        if columns == 0 {
            return;
        }
        let mut natural = vec![1; columns];
        for row in &table.rows {
            for (column, cell) in row.iter().enumerate() {
                natural[column] = natural[column].max(spans_width(cell));
            }
        }
        let widths = table_widths(&natural, self.width);
        let row_count = table.rows.len();
        self.table_rule('┌', '┬', '┐', &widths);
        for (row_index, row) in table.rows.into_iter().enumerate() {
            let cells: Vec<Vec<Vec<Span<'static>>>> = (0..columns)
                .map(|column| {
                    let spans = row.get(column).cloned().unwrap_or_default();
                    wrap_spans(spans, widths[column]).into_iter().collect()
                })
                .collect();
            let height = cells.iter().map(Vec::len).max().unwrap_or(1);
            for line_index in 0..height {
                let mut line = vec![table_border("│")];
                for column in 0..columns {
                    line.push(Span::raw(" "));
                    let mut content = cells[column].get(line_index).cloned().unwrap_or_default();
                    if row_index == 0 {
                        for span in &mut content {
                            span.style = span.style.fg(splash::GOLD).bold();
                        }
                    }
                    let used = spans_width(&content);
                    let gap = widths[column].saturating_sub(used);
                    let alignment = table
                        .alignments
                        .get(column)
                        .copied()
                        .unwrap_or(Alignment::None);
                    let (left, right) = match alignment {
                        Alignment::Right => (gap, 0),
                        Alignment::Center => (gap / 2, gap - gap / 2),
                        Alignment::None | Alignment::Left => (0, gap),
                    };
                    line.push(Span::raw(" ".repeat(left)));
                    line.append(&mut content);
                    line.push(Span::raw(" ".repeat(right + 1)));
                    line.push(table_border("│"));
                }
                self.lines.push(Line::from(line));
            }
            if row_index == 0 && row_count > 1 {
                self.table_rule('├', '┼', '┤', &widths);
            }
        }
        self.table_rule('└', '┴', '┘', &widths);
    }

    fn table_rule(&mut self, left: char, join: char, right: char, widths: &[usize]) {
        let mut rule = left.to_string();
        for (index, width) in widths.iter().enumerate() {
            rule.push_str(&"─".repeat(width + 2));
            rule.push(if index + 1 == widths.len() {
                right
            } else {
                join
            });
        }
        self.lines.push(Line::from(table_border(rule)));
    }
}

fn table_border(text: impl Into<String>) -> Span<'static> {
    Span::styled(text.into(), Style::default().fg(splash::DUST))
}

fn table_widths(natural: &[usize], width: usize) -> Vec<usize> {
    let chrome = natural.len() * 3 + 1;
    let available = width.saturating_sub(chrome).max(natural.len());
    let total: usize = natural.iter().sum();
    if total <= available {
        return natural.to_vec();
    }
    let mut widths: Vec<usize> = natural
        .iter()
        .map(|value| ((*value * available) / total).max(1))
        .collect();
    while widths.iter().sum::<usize>() > available {
        if let Some((index, _)) = widths
            .iter()
            .enumerate()
            .filter(|(_, w)| **w > 1)
            .max_by_key(|(_, w)| **w)
        {
            widths[index] -= 1;
        } else {
            break;
        }
    }
    while widths.iter().sum::<usize>() < available {
        let index = widths
            .iter()
            .enumerate()
            .max_by_key(|(index, value)| natural[*index].saturating_sub(**value))
            .map(|(index, _)| index)
            .unwrap_or(0);
        widths[index] += 1;
    }
    widths
}

fn spans_width(spans: &[Span<'_>]) -> usize {
    spans.iter().map(|span| display_width(&span.content)).sum()
}

fn wrap_spans(spans: Vec<Span<'static>>, width: usize) -> Vec<Vec<Span<'static>>> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut line = Vec::new();
    let mut used = 0;
    for span in spans {
        let mut token = String::new();
        let mut whitespace = None;
        for ch in span.content.chars().chain(std::iter::once('\0')) {
            let next_whitespace = ch.is_whitespace();
            if whitespace.is_some() && (ch == '\0' || whitespace != Some(next_whitespace)) {
                push_token(
                    &mut lines,
                    &mut line,
                    &mut used,
                    &token,
                    span.style,
                    width,
                    whitespace == Some(true),
                );
                token.clear();
            }
            if ch != '\0' {
                token.push(ch);
                whitespace = Some(next_whitespace);
            }
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    if lines.is_empty() {
        lines.push(Vec::new());
    }
    lines
}

fn push_token(
    lines: &mut Vec<Vec<Span<'static>>>,
    line: &mut Vec<Span<'static>>,
    used: &mut usize,
    token: &str,
    style: Style,
    width: usize,
    whitespace: bool,
) {
    let token = if whitespace { " " } else { token };
    let token_width = display_width(token);
    if whitespace {
        if *used == 0 {
            return;
        }
        if *used + token_width > width {
            lines.push(std::mem::take(line));
            *used = 0;
            return;
        }
        line.push(Span::styled(token.to_string(), style));
        *used += token_width;
        return;
    }
    if *used > 0 && *used + token_width > width {
        lines.push(std::mem::take(line));
        *used = 0;
    }
    let mut rest = token;
    while !rest.is_empty() {
        let room = width.saturating_sub(*used);
        let (part, tail) = split_width(rest, room);
        if part.is_empty() {
            lines.push(std::mem::take(line));
            *used = 0;
            continue;
        }
        line.push(Span::styled(part.to_string(), style));
        *used += display_width(part);
        rest = tail;
        if !rest.is_empty() {
            lines.push(std::mem::take(line));
            *used = 0;
        }
    }
}

fn split_width(text: &str, width: usize) -> (&str, &str) {
    let mut end = 0;
    let mut used = 0;
    for (index, ch) in text.char_indices() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + ch_width > width {
            break;
        }
        used += ch_width;
        end = index + ch.len_utf8();
    }
    text.split_at(end)
}
