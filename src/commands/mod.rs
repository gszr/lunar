//! Built-in slash command dispatch and completion.

pub(crate) mod auth;
mod compact;
mod completion;
mod config;
mod context;
pub(crate) mod format;
pub(crate) mod mission;
pub(crate) mod model;
mod names;
pub(crate) mod thinking;

pub(crate) use compact::finish_compaction;
pub use completion::*;
pub use names::*;

use crate::app::App;

/// Returns false for ordinary prompts; unknown slash commands are handled here.
pub(crate) fn dispatch(app: &mut App, line: &str) -> bool {
    match line {
        QUIT | Q => app.quit = true,
        HELP => app.notice = Some(crate::commands::help()),
        DEBUG => {
            app.debug_headers = !app.debug_headers;
            app.notice = Some(if app.debug_headers {
                "debug: on".into()
            } else {
                "debug: off".into()
            });
        }
        MARKDOWN | MD => {
            app.markdown = !app.markdown;
            crate::transcript::invalidate_paint(app);
            app.notice = Some(if app.markdown {
                "markdown: on".into()
            } else {
                "markdown: off (plain text)".into()
            });
        }
        CONFIG => config::edit_config(app),
        NEW => mission::new_mission(app),
        LOGIN => auth::open_login(app),
        cmd if arguments(cmd, LOGIN) == Some("xai") => auth::open_xai_login(app),
        cmd if arguments(cmd, LOGIN) == Some("openai") => auth::start_openai_oauth(app),
        cmd if arguments(cmd, LOGIN) == Some("anthropic") => auth::start_anthropic_oauth(app),
        LOGOUT => app.notice = Some(format!("usage: {LOGOUT} xai|openai|anthropic")),
        cmd if arguments(cmd, LOGOUT) == Some("xai") => auth::logout_xai(app),
        cmd if arguments(cmd, LOGOUT) == Some("openai") => auth::logout_openai(app),
        command if arguments(command, LOGOUT).is_some_and(crate::auth::is_subscription_profile) => {
            let profile = arguments(command, LOGOUT).unwrap();
            auth::logout_provider(app, profile, profile);
        }
        cmd if arguments(cmd, LOGOUT) == Some("anthropic") => auth::logout_anthropic(app),
        RESUME => mission::open_resume(app),
        MODEL => model::open_model(app),
        cmd if let Some(id) = arguments(cmd, MODEL) => model::apply(app, id),
        cmd if arguments(cmd, MODELS) == Some("refresh") => model::refresh_models(app),
        cmd if cmd == MODELS || arguments(cmd, MODELS).is_some() => {
            app.notice = Some(format!("usage: {MODELS} refresh"));
        }
        FORMAT => {
            app.notice = Some(format!(
                "usage: {FORMAT} response|chat_completions|messages"
            ))
        }
        cmd if let Some(raw) = arguments(cmd, FORMAT) => format::apply(app, raw),
        THINKING => thinking::open_thinking(app),
        cmd if let Some(raw) = arguments(cmd, THINKING) => thinking::apply(app, raw),
        MISSION => mission::show_mission(app),
        BTW => app.notice = Some(format!("usage: {BTW} <prompt>")),
        cmd if let Some(prompt) = arguments(cmd, BTW) => crate::turn::send_btw(app, prompt),
        COMPACT => compact::start_compaction(app, None),
        cmd if let Some(instructions) = arguments(cmd, COMPACT) => {
            compact::start_compaction(app, Some(instructions))
        }
        CONTEXT => context::open(app, false),
        cmd if arguments(cmd, CONTEXT) == Some("raw") => context::open(app, true),
        cmd if let Some(name) = arguments(cmd, NAME) => mission::name_mission(app, name),
        cmd if let Some(prefix) = arguments(cmd, RESUME) => mission::resume_prefix(app, prefix),
        cmd if cmd.starts_with('/') => {
            app.notice = Some(format!("unknown command: {cmd}"));
        }
        _ => return false,
    }
    true
}

/// Arguments follow the exact command name and one space; prefixes alone do not match.
pub(crate) fn arguments<'a>(line: &'a str, command: &str) -> Option<&'a str> {
    line.strip_prefix(command)?.strip_prefix(' ')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_preserve_exact_command_boundaries_and_text() {
        assert_eq!(arguments("/thinking high", THINKING), Some("high"));
        assert_eq!(arguments("/btw  keep spaces", BTW), Some(" keep spaces"));
        assert_eq!(arguments("/compact ", COMPACT), Some(""));
        for line in [
            "/thinking",
            "/thinking-more high",
            "/Thinking high",
            "/thinking\thigh",
        ] {
            assert_eq!(arguments(line, THINKING), None);
        }
    }
}
