//! Stable subscription account names, including disconnected accounts.
use super::{Credential, store};
use std::collections::BTreeMap;

fn path() -> std::path::PathBuf {
    crate::storage::recorder("subscription-profiles.json")
}

fn names() -> Result<BTreeMap<String, String>, String> {
    match std::fs::read(path()) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).map_err(|e| format!("subscription profiles: {e}"))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(e) => Err(format!("subscription profiles: {e}")),
    }
}

/// Keep a profile's number when its name predates the `openai-subscription-` prefix.
fn rename_legacy(names: &mut BTreeMap<String, String>, name: &str) -> String {
    let Some(number) = name
        .strip_prefix("openai-")
        .filter(|number| number.parse::<u64>().is_ok())
    else {
        return name.into();
    };
    let renamed = format!("openai-subscription-{number}");
    for value in names.values_mut() {
        if value == name {
            *value = renamed.clone();
        }
    }
    renamed
}

pub(super) fn save(integration: &str, credential: Credential) -> Result<(), String> {
    let Credential::OAuth { access, .. } = &credential else {
        return Err("subscription requires OAuth".into());
    };
    let account = account(integration, access)?;
    let mut names = names()?;
    let prefix = format!("{integration}-subscription-");
    let next = names
        .values()
        .filter_map(|name| name.strip_prefix(&prefix)?.parse::<u64>().ok())
        .max()
        .unwrap_or(0)
        + 1;
    let name = names
        .entry(account)
        .or_insert_with(|| format!("{integration}-subscription-{next}"))
        .clone();
    let name = rename_legacy(&mut names, &name);
    let path = path();
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, serde_json::to_vec(&names).unwrap()).map_err(|e| e.to_string())?;
    std::fs::rename(temp, path).map_err(|e| e.to_string())?;
    store::insert(&name, credential)
}

pub(super) fn connected() -> Result<Vec<(String, String)>, String> {
    let mut credentials = store::load(&store::path())?;
    if migrate(&credentials)? {
        credentials = store::load(&store::path())?;
    }
    let mut profiles = Vec::new();
    for (name, credential) in credentials {
        if is_profile(&name)
            && let Credential::OAuth { access, .. } = credential
        {
            let integration = integration(&name).unwrap();
            let identity = names()?
                .into_iter()
                .find(|(_, value)| value == &name)
                .map(|(key, _)| key);
            let identity = match identity {
                Some(identity) => identity,
                None => account(integration, &access)?,
            };
            profiles.push((name, identity));
        }
    }
    Ok(profiles)
}

/// Earlier slices stored one `openai` credential, then `openai-<n>` profiles.
fn migrate(credentials: &BTreeMap<String, Credential>) -> Result<bool, String> {
    let mut migrated = false;
    for (name, credential) in credentials {
        let number = name.strip_prefix("openai-").map(str::to_string);
        let legacy = match &number {
            None if ["openai", "anthropic", "xai"].contains(&name.as_str()) => true,
            Some(number) => number.parse::<u64>().is_ok(),
            None => false,
        };
        if !legacy || !matches!(credential, Credential::OAuth { .. }) {
            continue;
        }
        let integration = if number.is_some() { "openai" } else { name };
        // Refresh legacy credentials before asking the integration for account identity.
        // A disconnected/expired legacy login must not hide other connected profiles.
        if super::resolve_profile(integration, Some(name)).is_err() {
            continue;
        }
        let current = store::load(&store::path())?;
        if save(integration, current.get(name).unwrap_or(credential).clone()).is_err() {
            continue;
        }
        store::remove(name)?;
        if let Some(number) = number {
            let cache = crate::storage::recorder(&format!("openai-{number}-models.json"));
            let renamed =
                crate::storage::recorder(&format!("openai-subscription-{number}-models.json"));
            let _ = std::fs::rename(cache, renamed);
        }
        migrated = true;
    }
    Ok(migrated)
}

pub fn integration(name: &str) -> Option<&str> {
    let (integration, number) = name.split_once("-subscription-")?;
    (["openai", "anthropic", "xai"].contains(&integration)
        && number.parse::<u64>().is_ok_and(|n| n > 0))
    .then_some(integration)
}

pub fn is_profile(name: &str) -> bool {
    integration(name).is_some()
}

pub(super) fn account(integration: &str, access: &str) -> Result<String, String> {
    if integration == "openai" {
        return super::chatgpt_account_id(access);
    }
    let url = match integration {
        "anthropic" => "https://api.anthropic.com/api/oauth/profile",
        "xai" => "https://auth.x.ai/oauth2/userinfo",
        _ => return Err(format!("unknown subscription integration: {integration}")),
    };
    let body = crate::subscription_catalog::get(url, integration, access, None)?;
    let id = match integration {
        "anthropic" => body["account"]["uuid"].as_str(),
        _ => body["sub"].as_str(),
    }
    .filter(|id| !id.is_empty())
    .ok_or_else(|| format!("{integration} account response has no identity"))?;
    Ok(format!("{integration}:{id}"))
}

pub(super) fn credential_name(integration: &str, profile: Option<&str>) -> Result<String, String> {
    if let Some(profile) = profile {
        return Ok(profile.into());
    }
    if ["openai", "anthropic", "xai"].contains(&integration) {
        return Ok(connected()?
            .iter()
            .find(|(name, _)| self::integration(name) == Some(integration))
            .map(|(name, _)| name.clone())
            .unwrap_or_else(|| integration.into()));
    }
    Ok(integration.into())
}
