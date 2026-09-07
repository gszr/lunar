use super::model::restore_model;
use crate::app::{App, Mode};
use crate::mission;
use crate::protocol::Usage;
use crate::transcript::{invalidate_paint, jump_to_tail};

pub(crate) fn new_mission(app: &mut App) {
    if app.cancel.is_some() {
        return;
    }
    app.messages.clear();
    app.compaction = None;
    app.compacting = None;
    app.thinking_override = None;
    app.config = app.startup_config.clone();
    invalidate_paint(app);
    app.mission = None;
    app.usage = Usage::default();
    app.last_prompt = 0;
    app.notice = Some("new mission".into());
    jump_to_tail(app);
}

pub(crate) fn name_mission(app: &mut App, name: &str) {
    let name = name.trim();
    if name.is_empty() {
        app.notice = Some("usage: /name <text>".into());
        return;
    }
    if app.mission.is_none() {
        app.notice = Some("no mission yet".into());
        return;
    }
    if let Some(mission) = &mut app.mission
        && let Err(err) = mission::set_name(mission, name)
    {
        app.notice = Some(format!("mission: {err}"));
    }
}

pub(crate) fn show_mission(app: &mut App) {
    match &app.mission {
        Some(m) => {
            app.notice = Some(format!("{}  {}", m.label(), m.path.display()));
        }
        None => app.notice = Some("no mission yet".into()),
    }
}

pub(crate) fn open_resume(app: &mut App) {
    match mission::list() {
        Ok(items) if items.is_empty() => app.notice = Some("no missions in this directory".into()),
        Ok(items) => {
            app.mode = Mode::Resume {
                items,
                cursor: 0,
                title: "resume".into(),
                query: None,
            };
        }
        Err(err) => app.notice = Some(format!("resume: {err}")),
    }
}

pub(crate) fn resume_prefix(app: &mut App, prefix: &str) {
    let prefix = prefix.trim();
    match mission::list() {
        Ok(items) => {
            if let Some(meta) = items.into_iter().find(|m| {
                m.id.starts_with(prefix) || m.name.as_deref().is_some_and(|n| n.starts_with(prefix))
            }) {
                load_mission(app, &meta.path);
            } else {
                app.notice = Some(format!("no mission matching {prefix}"));
            }
        }
        Err(err) => app.notice = Some(format!("resume: {err}")),
    }
}

pub(crate) fn load_mission(app: &mut App, path: &std::path::Path) {
    match mission::load(path) {
        Ok(loaded) => {
            app.thinking_override = None;
            app.config = app.startup_config.clone();
            app.messages = loaded.messages;
            app.compaction =
                loaded
                    .compaction
                    .map(|(summary, first_kept)| crate::app::Compaction {
                        summary,
                        first_kept,
                    });
            app.compacting = None;
            app.mission = Some(loaded.mission);
            invalidate_paint(app);
            app.usage = loaded.usage;
            app.last_prompt = loaded.last_prompt;
            app.notice = None;
            if let Some((provider, id)) = loaded.model
                && !restore_model(app, &provider, &id)
            {
                app.notice = Some(format!(
                    "saved model {provider} / {id} is no longer configured; using startup default"
                ));
            }
            if let Some(level) = loaded.thinking {
                match app.config.as_mut() {
                    Some(config) if config.allows_thinking(&level) => {
                        app.thinking_override = Some(level.clone());
                        config.thinking = level;
                    }
                    Some(config) if app.notice.is_none() => {
                        app.notice = Some(format!(
                            "saved thinking level is not supported by {}: {level}",
                            config.model
                        ));
                    }
                    _ => {}
                }
            }
            jump_to_tail(app);
        }
        Err(err) => app.notice = Some(format!("resume: {err}")),
    }
}
