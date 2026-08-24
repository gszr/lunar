//! Resolve guest definitions into the model catalog and live configuration.

use std::collections::BTreeMap;
use std::process::Command;

use crate::protocol::{self, Api, Config, Thinking};

use super::guest::{Guest, Listed, ModelDef, ProviderDef, RawDefaults};
use super::{Loaded, ModelChoice};

pub(super) fn loaded(guest: &Guest) -> Loaded {
    let Some(defaults) = &guest.defaults else {
        return Loaded {
            config: None,
            models: choices(guest),
            notice: None,
        };
    };
    match config_from_lua(guest, defaults) {
        Ok((config, extra)) => {
            let notice = join_notices(
                guest
                    .model_notices
                    .iter()
                    .chain(guest.provider_notices.iter())
                    .chain(extra.iter()),
            );
            Loaded {
                config: Some(config),
                models: choices(guest),
                notice,
            }
        }
        Err(notice) => {
            let combined = join_notices(
                guest
                    .model_notices
                    .iter()
                    .chain(guest.provider_notices.iter())
                    .chain(std::iter::once(&notice)),
            );
            Loaded {
                config: None,
                models: choices(guest),
                notice: combined,
            }
        }
    }
}

fn choices(guest: &Guest) -> Vec<ModelChoice> {
    let mut out = Vec::new();
    for (provider_key, provider) in &guest.providers {
        let (models, _) = resolve_listed(&guest.models, &provider.models);
        for model in models {
            let result = provider_config(provider_key, provider, &model);
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

fn provider_config(
    provider_key: &str,
    provider: &ProviderDef,
    model: &ResolvedModel,
) -> Result<Config, String> {
    let auth_provider = match provider.key_in.as_str() {
        "env" => None,
        "auth" => {
            let auth_provider = provider
                .auth_provider
                .as_deref()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| format!("{provider_key} has no auth_provider"))?;
            if !matches!(auth_provider, "xai" | "openai") {
                return Err(format!(
                    "{provider_key} has unknown auth_provider: {auth_provider}"
                ));
            }
            Some(auth_provider.to_string())
        }
        _ => return Err(format!("{provider_key} key_in is not env or auth")),
    };
    let base_url = match provider.base_url_cmd.as_deref().filter(|s| !s.is_empty()) {
        Some(command) => command_value(provider_key, "base_url_cmd", command)?,
        None => provider
            .base_url
            .as_deref()
            .filter(|s| !s.is_empty())
            .or_else(|| default_auth_base(auth_provider.as_deref()))
            .ok_or_else(|| format!("{provider_key} has no base_url or base_url_cmd"))?
            .to_string(),
    };
    if model.api == Api::Messages
        || (auth_provider.as_deref() == Some("openai") && model.api != Api::Responses)
    {
        let name = model.alias.as_deref().unwrap_or(model.id.as_str());
        return Err(format!(
            "{name} uses {}, not implemented",
            model.api.as_str()
        ));
    }
    let api_key = match provider.key_in.as_str() {
        "env" => match provider.key_cmd.as_deref().filter(|s| !s.is_empty()) {
            Some(command) => command_value(provider_key, "key_cmd", command)?,
            None => {
                let key_name = provider
                    .key_name
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| format!("{provider_key} has no key_name or key_cmd"))?;
                nonempty(key_name).ok_or_else(|| format!("missing {key_name}"))?
            }
        },
        "auth" => crate::auth::resolve(auth_provider.as_deref().unwrap())?,
        _ => unreachable!(),
    };
    Ok(Config {
        api_key,
        base_url,
        model: model.id.clone(),
        provider: provider_key.to_string(),
        window: model.window.or_else(|| protocol::guess_window(&model.id)),
        api: model.api,
        auth_provider,
        thinking: model.thinking.or(provider.thinking).unwrap_or_default(),
    })
}

fn config_from_lua(guest: &Guest, defaults: &RawDefaults) -> Result<(Config, Vec<String>), String> {
    let provider_key = defaults
        .provider
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "defaults needs provider and model".to_string())?;
    let model_key = defaults
        .model
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "defaults needs provider and model".to_string())?;
    let provider = guest
        .providers
        .get(provider_key)
        .ok_or_else(|| format!("unknown provider: {provider_key}"))?;
    let (listed, skips) = resolve_listed(&guest.models, &provider.models);
    let chosen = match pick_model(&listed, model_key) {
        Some(model) => model,
        None => return Err(join_parts(skips, format!("unknown model: {model_key}"))),
    };
    match provider_config(provider_key, provider, chosen) {
        Ok(config) => Ok((config, skips)),
        Err(err) => Err(join_parts(skips, err)),
    }
}

struct ResolvedModel {
    alias: Option<String>,
    id: String,
    window: Option<u32>,
    api: Api,
    thinking: Option<Thinking>,
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
                    thinking: def.thinking,
                }),
                None => notices.push(format!("unknown alias: {alias}")),
            },
            Listed::Local(def) => out.push(ResolvedModel {
                alias: None,
                id: def.id.clone(),
                window: def.window,
                api: def.api,
                thinking: def.thinking,
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

fn join_notices<'a>(parts: impl Iterator<Item = &'a String>) -> Option<String> {
    let text = parts.cloned().collect::<Vec<_>>().join("\n");
    if text.is_empty() { None } else { Some(text) }
}

fn join_parts(mut parts: Vec<String>, last: String) -> String {
    parts.push(last);
    parts.join("\n")
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
        _ => None,
    }
}
