//! Empty-transcript mark. Not chrome — it disappears once there are messages.
//!
//! Homage to the Lua moon (disk + satellite). Wordmark is "lunar", not on the disk.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

/// Sunlight / the satellite.
pub const GOLD: Color = Color::Rgb(0xC4, 0xA5, 0x74);
/// Moon disk.
pub const BONE: Color = Color::Rgb(0xE8, 0xE2, 0xD2);
/// Stars.
pub const ASH: Color = Color::Rgb(0x8A, 0x86, 0x80);
/// Quiet labels.
pub const DUST: Color = Color::Rgb(0x5C, 0x58, 0x52);
/// User-turn strip (Pi dark theme).
pub const BAR: Color = Color::Rgb(0x34, 0x35, 0x41);
pub const USER: Color = Color::Rgb(0xD4, 0xD4, 0xD4);
/// Tool card, Pi green.
pub const TOOL_BG: Color = Color::Rgb(0x1C, 0x28, 0x20);
pub const TOOL_FG: Color = Color::Rgb(0x9A, 0xB8, 0xA0);
pub const TOOL_NAME: Color = Color::Rgb(0xC8, 0xE4, 0xCC);
/// Fenced code in assistant text.
pub const CODE_BG: Color = Color::Rgb(0x28, 0x26, 0x24);
pub const CODE_FG: Color = Color::Rgb(0xC8, 0xC2, 0xB4);

const RAW: &str = r#"
           +                  +
     +-          #########          -+
       +      ###############      +
            ###################
          #######################
         ####@@###################
        ####@    @#################
        ###@      @################
        ###@      @################
        ####@    @#################
         ####@@###################
          #######################         +
            ###################
       +      ###############      +
     -+          #########          +-
           +                  +
"#;

pub fn lines() -> Vec<Line<'static>> {
    RAW.lines()
        .filter(|l| !l.is_empty())
        .map(colorize)
        .collect()
}

pub fn width() -> u16 {
    RAW.lines()
        .map(|l| l.chars().count() as u16)
        .max()
        .unwrap_or(0)
}

fn colorize(line: &str) -> Line<'static> {
    let mut spans = Vec::new();
    let mut buf = String::new();
    let mut kind = Kind::Space;
    for ch in line.chars() {
        let next = Kind::of(ch);
        if next != kind && !buf.is_empty() {
            spans.push(kind.span(std::mem::take(&mut buf)));
        }
        kind = next;
        buf.push(ch);
    }
    if !buf.is_empty() {
        spans.push(kind.span(buf));
    }
    Line::from(spans)
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Space,
    Star,
    Moon,
    Satellite,
}

impl Kind {
    fn of(ch: char) -> Self {
        match ch {
            ' ' => Self::Space,
            '+' | '-' => Self::Star,
            '#' => Self::Moon,
            '@' => Self::Satellite,
            _ => Self::Moon,
        }
    }

    fn span(self, s: String) -> Span<'static> {
        let color = match self {
            Self::Space => return Span::raw(s),
            Self::Star => ASH,
            Self::Moon => BONE,
            Self::Satellite => GOLD,
        };
        Span::styled(s, Style::default().fg(color))
    }
}
