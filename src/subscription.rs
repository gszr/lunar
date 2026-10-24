//! Subscription models, independent of Lua configuration.

use std::sync::mpsc::{self, Receiver};

use serde_json::{Value, json};

use crate::app::{App, Mode};
use crate::lua::ModelChoice;
use crate::protocol::{Api, Config};

const PROVIDER: &str = "OpenAI subscription";
const BASE_URL: &str = "https://chatgpt.com/backend-api";
// Codex's catalog uses the client version to gate model capabilities.
const CLIENT_VERSION: &str = "0.164.0";

#[derive(Default)]
pub(crate) struct Subscription {
    pub(crate) models: Vec<ModelChoice>,
    pub(crate) rx: Option<Receiver<Result<Value, String>>>,
    account: Option<String>,
    pub(crate) profile: String,
    pending: Vec<(String, String)>,
}

pub(crate) fn refresh(app: &mut App) {
    // Dropping the receiver prevents a previous login's response being installed.
    app.subscription.rx = None;
    let profiles = match crate::auth::subscription_profiles() {
        Ok(profiles) => profiles,
        Err(error) => {
            app.notice = Some(error);
            return;
        }
    };
    app.subscription
        .models
        .retain(|model| profiles.iter().any(|(name, _)| name == &model.provider));
    if app.config.as_ref().is_some_and(|config| {
        is_subscription(config) && !profiles.iter().any(|(name, _)| name == &config.provider)
    }) {
        app.config = None;
        app.thinking_override = None;
        crate::limits::refresh(app);
    }
    for (profile, account) in &profiles {
        if app
            .subscription
            .models
            .iter()
            .any(|model| &model.provider == profile)
        {
            continue;
        }
        let path = crate::storage::recorder(&format!("{profile}-models.json"));
        let text = std::fs::read_to_string(path)
            .or_else(|_| std::fs::read_to_string(crate::storage::recorder("openai-models.json")));
        if let Ok(text) = text
            && let Ok(cache) = serde_json::from_str::<Value>(&text)
            && cache["account"].as_str() == Some(account)
            && let Ok(models) = profile_choices(&cache["catalog"], profile)
        {
            app.subscription.models.extend(models);
        }
    }
    app.subscription.pending = profiles;
    start_next(app);
}

fn start_next(app: &mut App) {
    let Some((profile, account)) = app.subscription.pending.pop() else {
        return;
    };
    app.subscription.profile = profile.clone();
    app.subscription.account = Some(account.clone());
    let (tx, rx) = mpsc::channel();
    app.subscription.rx = Some(rx);
    std::thread::spawn(move || {
        let _ = tx.send(fetch(&profile, &account));
    });
}

pub(crate) fn is_subscription(config: &Config) -> bool {
    config.auth_profile.is_some()
}

pub(crate) fn drain(app: &mut App) {
    let Some(rx) = &app.subscription.rx else {
        return;
    };
    let result = match rx.try_recv() {
        Ok(result) => result,
        Err(mpsc::TryRecvError::Empty) => return,
        Err(mpsc::TryRecvError::Disconnected) => Err("catalog request ended".into()),
    };
    app.subscription.rx = None;
    match result.and_then(|catalog| {
        profile_choices(&catalog, &app.subscription.profile).map(|models| (catalog, models))
    }) {
        Ok((catalog, models)) => {
            let cached: Vec<_> = app
                .subscription
                .models
                .iter()
                .filter(|model| model.provider == app.subscription.profile)
                .map(|model| model.id.clone())
                .collect();
            let changed = cached
                != models
                    .iter()
                    .map(|model| model.id.clone())
                    .collect::<Vec<_>>();
            app.subscription
                .models
                .retain(|model| model.provider != app.subscription.profile);
            app.subscription.models.extend(models);
            let cache = json!({"account": app.subscription.account, "catalog": catalog});
            let path =
                crate::storage::recorder(&format!("{}-models.json", app.subscription.profile));
            let saved = (|| -> std::io::Result<()> {
                std::fs::create_dir_all(path.parent().unwrap())?;
                let temp = path.with_extension("json.tmp");
                std::fs::write(&temp, cache.to_string())?;
                std::fs::rename(temp, path)
            })();
            // A refresh that matches the cache is routine; only report real news.
            app.notice = match saved {
                Ok(()) if !changed => app.notice.take(),
                Ok(()) => Some(format!(
                    "{} models loaded; choose with /model",
                    app.subscription.profile
                )),
                Err(error) => Some(format!(
                    "{} models loaded, but could not cache them: {error}",
                    app.subscription.profile
                )),
            };
        }
        Err(error) => {
            let fallback = if app.subscription.models.is_empty() {
                "connection saved, but models could not be loaded"
            } else {
                "showing cached models"
            };
            app.notice = Some(format!("{}: {fallback}: {error}", app.subscription.profile));
        }
    }
    if app.config.is_none()
        && let Ok(loaded) = crate::lua::load()
        && let Some(config) = loaded.config.filter(is_subscription)
    {
        app.config = Some(config);
        crate::limits::refresh(app);
    }
    start_next(app);
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

fn fetch(profile: &str, account: &str) -> Result<Value, String> {
    let integration =
        crate::auth::subscription_integration(profile).ok_or("invalid subscription profile")?;
    let access = crate::auth::resolve_profile(integration, Some(profile))?;
    if integration != "openai" {
        return crate::subscription_catalog::fetch(integration, &access);
    }
    if crate::auth::chatgpt_account_id(&access)? != account {
        return Err("account changed; log in again".into());
    }
    crate::subscription_catalog::get(
        &format!("{BASE_URL}/codex/models?client_version={CLIENT_VERSION}"),
        integration,
        &access,
        Some(account),
    )
}

fn profile_choices(catalog: &Value, profile: &str) -> Result<Vec<ModelChoice>, String> {
    let integration =
        crate::auth::subscription_integration(profile).ok_or("invalid subscription profile")?;
    let mut models = if integration == "openai" {
        choices(catalog)?
    } else {
        crate::subscription_catalog::choices(catalog, integration)?
    };
    for model in &mut models {
        model.provider = profile.into();
        if let Some(config) = &mut model.config {
            config.provider = profile.into();
            config.auth_profile = Some(profile.into());
        }
    }
    Ok(models)
}

pub(crate) fn cached_default(
    profile: &str,
    model: &str,
    thinking: Option<&str>,
) -> Result<Option<Config>, String> {
    let profiles = crate::auth::subscription_profiles()?;
    let Some((_, account)) = profiles.iter().find(|(name, _)| name == profile) else {
        return Ok(None);
    };
    let text = std::fs::read_to_string(crate::storage::recorder(&format!("{profile}-models.json")))
        .or_else(|_| std::fs::read_to_string(crate::storage::recorder("openai-models.json")));
    let Ok(text) = text else {
        return Ok(None);
    };
    let cache: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    if cache["account"].as_str() != Some(account) {
        return Ok(None);
    }
    let mut config = profile_choices(&cache["catalog"], profile)?
        .into_iter()
        .find(|choice| choice.id == model)
        .and_then(|choice| choice.config);
    if let Some(config) = &mut config
        && let Some(level) = thinking.filter(|level| config.allows_thinking(level))
    {
        config.thinking = level.into();
    }
    Ok(config)
}

fn choices(catalog: &Value) -> Result<Vec<ModelChoice>, String> {
    let entries = catalog["models"]
        .as_array()
        .ok_or("catalog has no models array")?;
    let mut entries: Vec<_> = entries
        .iter()
        .filter(|entry| entry["visibility"].as_str() == Some("list"))
        .collect();
    entries.sort_by_key(|entry| entry["priority"].as_i64().unwrap_or(i64::MAX));
    let mut models = Vec::new();
    for entry in entries {
        let id = entry["slug"]
            .as_str()
            .filter(|id| !id.trim().is_empty())
            .ok_or("model has no slug")?;
        if models.iter().any(|model: &ModelChoice| model.id == id) {
            return Err("catalog has duplicate model IDs".into());
        }
        let mut levels = Vec::new();
        if let Some(efforts) = entry["supported_reasoning_levels"].as_array() {
            for effort in efforts {
                let level = effort["effort"]
                    .as_str()
                    .filter(|level| !level.is_empty())
                    .ok_or("invalid reasoning level")?;
                if !levels.iter().any(|existing| existing == level) {
                    levels.push(level.to_string());
                }
            }
        }
        if levels.is_empty() {
            levels.push("off".into());
        }
        let thinking = entry["default_reasoning_level"]
            .as_str()
            .unwrap_or(&levels[0]);
        if !levels.iter().any(|level| level == thinking) {
            return Err(format!("invalid default reasoning level for {id}"));
        }
        models.push(ModelChoice {
            provider: PROVIDER.into(),
            alias: entry["display_name"].as_str().map(str::to_string),
            id: id.into(),
            config: Some(Config {
                api_key: String::new(),
                base_url: BASE_URL.into(),
                model: id.into(),
                provider: PROVIDER.into(),
                window: entry["context_window"]
                    .as_u64()
                    .and_then(|n| u32::try_from(n).ok())
                    .filter(|n| *n > 0),
                api: Api::Responses,
                auth_provider: Some("openai".into()),
                auth_profile: None,
                thinking: thinking.into(),
                thinking_levels: levels,
            }),
            error: None,
        });
    }
    if models.is_empty() {
        return Err("catalog has no visible models".into());
    }
    Ok(models)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_maps_subscription_capabilities_and_hides_unlisted_models() {
        let catalog = json!({"models": [
            {"slug": "hidden", "visibility": "hide"},
            {"slug": "second", "visibility": "list", "priority": 2},
            {"slug": "first", "display_name": "First", "visibility": "list", "priority": 1,
             "context_window": 272000, "default_reasoning_level": "high",
             "supported_reasoning_levels": [{"effort": "low"}, {"effort": "high"}]}
        ]});
        let models = choices(&catalog).unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "first");
        let config = models[0].config.as_ref().unwrap();
        assert_eq!(config.api, Api::Responses);
        assert_eq!(config.window, Some(272000));
        assert_eq!(config.thinking, "high");
        assert_eq!(config.thinking_levels, ["low", "high"]);
        assert_eq!(config.auth_provider.as_deref(), Some("openai"));
        assert_eq!(models[1].config.as_ref().unwrap().thinking, "off");
    }

    #[test]
    fn invalid_catalogs_cannot_replace_a_cached_catalog() {
        for catalog in [
            json!({}),
            json!({"models": []}),
            json!({"models": [
                {"slug": "bad", "visibility": "list", "default_reasoning_level": "high",
                 "supported_reasoning_levels": [{"effort": "low"}]}
            ]}),
        ] {
            assert!(choices(&catalog).is_err());
        }
    }
}
