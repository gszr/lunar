//! Authentication, configuration, model, and mission actions.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{self, TryRecvError};

use crate::app::{App, AuthEvent, AuthPrompt, Message, Mode};
use crate::protocol::Usage;
use crate::transcript::{invalidate_paint, jump_to_tail};
use crate::turn::persist_value;
use crate::view::draw;
use crate::{auth, lua, mission, terminal};

pub(crate) fn drain_auth(app: &mut App) {
    let Some(rx) = app.auth_rx.as_ref() else {
        return;
    };
    let event = match rx.try_recv() {
        Ok(event) => Some(event),
        Err(TryRecvError::Disconnected) => Some(AuthEvent::Failed("login ended".into())),
        Err(TryRecvError::Empty) => None,
    };
    match event {
        Some(AuthEvent::DeviceCode {
            url,
            code,
            browser_opened,
        }) => {
            app.auth_prompt = Some(AuthPrompt {
                url,
                code,
                browser_opened,
            });
        }
        Some(AuthEvent::Done) => {
            let brand = app.auth_brand.take().unwrap_or("xAI");
            app.auth_rx = None;
            app.auth_cancel = None;
            app.auth_prompt = None;
            app.notice = Some(format!("logged in to {brand}"));
            reload_config(app);
        }
        Some(AuthEvent::Failed(err)) => {
            app.auth_brand = None;
            app.auth_rx = None;
            app.auth_cancel = None;
            app.auth_prompt = None;
            app.notice = Some(err);
        }
        None => {}
    }
}

pub(crate) fn open_login(app: &mut App) {
    app.mode = Mode::LoginProvider { cursor: 0 };
}

pub(crate) fn open_xai_login(app: &mut App) {
    app.mode = Mode::LoginMethod { cursor: 0 };
}

pub(crate) fn start_xai_oauth(app: &mut App) {
    start_oauth(app, "xAI", |cancel, tx| {
        auth::request_xai_device_code()
            .and_then(|device| {
                let browser_opened = webbrowser::open(&device.verification_uri).is_ok();
                let _ = tx.send(AuthEvent::DeviceCode {
                    url: device.verification_uri.clone(),
                    code: device.user_code.clone(),
                    browser_opened,
                });
                auth::poll_xai(&device, cancel)
            })
            .and_then(|credential| auth::save_oauth("xai", credential))
    });
}

pub(crate) fn start_openai_oauth(app: &mut App) {
    start_oauth(app, "OpenAI", |cancel, tx| {
        auth::request_openai_device_code()
            .and_then(|device| {
                let browser_opened = webbrowser::open(&device.verification_uri).is_ok();
                let _ = tx.send(AuthEvent::DeviceCode {
                    url: device.verification_uri.clone(),
                    code: device.user_code.clone(),
                    browser_opened,
                });
                auth::poll_openai(&device, cancel)
            })
            .and_then(|credential| auth::save_oauth("openai", credential))
    });
}

fn start_oauth(
    app: &mut App,
    brand: &'static str,
    run: impl FnOnce(&AtomicBool, mpsc::Sender<AuthEvent>) -> Result<(), String> + Send + 'static,
) {
    app.mode = Mode::Chat;
    let cancel = Arc::new(AtomicBool::new(false));
    let thread_cancel = cancel.clone();
    let (tx, rx) = mpsc::channel();
    app.auth_cancel = Some(cancel);
    app.auth_rx = Some(rx);
    app.auth_prompt = None;
    app.auth_brand = Some(brand);
    app.notice = None;
    std::thread::spawn(move || {
        let result = run(&thread_cancel, tx.clone());
        let _ = tx.send(match result {
            Ok(()) => AuthEvent::Done,
            Err(err) => AuthEvent::Failed(err),
        });
    });
}

pub(crate) fn save_api_key(app: &mut App) {
    let key = std::mem::take(&mut app.input);
    app.cursor = 0;
    app.mode = Mode::Chat;
    match auth::save_api_key("xai", &key) {
        Ok(()) => {
            app.notice = Some("saved xAI API key".into());
            reload_config(app);
        }
        Err(err) => app.notice = Some(err),
    }
}

pub(crate) fn logout_xai(app: &mut App) {
    logout_provider(app, "xai", "xAI");
}

pub(crate) fn logout_openai(app: &mut App) {
    logout_provider(app, "openai", "OpenAI");
}

fn logout_provider(app: &mut App, provider: &str, brand: &str) {
    match auth::logout(provider) {
        Ok(true) => {
            app.notice = Some(format!("logged out of {brand}"));
            reload_config(app);
        }
        Ok(false) => app.notice = Some(format!("not logged in to {brand}")),
        Err(err) => app.notice = Some(err),
    }
}

pub(crate) fn reload_config(app: &mut App) {
    let loaded = lua::load();
    app.config = loaded.config.clone();
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

    let path = mission::home().join("init.lua");
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

pub(crate) fn new_mission(app: &mut App) {
    if app.cancel.is_some() {
        return;
    }
    app.messages.clear();
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

pub(crate) fn open_model(app: &mut App) {
    if app.models.is_empty() {
        app.notice = Some("no configured models".into());
        return;
    }
    let cursor = app
        .config
        .as_ref()
        .and_then(|cfg| {
            app.models
                .iter()
                .position(|m| m.provider == cfg.provider && m.id == cfg.model)
        })
        .unwrap_or(0);
    app.mode = Mode::Model {
        items: app.models.clone(),
        cursor,
    };
}

pub(crate) fn select_model(app: &mut App, item: lua::ModelChoice, persist: bool) {
    let Some(config) = item.config else {
        app.notice = Some(item.error.unwrap_or_else(|| "model is unavailable".into()));
        return;
    };
    app.config = Some(config);
    app.notice = Some(format!("model: {} / {}", item.provider, item.id));
    if persist {
        persist_value(app, &mission::model_line(&item.provider, &item.id));
    }
}

pub(crate) fn restore_model(app: &mut App, provider: &str, id: &str) -> bool {
    let choice = app
        .models
        .iter()
        .find(|m| m.provider == provider && m.id == id)
        .cloned();
    if let Some(item) = choice.filter(|m| m.config.is_some()) {
        select_model(app, item, false);
        true
    } else {
        false
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
        Ok((loaded, saved)) => {
            app.config = app.startup_config.clone();
            let persisted_model = saved.iter().rev().find_map(|s| match s {
                mission::Saved::Model { provider, id } => Some((provider.clone(), id.clone())),
                _ => None,
            });
            app.messages = saved
                .into_iter()
                .filter_map(|s| match s {
                    mission::Saved::Model { .. } => None,
                    mission::Saved::User(text) => Some(Message::user(text)),
                    mission::Saved::Assistant { text, tool_calls } => {
                        let mut msg = Message::assistant();
                        msg.text = text;
                        msg.tool_calls = tool_calls;
                        Some(msg)
                    }
                    mission::Saved::Tool { id, title, content } => {
                        Some(Message::tool(id, title, content))
                    }
                })
                .collect();
            app.mission = Some(loaded);
            invalidate_paint(app);
            app.usage = Usage::default();
            app.last_prompt = 0;
            app.notice = None;
            if let Some((provider, id)) = persisted_model
                && !restore_model(app, &provider, &id)
            {
                app.notice = Some(format!(
                    "saved model {provider} / {id} is no longer configured; using startup default"
                ));
            }
            jump_to_tail(app);
        }
        Err(err) => app.notice = Some(format!("resume: {err}")),
    }
}
