//! Transcript paint. User bar, tool cards, terminal CommonMark.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::splash;

mod markdown;

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
    let plain = markdown::plain(text);
    let flat: String = plain.split_whitespace().collect::<Vec<_>>().join(" ");
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
        let cut = take_width(first, max);
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
    markdown::render(text, width)
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
    let pad = width.saturating_sub(display_width(text));
    Line::from(Span::styled(
        format!("{text}{}", " ".repeat(pad)),
        Style::default().fg(fg).bg(bg),
    ))
}

fn pad_spans(mut spans: Vec<Span<'static>>, width: usize, bg: Color) -> Line<'static> {
    let used: usize = spans.iter().map(|s| display_width(&s.content)).sum();
    let pad = width.saturating_sub(used);
    if pad > 0 {
        spans.push(Span::styled(" ".repeat(pad), Style::default().bg(bg)));
    }
    Line::from(spans)
}

fn display_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

fn take_width(text: &str, width: usize) -> String {
    text.chars()
        .scan(0, |used, ch| {
            let next = *used + unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
            (next <= width).then(|| {
                *used = next;
                ch
            })
        })
        .collect()
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
            } else if display_width(&line) + 1 + display_width(word) <= width {
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
    fn assistant_renders_aligned_tables() {
        let markdown = "| Name | Count |\n|:-----|------:|\n| 月 | 12 |";
        let lines = assistant(markdown, 40);
        let got: Vec<String> = lines.iter().map(line_text).collect();
        assert_eq!(
            got,
            vec![
                "┌──────┬───────┐",
                "│ Name │ Count │",
                "├──────┼───────┤",
                "│ 月   │    12 │",
                "└──────┴───────┘",
            ]
        );
        assert!(lines[1].spans.iter().any(|span| {
            span.style.fg == Some(splash::GOLD) && span.style.add_modifier.contains(Modifier::BOLD)
        }));
    }

    #[test]
    fn assistant_wraps_table_cells_to_fit() {
        let markdown = "| First | Second |\n|---|---|\n| alpha beta | gamma delta |";
        let lines = assistant(markdown, 17);
        assert!(
            lines
                .iter()
                .all(|line| display_width(&line_text(line)) <= 17)
        );
        let text = lines.iter().map(line_text).collect::<Vec<_>>().join("\n");
        assert!(text.contains("alph"), "{text}");
        assert!(text.contains("beta"));
        assert!(text.contains("gamma"));
        assert!(text.contains("delta"));
    }

    #[test]
    fn assistant_keeps_nested_inline_styles() {
        let lines = assistant("**bold *both* bold** plain", 80);
        let spans = &lines[0].spans;
        let both = spans.iter().find(|span| span.content == "both").unwrap();
        assert!(both.style.add_modifier.contains(Modifier::BOLD));
        assert!(both.style.add_modifier.contains(Modifier::ITALIC));
        let trailing_bold = spans
            .iter()
            .rev()
            .find(|span| span.content.contains("bold"))
            .unwrap();
        assert!(trailing_bold.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn assistant_wraps_wide_and_unbroken_text() {
        let lines = assistant("日本語 abcdefghij", 6);
        let got: Vec<String> = lines.iter().map(line_text).collect();
        assert_eq!(got, vec!["日本語", "abcdef", "ghij"]);
        assert!(got.iter().all(|line| display_width(line) <= 6));
    }

    #[test]
    fn assistant_renders_tasks_images_and_nested_quotes() {
        let markdown = "- [x] done\n\n> > ![moon](moon.png)";
        let lines = assistant(markdown, 80);
        let got: Vec<String> = lines.iter().map(line_text).collect();
        assert_eq!(got, vec!["• [x] done", "│ │ moon (moon.png)"]);
    }

    #[test]
    fn assistant_preserves_fenced_code_lines() {
        let lines = assistant("```\none\ntwo\n```", 20);
        let got: Vec<String> = lines.iter().map(line_text).collect();
        assert_eq!(got, vec!["one                 ", "two                 "]);
    }

    #[test]
    fn tool_card_fills_terminal_width_after_wide_text() {
        let lines = tool_card("bash", "✓ 日本語", 12);
        for line in lines {
            let width: usize = line
                .spans
                .iter()
                .map(|span| display_width(&span.content))
                .sum();
            assert_eq!(width, 12);
        }
    }

    #[test]
    fn tool_card_wraps_using_terminal_width() {
        let lines = tool_card("read", "日本語 ab", 7);
        let got: Vec<String> = lines.iter().map(line_text).collect();
        assert_eq!(got[1].trim_end(), "日本語");
        assert_eq!(got[2].trim_end(), "ab");
    }

    #[test]
    fn thinking_preview_keeps_short_text() {
        let lines = thinking_preview("hello world", 20);
        assert_eq!(lines.len(), 1);
        assert_eq!(line_text(&lines[0]), "hello world");
    }

    #[test]
    fn thinking_preview_strips_markdown_syntax() {
        let lines = thinking_preview("## Result\n**bold** and `code`", 40);
        assert_eq!(line_text(&lines[0]), "Result bold and code");
        assert!(
            lines[0].spans[0]
                .style
                .add_modifier
                .contains(Modifier::ITALIC)
        );
    }

    #[test]
    fn thinking_preview_shows_the_tail() {
        let text = "alpha beta gamma delta epsilon";
        let lines = thinking_preview(text, 5);
        let got: Vec<String> = lines.iter().map(line_text).collect();
        assert_eq!(got, vec!["...ga", "delta", "epsilon"]);
    }
}
