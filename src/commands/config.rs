use crate::app::App;
use crate::view::draw;
use crate::{lua, terminal};

pub(crate) fn reload_config(app: &mut App) {
    let loaded = lua::load();
    let mut config = loaded.config.clone();
    if let (Some(config), Some(level)) = (&mut config, app.thinking_override.as_deref()) {
        if config.allows_thinking(level) {
            config.thinking = level.to_string();
        } else {
            app.thinking_override = None;
        }
    }
    app.config = config;
    app.startup_config = loaded.config;
    app.models = loaded.models;
    if let Some(notice) = loaded.notice {
        app.notice = Some(notice);
    }
}

pub(crate) fn edit_config(app: &mut App) {
    let editor = std::env::var("VISUAL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            std::env::var("EDITOR")
                .ok()
                .filter(|value| !value.trim().is_empty())
        });
    let Some(editor) = editor else {
        app.notice = Some("set VISUAL or EDITOR to edit init.lua".into());
        return;
    };

    let path = crate::storage::control("init.lua");
    if let Some(parent) = path.parent()
        && let Err(err) = std::fs::create_dir_all(parent)
    {
        app.notice = Some(format!("config: {err}"));
        return;
    }
    if !path.exists()
        && let Err(err) = std::fs::File::create(&path)
    {
        app.notice = Some(format!("config: {err}"));
        return;
    }

    terminal::suspend();
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg("exec $0 \"$@\"")
        .arg(editor)
        .arg(&path)
        .status();
    let mut terminal = terminal::resume();

    match status {
        Ok(_) => {
            app.notice = Some("config reloaded".into());
            reload_config(app);
        }
        Err(err) => app.notice = Some(format!("editor: {err}")),
    }
    let _ = terminal.clear();
    let _ = terminal.draw(|frame| draw(frame, app));
}
