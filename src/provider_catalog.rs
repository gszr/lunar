//! Model discovery for Lua providers with `models = "auto"`.

use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use serde_json::Value;

use crate::app::{App, Mode};
use crate::lua::ModelChoice;
use crate::protocol::{self, Config};

pub(crate) struct CatalogResult {
    provider: String,
    models: Result<Vec<ModelChoice>, String>,
}

#[derive(Default)]
pub(crate) struct Discovery {
    pub(crate) providers: Vec<Config>,
    pub(crate) default: Option<(String, String)>,
    pub(crate) rx: Option<Receiver<CatalogResult>>,
}

pub(crate) fn refresh(app: &mut App) {
    let (tx, rx) = mpsc::channel();
    app.discovery.rx = None;
    if app.discovery.providers.is_empty() {
        return;
    }
    app.discovery.rx = Some(rx);
    for config in app.discovery.providers.clone() {
        let tx = tx.clone();
        std::thread::spawn(move || {
            let result = fetch(&config).and_then(|catalog| choices(&catalog, &config));
            let _ = tx.send(CatalogResult {
                provider: config.provider,
                models: result,
            });
        });
    }
}

pub(crate) fn drain(app: &mut App) {
    loop {
        let Some(rx) = &app.discovery.rx else {
            return;
        };
        let CatalogResult {
            provider,
            models: result,
        } = match rx.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                app.discovery.rx = None;
                return;
            }
        };
        match result {
            Ok(models) => {
                app.models.retain(|model| model.provider != provider);
                app.models.extend(models);
                app.notice = Some(format!("{provider} models loaded; choose with /model"));
                if let Some((name, id)) = &app.discovery.default
                    && name == &provider
                {
                    let selected = app
                        .models
                        .iter()
                        .find(|m| m.provider == *name && m.id == *id);
                    if let Some(config) = selected.and_then(|m| m.config.clone()) {
                        app.startup_config = Some(config.clone());
                        if app.config.is_none() {
                            let mut config = config;
                            if let Some(level) = &app.thinking_override
                                && config.allows_thinking(level)
                            {
                                config.thinking = level.clone();
                            }
                            match crate::commands::format::override_config(
                                app.format_override,
                                &mut config,
                            ) {
                                Ok(()) => app.config = Some(config),
                                Err(error) => app.notice = Some(error),
                            }
                        }
                    } else {
                        app.notice = Some(format!(
                            "{provider}: default model {id} unavailable; choose with /model"
                        ));
                    }
                }
                if let Mode::Model { items, cursor, .. } = &mut app.mode {
                    *items = app
                        .models
                        .iter()
                        .chain(&app.subscription.models)
                        .cloned()
                        .collect();
                    *cursor = (*cursor).min(items.len().saturating_sub(1));
                }
            }
            Err(error) => {
                app.notice = Some(format!("{provider}: models could not be loaded: {error}"));
            }
        }
    }
}

fn fetch(config: &Config) -> Result<Value, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .http_status_as_error(false)
        .build()
        .into();
    let url = format!("{}/models", config.base_url.trim_end_matches('/'));
    let mut request = agent.get(&url);
    if !config.api_key.is_empty() {
        request = request.header("Authorization", &format!("Bearer {}", config.api_key));
    }
    if config.auth_provider.as_deref() == Some("openai") {
        let account = crate::auth::chatgpt_account_id(&config.api_key)?;
        request = request
            .header("chatgpt-account-id", &account)
            .header("originator", "lunar");
    }
    let mut response = request.call().map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!("discovery HTTP {}", response.status().as_u16()));
    }
    let text = response
        .body_mut()
        .read_to_string()
        .map_err(|error| error.to_string())?;
    serde_json::from_str(&text).map_err(|error| error.to_string())
}

fn choices(catalog: &Value, template: &Config) -> Result<Vec<ModelChoice>, String> {
    let entries = catalog["data"]
        .as_array()
        .ok_or("catalog has no data array")?;
    let mut models = Vec::new();
    for entry in entries {
        let id = entry["id"]
            .as_str()
            .filter(|id| !id.trim().is_empty())
            .ok_or("model has no id")?;
        if models.iter().any(|model: &ModelChoice| model.id == id) {
            return Err("catalog has duplicate model IDs".into());
        }
        let mut config = template.clone();
        config.model = id.into();
        config.window = entry["context_window"]
            .as_u64()
            .or_else(|| entry["context_length"].as_u64())
            .or_else(|| entry["max_input_tokens"].as_u64())
            .and_then(|window| u32::try_from(window).ok())
            .filter(|window| *window > 0)
            .or_else(|| protocol::guess_window(id));
        models.push(ModelChoice {
            provider: config.provider.clone(),
            alias: None,
            id: id.into(),
            config: Some(config),
            error: None,
        });
    }
    if models.is_empty() {
        return Err("catalog has no models".into());
    }
    Ok(models)
}
