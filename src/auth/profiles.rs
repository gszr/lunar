//! Stable OpenAI account names, including disconnected accounts.
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

pub(super) fn save(credential: Credential) -> Result<(), String> {
    let Credential::OAuth { access, .. } = &credential else {
        return Err("OpenAI subscription requires OAuth".into());
    };
    let account = super::chatgpt_account_id(access)?;
    let mut names = names()?;
    let next = names
        .values()
        .filter_map(|name| {
            name.strip_prefix("openai-subscription-")?
                .parse::<u64>()
                .ok()
        })
        .max()
        .unwrap_or(0)
        + 1;
    let name = names
        .entry(account)
        .or_insert_with(|| format!("openai-subscription-{next}"))
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
            profiles.push((name, super::chatgpt_account_id(&access)?));
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
            None if name == "openai" => true,
            Some(number) => number.parse::<u64>().is_ok(),
            None => false,
        };
        if !legacy || !matches!(credential, Credential::OAuth { .. }) {
            continue;
        }
        save(credential.clone())?;
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

pub fn is_profile(name: &str) -> bool {
    name.strip_prefix("openai-subscription-")
        .is_some_and(|n| n.parse::<u64>().is_ok_and(|n| n > 0))
}

pub(super) fn credential_name(integration: &str, profile: Option<&str>) -> Result<String, String> {
    if let Some(profile) = profile {
        return Ok(profile.into());
    }
    if integration == "openai" {
        return Ok(connected()?
            .first()
            .map(|(name, _)| name.clone())
            .unwrap_or_else(|| integration.into()));
    }
    Ok(integration.into())
}
