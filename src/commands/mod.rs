//! Built-in slash command dispatch and completion.

pub(crate) mod auth;
mod compact;
mod completion;
mod config;
mod context;
pub(crate) mod mission;
pub(crate) mod model;
pub(crate) mod thinking;

pub(crate) use compact::finish_compaction;
pub use completion::*;

use crate::app::App;

/// Returns false for ordinary prompts; unknown slash commands are handled here.
pub(crate) fn dispatch(app: &mut App, line: &str) -> bool {
    match line {
        "/quit" | "/q" => app.quit = true,
        "/help" => app.notice = Some(crate::commands::help()),
        "/config" => config::edit_config(app),
        "/new" => mission::new_mission(app),
        "/login" => auth::open_login(app),
        "/login xai" => auth::open_xai_login(app),
        "/login openai" => auth::start_openai_oauth(app),
        "/logout" => app.notice = Some("usage: /logout xai|openai".into()),
        "/logout xai" => auth::logout_xai(app),
        "/logout openai" => auth::logout_openai(app),
        "/resume" => mission::open_resume(app),
        "/model" => model::open_model(app),
        "/thinking" => thinking::open_thinking(app),
        cmd if let Some(raw) = cmd.strip_prefix("/thinking ") => thinking::apply(app, raw),
        "/mission" => mission::show_mission(app),
        "/compact" => compact::start_compaction(app, None),
        cmd if let Some(instructions) = cmd.strip_prefix("/compact ") => {
            compact::start_compaction(app, Some(instructions))
        }
        "/context" | "/context raw" => context::open(app, line == "/context raw"),
        cmd if let Some(name) = cmd.strip_prefix("/name ") => mission::name_mission(app, name),
        cmd if let Some(prefix) = cmd.strip_prefix("/resume ") => {
            mission::resume_prefix(app, prefix)
        }
        cmd if cmd.starts_with('/') => {
            app.notice = Some(format!("unknown command: {cmd}"));
        }
        _ => return false,
    }
    true
}
