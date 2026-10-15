//! Terminal setup and restoration.

use std::fmt;
use std::io;

use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use ratatui::crossterm::{Command, execute};

pub struct Terminal {
    inner: DefaultTerminal,
}

struct UseNormalMouseTracking;

impl Command for UseNormalMouseTracking {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        // Crossterm enables button- and all-motion modes, but Lunar only uses wheel events.
        f.write_str("\x1b[?1003l\x1b[?1002l\x1b[?1000h")
    }

    #[cfg(windows)]
    fn execute_winapi(&self) -> io::Result<()> {
        Ok(())
    }
}

impl Terminal {
    pub fn init() -> Self {
        let inner = ratatui::init();
        enable_features();
        install_panic_hook();
        Self { inner }
    }

    pub fn get_mut(&mut self) -> &mut DefaultTerminal {
        &mut self.inner
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        restore();
    }
}

fn install_panic_hook() {
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        hook(info);
    }));
}

pub fn suspend() {
    restore();
}

pub fn resume() -> DefaultTerminal {
    let terminal = ratatui::init();
    enable_features();
    terminal
}

fn enable_features() {
    let _ = execute!(
        io::stdout(),
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES),
        EnableBracketedPaste,
        EnableMouseCapture,
        UseNormalMouseTracking,
    );
}

fn restore() {
    let _ = execute!(
        io::stdout(),
        PopKeyboardEnhancementFlags,
        DisableBracketedPaste,
        DisableMouseCapture,
    );
    ratatui::restore();
}
