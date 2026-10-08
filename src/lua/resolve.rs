//! Resolve guest definitions into the model catalog and live configuration.

use std::collections::BTreeMap;
use std::process::Command;

use crate::protocol::{self, Api, Config};

use super::guest::{Guest, Listed, ModelDef, ProviderDef, RawDefaults};
use super::{Loaded, ModelChoice, diagnostic};

pub(super) fn loaded(guest: &Guest) -> Result<Loaded, String> {
    // Validate references before running any credential helpers.
    for (name, provider) in &guest.providers {
        let (models, errors) = resolve_listed(&guest.models, &provider.models);
        if !errors.is_empty() {
            return Err(diagnostic::config(
                &provider.source,
                Some(&format!("providers.{name}.models")),
                &errors.join("\n"),
            ));
        }
        for model in &models {
            validate_api(provider, model).map_err(|err| {
                diagnostic::config(
                    &provider.source,
                    Some(&format!("providers.{name}.models")),
                    &err,
                )
            })?;
        }
    }
    let selected = guest
        .defaults
        .as_ref()
        .map(|defaults| selected_model(guest, defaults))
        .transpose()?;
    let mut providers = BTreeMap::new();
    for (name, provider) in &guest.providers {
        let resolved = resolve_provider(name, provider).map_err(|err| {
            diagnostic::config(&provider.source, Some(&format!("providers.{name}")), &err)
        })?;
        providers.insert(name.clone(), resolved);
    }
    let models = choices(guest, &providers);
    let (config, notice) = match selected {
        Some((provider, model)) => {
            let choice = models
                .iter()
                .find(|choice| {
                    choice.provider == provider
                        && choice.id == model.id
                        && choice.alias == model.alias
                })
                .unwrap();
            (choice.config.clone(), choice.error.clone())
        }
        None => (None, None),
    };
    Ok(Loaded {
        config,
        models,
        stack: crate::stack::render(&guest.stack),
        notice,
    })
}

fn choices(guest: &Guest, providers: &BTreeMap<String, ResolvedProvider>) -> Vec<ModelChoice> {
    let mut out = Vec::new();
    for (provider_key, provider) in &guest.providers {
        let (models, _) = resolve_listed(&guest.models, &provider.models);
        for model in models {
            let result = provider_config(
                provider_key,
                &model,
                guest
                    .defaults
                    .as_ref()
                    .and_then(|defaults| defaults.thinking.as_deref()),
                providers,
            );
            let (config, error) = match result {
                Ok(config) => (Some(config), None),
                Err(error) => (None, Some(error)),
            };
            out.push(ModelChoice {
                provider: provider_key.clone(),
                alias: model.alias,
                id: model.id,
                config,
                error,
            });
        }
    }
    out
}

#[derive(Clone)]
struct ResolvedProvider {
    api_key: Result<String, String>,
    base_url: String,
    auth_provider: Option<String>,
}

fn provider_config(
    provider_key: &str,
    model: &ResolvedModel,
    default_thinking: Option<&str>,
    providers: &BTreeMap<String, ResolvedProvider>,
) -> Result<Config, String> {
    let provider = &providers[provider_key];
    Ok(Config {
        api_key: provider.api_key.clone()?,
        base_url: provider.base_url.clone(),
        model: model.id.clone(),
        provider: provider_key.to_string(),
        window: model.window.or_else(|| protocol::guess_window(&model.id)),
        api: model.api,
        auth_provider: provider.auth_provider.clone(),
        thinking: default_thinking
            .filter(|level| model.thinking_levels.iter().any(|allowed| allowed == level))
            .unwrap_or(&model.thinking)
            .to_string(),
        thinking_levels: model.thinking_levels.clone(),
    })
}

fn validate_api(provider: &ProviderDef, model: &ResolvedModel) -> Result<(), String> {
    if provider.key_in == "auth" {
        match (provider.auth_provider.as_deref(), model.api) {
            (Some("openai"), api) if api != Api::Responses => {
                let name = model.alias.as_deref().unwrap_or(model.id.as_str());
                return Err(format!("{name} uses {}, not implemented", api.as_str()));
            }
            (Some("anthropic"), api) if api != Api::Messages => {
                let name = model.alias.as_deref().unwrap_or(model.id.as_str());
                return Err(format!(
                    "{name} uses {}, Anthropic subscription auth requires messages",
                    api.as_str()
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

fn resolve_provider(
    provider_key: &str,
    provider: &ProviderDef,
) -> Result<ResolvedProvider, String> {
    let auth_provider = match provider.key_in.as_str() {
        "env" | "none" => None,
        "auth" => {
            let auth_provider = provider
                .auth_provider
                .as_deref()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| format!("{provider_key} has no auth_provider"))?;
            if !matches!(auth_provider, "xai" | "openai" | "anthropic") {
                return Err(format!(
                    "{provider_key} has unknown auth_provider: {auth_provider}"
                ));
            }
            Some(auth_provider.to_string())
        }
        _ => return Err(format!("{provider_key} key_in is not env, auth, or none")),
    };
    let base_url = if provider.key_in == "none" {
        provider
            .base_url
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| format!("{provider_key} has no base_url"))?
            .to_string()
    } else {
        match provider.base_url_cmd.as_deref().filter(|s| !s.is_empty()) {
            Some(command) => command_value(provider_key, "base_url_cmd", command)?,
            None => provider
                .base_url
                .as_deref()
                .filter(|s| !s.is_empty())
                .or_else(|| default_auth_base(auth_provider.as_deref()))
                .ok_or_else(|| format!("{provider_key} has no base_url or base_url_cmd"))?
                .to_string(),
        }
    };
    let api_key = match provider.key_in.as_str() {
        "env" => match provider.key_cmd.as_deref().filter(|s| !s.is_empty()) {
            Some(command) => command_value(provider_key, "key_cmd", command),
            None => {
                let key_name = provider
                    .key_name
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| format!("{provider_key} has no key_name or key_cmd"))?;
                nonempty(key_name).ok_or_else(|| format!("missing {key_name}"))
            }
        },
        "auth" => crate::auth::resolve(auth_provider.as_deref().unwrap()),
        "none" => Ok(String::new()),
        _ => unreachable!(),
    };
    Ok(ResolvedProvider {
        api_key,
        base_url,
        auth_provider,
    })
}

fn selected_model<'a>(
    guest: &Guest,
    defaults: &'a RawDefaults,
) -> Result<(&'a str, ResolvedModel), String> {
    let provider_key = defaults.provider.as_str();
    let model_key = defaults.model.as_str();
    let provider = guest.providers.get(provider_key).ok_or_else(|| {
        diagnostic::config(
            &defaults.source,
            Some("defaults.provider"),
            &format!(
                "unknown provider: {provider_key}\nAvailable providers: {}",
                available(guest.providers.keys().cloned())
            ),
        )
    })?;
    let (listed, _) = resolve_listed(&guest.models, &provider.models);
    let chosen = pick_model(&listed, model_key).ok_or_else(|| {
        diagnostic::config(
            &defaults.source,
            Some("defaults.model"),
            &format!(
                "unknown model: {model_key}\nAvailable models for {provider_key}: {}",
                available(listed.iter().map(|model| {
                    match &model.alias {
                        Some(alias) if alias != &model.id => format!("{alias} ({})", model.id),
                        _ => model.id.clone(),
                    }
                }))
            ),
        )
    })?;
    Ok((provider_key, chosen.clone()))
}

#[derive(Clone)]
struct ResolvedModel {
    alias: Option<String>,
    id: String,
    window: Option<u32>,
    api: Api,
    thinking: String,
    thinking_levels: Vec<String>,
}

fn resolve_listed(
    catalog: &BTreeMap<String, ModelDef>,
    listed: &[Listed],
) -> (Vec<ResolvedModel>, Vec<String>) {
    let mut out = Vec::new();
    let mut notices = Vec::new();
    for item in listed {
        match item {
            Listed::Alias(alias) => match catalog.get(alias) {
                Some(def) => out.push(ResolvedModel {
                    alias: Some(alias.clone()),
                    id: def.id.clone(),
                    window: def.window,
                    api: def.api,
                    thinking: def.thinking.default.clone(),
                    thinking_levels: def.thinking.levels.clone(),
                }),
                None => notices.push(format!("unknown alias: {alias}")),
            },
            Listed::Local(def) => out.push(ResolvedModel {
                alias: None,
                id: def.id.clone(),
                window: def.window,
                api: def.api,
                thinking: def.thinking.default.clone(),
                thinking_levels: def.thinking.levels.clone(),
            }),
        }
    }
    (out, notices)
}

fn pick_model<'a>(listed: &'a [ResolvedModel], key: &str) -> Option<&'a ResolvedModel> {
    listed
        .iter()
        .find(|m| m.alias.as_deref() == Some(key))
        .or_else(|| listed.iter().find(|m| m.id == key))
}

fn nonempty(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|s| !s.is_empty())
}

fn command_value(provider: &str, field: &str, command: &str) -> Result<String, String> {
    let output = Command::new("sh")
        .arg("-c")
        .arg(command)
        .output()
        .map_err(|err| format!("{provider} {field}: {err}"))?;
    if !output.status.success() {
        return Err(format!("{provider} {field} failed with {}", output.status));
    }
    let value = String::from_utf8(output.stdout)
        .map_err(|_| format!("{provider} {field} output is not UTF-8"))?
        .trim_end()
        .to_string();
    if value.is_empty() {
        Err(format!("{provider} {field} returned an empty value"))
    } else {
        Ok(value)
    }
}

pub(super) fn default_auth_base(auth_provider: Option<&str>) -> Option<&'static str> {
    match auth_provider {
        Some("xai") => Some("https://api.x.ai/v1"),
        Some("openai") => Some("https://chatgpt.com/backend-api"),
        Some("anthropic") => Some("https://api.anthropic.com"),
        _ => None,
    }
}

fn available(values: impl Iterator<Item = String>) -> String {
    let values = values.collect::<Vec<_>>().join(", ");
    if values.is_empty() {
        "(none configured)".into()
    } else {
        values
    }
}
