mod actions;
mod app;
mod auth;
mod cli;
mod commands;
mod debug;
mod event;
mod history;
mod input;
mod lua;
mod mission;
mod prompt;
mod protocol;
mod render;
mod splash;
mod terminal;
mod tool_output;
mod tools;
mod transcript;
mod turn;
mod view;

use std::io;

use actions::load_mission;
use app::App;

fn main() -> io::Result<()> {
    let open = match cli::parse(std::env::args_os().skip(1)) {
        Ok(cli::Action::Open(options)) => options,
        Ok(cli::Action::Help) => {
            print!("{}", cli::HELP);
            return Ok(());
        }
        Err(message) => {
            eprintln!("lunar: {message}\n\nTry 'lunar --help' for more information.");
            std::process::exit(2);
        }
    };
    tool_output::cleanup();
    let loaded = lua::load();
    let mut terminal = terminal::Terminal::init();
    let mut app = App::new(loaded);
    if open.debug
        && let Err(err) = debug::enable()
    {
        app.notice = Some(format!("debug log: {err}"));
    }
    match open.open {
        cli::Open::New => {}
        cli::Open::Continue => match mission::list()?.into_iter().next() {
            Some(meta) => load_mission(&mut app, &meta.path),
            None => app.notice = Some("no missions in this directory".into()),
        },
        cli::Open::Mission(selector) => {
            let all = mission::list()?;
            let selection = selector
                .as_deref()
                .map(|selector| mission::select(&all, selector))
                .unwrap_or_else(|| mission::Selection::Log(all));
            match selection {
                mission::Selection::Mission(meta) => load_mission(&mut app, &meta.path),
                mission::Selection::Log(items) => {
                    app.mode = app::Mode::Resume {
                        items,
                        cursor: 0,
                        title: selector
                            .filter(|selector| {
                                selector.len() == 10
                                    && selector.as_bytes().get(4) == Some(&b'-')
                                    && selector.as_bytes().get(7) == Some(&b'-')
                            })
                            .map(|selector| format!("missions · {selector}"))
                            .unwrap_or_else(|| "missions".into()),
                    };
                }
            }
        }
    }
    if app.notice.is_none() {
        app.notice = prompt::budget_warning();
    }
    event::run(terminal.get_mut(), &mut app)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Message, Mode};
    use crate::event::{on_key, on_paste};
    use crate::protocol::{ToolCall, Usage};
    use crate::transcript::painted_lines;
    use crate::transcript::{jump_to_tail, on_mouse};
    use crate::turn::{run_tools_parallel, skipped_truncated};
    use crate::view::{char_wrap, cursor_xy, working_text};
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::crossterm::event::{MouseEvent, MouseEventKind};
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, mpsc};

    fn test_app() -> App {
        App {
            input: String::new(),
            cursor: 0,
            notice: None,
            messages: Vec::new(),
            config: None,
            startup_config: None,
            models: Vec::new(),
            stream_rx: None,
            cancel: None,
            rounds: 0,
            usage: Usage::default(),
            last_prompt: 0,
            preamble: None,
            mission: None,
            mode: Mode::Chat,
            complete_sel: 0,
            quit: false,
            scroll: 0,
            follow: true,
            transcript_w: 0,
            transcript_h: 0,
            paint_width: 0,
            paint_frozen: Vec::new(),
            paint_upto: 0,
            paint_prev_tool: false,
            history: Vec::new(),
            history_cursor: None,
            history_draft: String::new(),
            search: None,
            auth_rx: None,
            auth_cancel: None,
            auth_prompt: None,
            auth_brand: None,
        }
    }

    fn key(modifiers: KeyModifiers, code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn multiline_paste_inserts_without_sending() {
        let mut app = test_app();
        app.input = "ab".into();
        app.cursor = 1;
        on_paste(&mut app, "one\r\ntwo\rthree");
        assert_eq!(app.input, "aone\ntwo\nthreeb");
        assert_eq!(app.cursor, 14);
        assert!(app.messages.is_empty());
        assert_eq!(app.notice, None);
    }

    #[test]
    fn shift_enter_inserts_newline() {
        let mut app = test_app();
        app.input = "ab".into();
        app.cursor = 1;
        on_key(&mut app, key(KeyModifiers::SHIFT, KeyCode::Enter));
        assert_eq!(app.input, "a\nb");
        assert_eq!(app.cursor, 2);
        assert!(app.messages.is_empty());
    }

    #[test]
    fn ctrl_j_inserts_newline() {
        let mut app = test_app();
        app.input = "hi".into();
        app.cursor = 2;
        on_key(&mut app, key(KeyModifiers::CONTROL, KeyCode::Char('j')));
        assert_eq!(app.input, "hi\n");
        assert_eq!(app.cursor, 3);
    }

    #[test]
    fn enter_still_sends() {
        let mut app = test_app();
        app.input = "hi".into();
        app.cursor = 2;
        on_key(&mut app, key(KeyModifiers::NONE, KeyCode::Enter));
        assert_eq!(app.input, "");
        assert_eq!(app.notice.as_deref(), Some("no model configured"));
    }

    #[test]
    fn shift_enter_does_not_accept_completion() {
        let mut app = test_app();
        app.input = "/he".into();
        app.cursor = 3;
        on_key(&mut app, key(KeyModifiers::SHIFT, KeyCode::Enter));
        assert_eq!(app.input, "/he\n");
        assert_eq!(app.notice, None);
    }

    #[test]
    fn arrows_walk_history_and_restore_draft() {
        let mut app = test_app();
        app.history = vec!["one".into(), "two".into()];
        app.input = "draft".into();
        app.cursor = 5;
        on_key(&mut app, key(KeyModifiers::NONE, KeyCode::Up));
        assert_eq!(app.input, "two");
        on_key(&mut app, key(KeyModifiers::NONE, KeyCode::Up));
        assert_eq!(app.input, "one");
        on_key(&mut app, key(KeyModifiers::NONE, KeyCode::Down));
        on_key(&mut app, key(KeyModifiers::NONE, KeyCode::Down));
        assert_eq!(app.input, "draft");
    }

    #[test]
    fn up_from_typed_command_walks_history() {
        let mut app = test_app();
        app.history = vec!["previous prompt".into()];
        app.input = "/help".into();
        app.cursor = 5;
        on_key(&mut app, key(KeyModifiers::NONE, KeyCode::Up));
        assert_eq!(app.input, "previous prompt");
    }

    #[test]
    fn up_cycles_completion_when_several_commands_match() {
        let mut app = test_app();
        app.history = vec!["previous prompt".into()];
        app.input = "/".into();
        app.cursor = 1;
        on_key(&mut app, key(KeyModifiers::NONE, KeyCode::Up));
        assert_eq!(app.input, "/");
        assert_eq!(app.complete_sel, crate::commands::COMMANDS.len() - 1);
    }

    #[test]
    fn reverse_search_cycles_and_escape_restores_draft() {
        let mut app = test_app();
        app.history = vec!["cargo test".into(), "git status".into(), "cargo fmt".into()];
        app.input = "draft".into();
        app.cursor = 5;
        on_key(&mut app, key(KeyModifiers::CONTROL, KeyCode::Char('r')));
        on_key(&mut app, key(KeyModifiers::NONE, KeyCode::Char('c')));
        assert_eq!(app.search.as_ref().unwrap().matched, Some(2));
        on_key(&mut app, key(KeyModifiers::CONTROL, KeyCode::Char('r')));
        assert_eq!(app.search.as_ref().unwrap().matched, Some(0));
        on_key(&mut app, key(KeyModifiers::NONE, KeyCode::Esc));
        assert_eq!(app.input, "draft");
    }

    #[test]
    fn reverse_search_enter_accepts_without_submitting() {
        let mut app = test_app();
        app.history = vec!["cargo test".into()];
        on_key(&mut app, key(KeyModifiers::CONTROL, KeyCode::Char('r')));
        on_key(&mut app, key(KeyModifiers::NONE, KeyCode::Char('t')));
        on_key(&mut app, key(KeyModifiers::NONE, KeyCode::Enter));
        assert_eq!(app.input, "cargo test");
        assert!(app.messages.is_empty());
    }

    #[test]
    fn char_wrap_keeps_hard_newlines() {
        assert_eq!(char_wrap("ab\ncd", 10), vec!["ab", "cd"]);
        assert_eq!(char_wrap("hello\n", 10), vec!["hello", ""]);
        assert_eq!(char_wrap("abcd", 2), vec!["ab", "cd"]);
        assert_eq!(char_wrap("ab\ncdef", 2), vec!["ab", "cd", "ef"]);
    }

    #[test]
    fn cursor_follows_hard_newline() {
        assert_eq!(cursor_xy("ab\ncd", 3, 10), (1, 0));
        assert_eq!(cursor_xy("ab\ncd", 5, 10), (1, 2));
        assert_eq!(cursor_xy("hello", 5, 5), (1, 0));
    }

    #[test]
    fn tools_in_a_round_keep_call_order() {
        let cancel = AtomicBool::new(false);
        let calls = vec![
            ToolCall {
                id: "1".into(),
                name: "read".into(),
                arguments: r#"{"path":"Cargo.toml"}"#.into(),
            },
            ToolCall {
                id: "2".into(),
                name: "read".into(),
                arguments: r#"{"path":"LICENSE"}"#.into(),
            },
        ];
        let results = run_tools_parallel(&calls, &cancel);
        assert_eq!(results[0].id, "1");
        assert_eq!(results[1].id, "2");
        assert!(results[0].content.contains("lunar"));
        assert!(results[1].content.contains("MIT"));
    }

    #[test]
    fn paint_cache_freezes_finished_messages() {
        let mut app = test_app();
        app.messages.push(Message::user("hello".into()));
        let first = painted_lines(&mut app, 40).len();
        assert_eq!(app.paint_upto, 1);
        let frozen = app.paint_frozen.len();
        app.messages.push(Message::user("again".into()));
        let second = painted_lines(&mut app, 40).len();
        assert_eq!(app.paint_upto, 2);
        assert_eq!(app.paint_frozen.len(), frozen + second - first);
    }

    #[test]
    fn truncated_calls_are_not_executed() {
        let calls = vec![ToolCall {
            id: "1".into(),
            name: "bash".into(),
            arguments: r#"{"command":"echo should-not-run"}"#.into(),
        }];
        let results = skipped_truncated(&calls);
        assert_eq!(results[0].id, "1");
        assert!(results[0].content.contains("token limit"));
        assert!(!results[0].content.contains("should-not-run"));
    }

    #[test]
    fn working_text_is_thinking_until_tools() {
        let mut app = test_app();
        app.messages.push(Message::assistant());
        assert_eq!(working_text(&app), " Thinking...");
        app.messages.last_mut().unwrap().tool_calls.push(ToolCall {
            id: "1".into(),
            name: "bash".into(),
            arguments: "{}".into(),
        });
        assert_eq!(working_text(&app), " Running tools...");
    }

    #[test]
    fn esc_aborts_a_turn_immediately() {
        let mut app = test_app();
        app.cancel = Some(Arc::new(AtomicBool::new(false)));
        let (_tx, rx) = mpsc::channel();
        app.stream_rx = Some(rx);
        app.messages.push(Message::assistant());
        on_key(&mut app, key(KeyModifiers::NONE, KeyCode::Esc));
        assert!(app.cancel.is_none());
        assert!(app.stream_rx.is_none());
        assert!(app.messages.is_empty());
        assert_eq!(app.notice.as_deref(), Some("aborted"));
    }

    fn tall_app() -> App {
        let mut app = test_app();
        app.transcript_w = 40;
        app.transcript_h = 8;
        for i in 0..12 {
            app.messages.push(Message::user(format!("line {i}")));
        }
        let max = painted_lines(&mut app, 40).len().saturating_sub(8);
        app.scroll = max;
        app.follow = true;
        app
    }

    #[test]
    fn page_up_leaves_follow() {
        let mut app = tall_app();
        on_key(&mut app, key(KeyModifiers::NONE, KeyCode::PageUp));
        assert!(!app.follow);
        assert!(app.scroll < painted_lines(&mut app, 40).len().saturating_sub(8));
    }

    #[test]
    fn page_down_to_end_follows_again() {
        let mut app = tall_app();
        on_key(&mut app, key(KeyModifiers::NONE, KeyCode::PageUp));
        on_key(&mut app, key(KeyModifiers::NONE, KeyCode::PageDown));
        on_key(&mut app, key(KeyModifiers::NONE, KeyCode::PageDown));
        assert!(app.follow);
    }

    #[test]
    fn ctrl_home_goes_to_top() {
        let mut app = tall_app();
        on_key(&mut app, key(KeyModifiers::CONTROL, KeyCode::Home));
        assert_eq!(app.scroll, 0);
        assert!(!app.follow);
    }

    #[test]
    fn wheel_does_nothing_in_resume() {
        let mut app = tall_app();
        app.mode = Mode::Resume {
            items: Vec::new(),
            cursor: 0,
            title: "resume".into(),
        };
        let before = app.scroll;
        on_mouse(
            &mut app,
            MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: 0,
                row: 0,
                modifiers: KeyModifiers::NONE,
            },
        );
        assert_eq!(app.scroll, before);
        assert!(app.follow);
    }

    #[test]
    fn submit_jumps_to_tail() {
        let mut app = tall_app();
        app.follow = false;
        app.scroll = 0;
        jump_to_tail(&mut app);
        assert!(app.follow);
    }
}
