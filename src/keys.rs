//! Default command shortcuts. Keep bindings separate from input routing and commands.

use crate::commands;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

struct Binding {
    key: char,
    modifiers: KeyModifiers,
    command: &'static str,
}

#[cfg(target_os = "macos")]
const COMMAND_MODIFIER: KeyModifiers = KeyModifiers::SUPER;
#[cfg(not(target_os = "macos"))]
const COMMAND_MODIFIER: KeyModifiers = KeyModifiers::ALT;
const COMMAND_SHORTCUT: KeyModifiers = COMMAND_MODIFIER.union(KeyModifiers::SHIFT);
const BINDINGS: &[Binding] = &[
    Binding {
        key: 'm',
        modifiers: COMMAND_SHORTCUT,
        command: commands::MODEL,
    },
    Binding {
        key: 't',
        modifiers: COMMAND_SHORTCUT,
        command: commands::THINKING,
    },
    Binding {
        key: 'o',
        modifiers: COMMAND_SHORTCUT,
        command: commands::CONFIG,
    },
    Binding {
        key: 'c',
        modifiers: COMMAND_SHORTCUT,
        command: commands::CONTEXT,
    },
    Binding {
        key: 'd',
        modifiers: KeyModifiers::CONTROL,
        command: commands::QUIT,
    },
];

pub(crate) fn command(key: KeyEvent) -> Option<&'static str> {
    let KeyCode::Char(c) = key.code else {
        return None;
    };
    BINDINGS
        .iter()
        .find(|binding| binding.key == c.to_ascii_lowercase() && binding.modifiers == key.modifiers)
        .map(|binding| binding.command)
}

pub(crate) fn help() -> String {
    BINDINGS
        .iter()
        .map(|binding| {
            let modifiers = [
                (KeyModifiers::CONTROL, "ctrl"),
                (KeyModifiers::SUPER, "cmd"),
                (KeyModifiers::ALT, "alt"),
                (KeyModifiers::SHIFT, "shift"),
            ]
            .into_iter()
            .filter_map(|(modifier, label)| binding.modifiers.contains(modifier).then_some(label))
            .collect::<Vec<_>>()
            .join("+");
            format!("{modifiers}+{} {}", binding.key, binding.command)
        })
        .collect::<Vec<_>>()
        .join("    ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_match_lowercase_and_uppercase_terminal_events() {
        for binding in BINDINGS {
            for c in [binding.key, binding.key.to_ascii_uppercase()] {
                assert_eq!(
                    command(KeyEvent::new(KeyCode::Char(c), binding.modifiers)),
                    Some(binding.command)
                );
            }
        }
    }

    #[test]
    fn shortcuts_use_the_platform_modifier_and_help_labels() {
        let (modifier, label) = if cfg!(target_os = "macos") {
            (KeyModifiers::SUPER, "cmd")
        } else {
            (KeyModifiers::ALT, "alt")
        };
        for (key, expected) in [
            ('m', commands::MODEL),
            ('t', commands::THINKING),
            ('o', commands::CONFIG),
            ('c', commands::CONTEXT),
        ] {
            assert_eq!(
                command(KeyEvent::new(
                    KeyCode::Char(key),
                    modifier | KeyModifiers::SHIFT
                )),
                Some(expected)
            );
            assert!(help().contains(&format!("{label}+shift+{key} {expected}")));
        }
        assert!(help().contains("ctrl+d /quit"));
    }

    #[test]
    fn shortcut_commands_are_in_the_command_catalog() {
        for binding in BINDINGS {
            assert!(
                commands::COMMANDS
                    .iter()
                    .any(|cmd| cmd.name == binding.command)
            );
        }
    }

    #[test]
    fn shortcuts_require_exact_modifiers() {
        for c in ['m', 't', 'o', 'c'] {
            for modifiers in [
                KeyModifiers::NONE,
                KeyModifiers::SHIFT,
                KeyModifiers::CONTROL,
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
                COMMAND_MODIFIER,
                COMMAND_SHORTCUT | KeyModifiers::CONTROL,
            ] {
                assert_eq!(command(KeyEvent::new(KeyCode::Char(c), modifiers)), None);
            }
        }
        assert_eq!(
            command(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            None
        );
        assert_eq!(
            command(KeyEvent::new(KeyCode::Char('d'), COMMAND_SHORTCUT)),
            None
        );
    }

    fn app() -> crate::app::App {
        use crate::protocol::{Api, Config};

        let config = Config {
            api_key: "k".into(),
            base_url: "https://example.test".into(),
            model: "test".into(),
            provider: "test".into(),
            window: None,
            api: Api::Completions,
            auth_provider: None,
            auth_profile: None,
            thinking: "off".into(),
            thinking_levels: vec!["off".into(), "high".into()],
        };
        let mut app = crate::app::App::new(crate::lua::Loaded {
            config: Some(config.clone()),
            models: vec![crate::lua::ModelChoice {
                provider: "test".into(),
                alias: None,
                id: "test".into(),
                config: Some(config),
                error: None,
            }],
            stack: String::new(),
            notice: None,
            discovery: Default::default(),
        });
        app.input = "draft prompt".into();
        app.cursor = 5;
        app.history.clear();
        app
    }

    #[test]
    fn command_shortcuts_open_pickers_without_changing_the_draft() {
        use crate::app::Mode;
        use crate::event::on_key;

        for c in ['m', 't', 'c'] {
            let mut app = app();
            on_key(&mut app, KeyEvent::new(KeyCode::Char(c), COMMAND_SHORTCUT));
            match c {
                'm' => assert!(matches!(app.mode, Mode::Model { .. })),
                't' => assert!(matches!(app.mode, Mode::Thinking { .. })),
                'c' => assert!(matches!(app.mode, Mode::Context { .. })),
                _ => unreachable!(),
            }
            assert_eq!(app.input, "draft prompt");
            assert_eq!(app.cursor, 5);
            assert!(app.history.is_empty());
            on_key(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
            assert!(matches!(app.mode, Mode::Chat));
            assert_eq!(app.input, "draft prompt");
            assert_eq!(app.cursor, 5);
        }
    }

    #[test]
    fn command_shortcuts_do_not_change_an_active_turn_or_picker() {
        use crate::app::Mode;
        use crate::event::on_key;
        use std::sync::{Arc, atomic::AtomicBool};

        let mut app = app();
        app.cancel = Some(Arc::new(AtomicBool::new(false)));
        for c in ['m', 't', 'o', 'c'] {
            on_key(&mut app, KeyEvent::new(KeyCode::Char(c), COMMAND_SHORTCUT));
            assert!(matches!(app.mode, Mode::Chat));
            assert_eq!(app.input, "draft prompt");
            assert!(app.notice.is_none());
        }
        app.cancel = None;
        app.mode = Mode::Thinking { cursor: 0 };
        on_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('c'), COMMAND_SHORTCUT),
        );
        assert!(matches!(app.mode, Mode::Thinking { .. }));
    }

    #[test]
    fn ctrl_d_quits_even_with_a_draft_picker_or_login() {
        use crate::app::Mode;
        use crate::event::on_key;

        for mode in [Mode::Chat, Mode::Thinking { cursor: 0 }, Mode::LoginCode] {
            let mut app = app();
            app.mode = mode;
            on_key(
                &mut app,
                KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
            );
            assert!(app.quit);
            assert_eq!(app.input, "draft prompt");
        }
        let mut app = app();
        let (_tx, rx) = std::sync::mpsc::channel();
        app.auth_rx = Some(rx);
        on_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
        );
        assert!(app.quit);
    }

    #[test]
    fn ctrl_c_no_longer_quits_and_delete_still_deletes() {
        use crate::event::on_key;

        let mut app = app();
        on_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
        );
        assert!(!app.quit);
        assert_eq!(app.input, "draft prompt");
        on_key(&mut app, KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        assert_eq!(app.input, "draftprompt");
    }
}
