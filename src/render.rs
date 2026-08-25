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
    let title = sanitize_terminal_text(title);
    let body = sanitize_terminal_text(body);
    let (name, rest) = title.split_once(' ').unwrap_or((&title, ""));
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

fn sanitize_terminal_text(text: &str) -> String {
    let mut clean = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\u{1b}' => match chars.next() {
                Some('[') => skip_csi(&mut chars),
                Some(']') | Some('P') | Some('X') | Some('^') | Some('_') => {
                    skip_string_escape(&mut chars)
                }
                Some(_) | None => {}
            },
            '\u{9b}' => skip_csi(&mut chars),
            '\u{90}' | '\u{98}' | '\u{9d}' | '\u{9e}' | '\u{9f}' => skip_string_escape(&mut chars),
            // GitHub logs and similar output sometimes escape ESC as the literal
            // characters "^[[" instead of preserving the control byte.
            '^' if chars.peek() == Some(&'[') => {
                chars.next();
                if chars.peek() == Some(&'[') {
                    chars.next();
                    skip_csi(&mut chars);
                } else {
                    clean.push('^');
                    clean.push('[');
                }
            }
            '\u{feff}' => {}
            '\n' => clean.push('\n'),
            '\t' => clean.push_str("    "),
            ch if ch.is_control() => {}
            _ => clean.push(ch),
        }
    }
    clean
}

fn skip_csi(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    for ch in chars.by_ref() {
        if ('@'..='~').contains(&ch) {
            break;
        }
    }
}

fn skip_string_escape(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    let mut esc = false;
    for ch in chars.by_ref() {
        if ch == '\u{7}' || (esc && ch == '\\') {
            break;
        }
        esc = ch == '\u{1b}';
    }
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
    fn assistant_formats_fenced_json() {
        let lines = assistant(
            "```json\n{\"data\":[{\"id\":\"model-name\",\"owned_by\":\"provider\"}]}\n```",
            80,
        );
        let got: Vec<String> = lines
            .iter()
            .map(line_text)
            .map(|line| line.trim_end().to_string())
            .collect();
        assert_eq!(
            got,
            vec![
                "{",
                "  \"data\": [",
                "    {",
                "      \"id\": \"model-name\",",
                "      \"owned_by\": \"provider\"",
                "    }",
                "  ]",
                "}",
            ]
        );
    }

    #[test]
    fn assistant_preserves_invalid_fenced_json() {
        let lines = assistant("```json\n{not json}\n```", 20);
        assert_eq!(line_text(&lines[0]).trim_end(), "{not json}");
    }

    #[test]
    fn assistant_does_not_format_unlabelled_json() {
        let lines = assistant("```\n{\"a\":1}\n```", 20);
        assert_eq!(line_text(&lines[0]).trim_end(), "{\"a\":1}");
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
    fn tool_card_sanitizes_terminal_sequences_before_painting() {
        let lines = tool_card(
            "bash printf color",
            "\u{feff}\u{1b}[36;1mcolored\u{1b}[0m plain\n^[[36;1mliteral color^[[0m\n\u{9b}31mC1 color\u{9b}0m\n\u{1b}]8;;https://example.com\u{7}link\u{1b}]8;;\u{7}",
            20,
        );
        let got: Vec<String> = lines.iter().map(line_text).collect();
        assert_eq!(got[1].trim_end(), "colored plain");
        assert_eq!(got[2].trim_end(), "literal color");
        assert_eq!(got[3].trim_end(), "C1 color");
        assert_eq!(got[4].trim_end(), "link");
        assert!(got.iter().all(|line| !line.contains('\u{1b}')));
        assert!(got.iter().all(|line| !line.contains("^[[")));
        assert!(got.iter().all(|line| !line.contains('\u{feff}')));
        assert!(
            lines
                .iter()
                .flat_map(|line| &line.spans)
                .all(|span| { span.style.bg == Some(splash::TOOL_BG) })
        );
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
