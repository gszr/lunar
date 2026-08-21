//! Terminal setup and restoration.

use std::io;

use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use ratatui::crossterm::execute;

pub struct Terminal {
    inner: DefaultTerminal,
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
