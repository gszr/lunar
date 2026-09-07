use crate::app::{App, Mode};
use crate::mission;
use crate::turn::persist_value;

pub(crate) fn open_thinking(app: &mut App) {
    let Some(config) = app.config.as_ref() else {
        app.notice = Some("no model configured".into());
        return;
    };
    let cursor = config
        .thinking_levels
        .iter()
        .position(|level| level == &config.thinking)
        .unwrap_or(0);
    app.mode = Mode::Thinking { cursor };
}

pub(crate) fn set_thinking(app: &mut App, level: &str) -> bool {
    let Some(config) = &mut app.config else {
        return false;
    };
    if !config.allows_thinking(level) {
        return false;
    }
    app.thinking_override = Some(level.to_string());
    config.thinking = level.to_string();
    if app.mission.is_some() {
        persist_value(app, &mission::thinking_line(level));
    }
    true
}

pub(super) fn apply(app: &mut App, raw: &str) {
    let level = raw.trim();
    if set_thinking(app, level) {
        app.notice = Some(format!("thinking: {level}"));
    } else if let Some(config) = app.config.as_ref() {
        app.notice = Some(format!(
            "usage: /thinking {}",
            config.thinking_levels.join("|")
        ));
    } else {
        app.notice = Some("no model configured".into());
    }
}
