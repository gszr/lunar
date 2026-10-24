//! Provider wire formats for subscription discovery; lifecycle lives in subscription.
use crate::{
    lua::ModelChoice,
    protocol::{Api, Config},
};
use serde_json::{Value, json};

pub(crate) fn get(
    url: &str,
    integration: &str,
    access: &str,
    account: Option<&str>,
) -> Result<Value, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(20)))
        .http_status_as_error(false)
        .build()
        .into();
    let mut request = agent
        .get(url)
        .header("Authorization", &format!("Bearer {access}"));
    match integration {
        "anthropic" => {
            request = request
                .header("anthropic-version", "2023-06-01")
                .header("anthropic-beta", "oauth-2025-04-20")
                .header("user-agent", "claude-cli/2.1.280");
        }
        "openai" => {
            request = request
                .header("chatgpt-account-id", account.unwrap_or(""))
                .header("originator", "lunar");
        }
        _ => {}
    }
    let mut response = request.call().map_err(|e| format!("{integration}: {e}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "{integration} discovery HTTP {}",
            response.status().as_u16()
        ));
    }
    let text = response
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("{integration} discovery: {e}"))?;
    serde_json::from_str(&text).map_err(|e| format!("{integration} discovery: {e}"))
}

pub(crate) fn fetch(integration: &str, access: &str) -> Result<Value, String> {
    let base = match integration {
        "anthropic" => "https://api.anthropic.com/v1/models",
        "xai" => "https://api.x.ai/v1/models",
        _ => return Err(format!("unknown integration: {integration}")),
    };
    let mut entries = Vec::new();
    let mut url = base.to_string();
    loop {
        let page = get(&url, integration, access, None)?;
        entries.extend(
            page["data"]
                .as_array()
                .ok_or("catalog has no data array")?
                .iter()
                .cloned(),
        );
        if page["has_more"].as_bool() != Some(true) {
            break;
        }
        let last = page["last_id"]
            .as_str()
            .ok_or("catalog has_more without last_id")?;
        let next = format!("{base}?after_id={last}");
        if next == url {
            return Err("catalog pagination did not advance".into());
        }
        url = next;
    }
    Ok(json!({"data": entries}))
}

pub(crate) fn choices(catalog: &Value, integration: &str) -> Result<Vec<ModelChoice>, String> {
    let entries = catalog["data"]
        .as_array()
        .ok_or("catalog has no data array")?;
    let mut models = Vec::new();
    for entry in entries {
        let id = entry["id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or("model has no id")?;
        if models.iter().any(|model: &ModelChoice| model.id == id) {
            return Err("catalog has duplicate model IDs".into());
        }
        let mut levels = vec!["off".to_string()];
        let capabilities = &entry["capabilities"];
        if capabilities["thinking"]["types"]["adaptive"]["supported"].as_bool() == Some(true) {
            for level in ["low", "medium", "high", "xhigh", "max"] {
                if capabilities["effort"][level]["supported"].as_bool() == Some(true) {
                    levels.push(level.into());
                }
            }
        }
        let (base_url, api) = match integration {
            "anthropic" => ("https://api.anthropic.com", Api::Messages),
            "xai" => ("https://api.x.ai/v1", Api::Completions),
            _ => return Err(format!("unknown integration: {integration}")),
        };
        models.push(ModelChoice {
            provider: integration.into(),
            alias: entry["display_name"].as_str().map(str::to_string),
            id: id.into(),
            error: None,
            config: Some(Config {
                api_key: String::new(),
                base_url: base_url.into(),
                model: id.into(),
                provider: integration.into(),
                window: entry["max_input_tokens"]
                    .as_u64()
                    .or_else(|| entry["context_window"].as_u64())
                    .or_else(|| entry["context_length"].as_u64())
                    .and_then(|n| u32::try_from(n).ok())
                    .filter(|n| *n > 0),
                api,
                auth_provider: Some(integration.into()),
                auth_profile: None,
                thinking: "off".into(),
                thinking_levels: levels,
            }),
        });
    }
    if models.is_empty() {
        return Err("catalog has no models".into());
    }
    Ok(models)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogs_keep_wire_capabilities_without_guessing() {
        for (integration, catalog, api, window, levels) in [
            (
                "anthropic",
                json!({"data": [{"id": "claude-test", "max_input_tokens": 200000,
                "capabilities": {"thinking": {"types": {"adaptive": {"supported": true}}},
                    "effort": {"high": {"supported": true}}}}]}),
                Api::Messages,
                200000,
                vec!["off", "high"],
            ),
            (
                "xai",
                json!({"data": [{"id": "grok-test", "context_length": 131072}]}),
                Api::Completions,
                131072,
                vec!["off"],
            ),
        ] {
            let models = choices(&catalog, integration).unwrap();
            let config = models[0].config.as_ref().unwrap();
            assert_eq!(config.api, api);
            assert_eq!(config.auth_provider.as_deref(), Some(integration));
            assert_eq!(config.window, Some(window));
            assert_eq!(config.thinking_levels, levels);
        }
        assert!(choices(&json!({"data": []}), "xai").is_err());
        assert!(choices(&json!({"data": [{"id": "a"}, {"id": "a"}]}), "anthropic").is_err());
    }
}
