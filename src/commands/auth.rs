use super::config::reload_config;
use crate::app::{App, AuthEvent, AuthPrompt, Mode};
use crate::auth;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{self, TryRecvError};

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
