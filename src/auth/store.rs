//! Credential persistence and auth.json file permissions.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::Credential;

pub(super) fn path() -> PathBuf {
    crate::storage::recorder("auth.json")
}

pub(super) fn insert(provider: &str, credential: Credential) -> Result<(), String> {
    let path = path();
    let mut credentials = load(&path)?;
    credentials.insert(provider.into(), credential);
    save(&path, &credentials).map_err(|e| format!("auth.json: {e}"))
}

pub(super) fn remove(provider: &str) -> Result<bool, String> {
    let path = path();
    let mut credentials = load(&path)?;
    let removed = credentials.remove(provider).is_some();
    if removed {
        save(&path, &credentials).map_err(|e| format!("auth.json: {e}"))?;
    }
    Ok(removed)
}

pub(super) fn load(path: &Path) -> Result<BTreeMap<String, Credential>, String> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(err) => return Err(format!("auth.json: {err}")),
    };
    let root: Value = serde_json::from_str(&text).map_err(|e| format!("auth.json: {e}"))?;
    let object = root
        .as_object()
        .ok_or_else(|| "auth.json: expected object".to_string())?;
    let mut out = BTreeMap::new();
    for (provider, value) in object {
        let kind = value.get("type").and_then(Value::as_str).unwrap_or("");
        let credential = match kind {
            "api_key" => Credential::ApiKey {
                key: value
                    .get("key")
                    .and_then(Value::as_str)
                    .ok_or_else(|| format!("auth.json: {provider} has no key"))?
                    .into(),
            },
            "oauth" => Credential::OAuth {
                access: value
                    .get("access")
                    .and_then(Value::as_str)
                    .ok_or_else(|| format!("auth.json: {provider} has no access"))?
                    .into(),
                refresh: value
                    .get("refresh")
                    .and_then(Value::as_str)
                    .ok_or_else(|| format!("auth.json: {provider} has no refresh"))?
                    .into(),
                expires: value
                    .get("expires")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| format!("auth.json: {provider} has no expires"))?,
            },
            _ => return Err(format!("auth.json: {provider} has unknown credential type")),
        };
        out.insert(provider.clone(), credential);
    }
    Ok(out)
}

pub(super) fn save(path: &Path, credentials: &BTreeMap<String, Credential>) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let value: serde_json::Map<String, Value> = credentials
        .iter()
        .map(|(provider, credential)| {
            let value = match credential {
                Credential::ApiKey { key } => json!({"type":"api_key", "key":key}),
                Credential::OAuth {
                    access,
                    refresh,
                    expires,
                } => json!({"type":"oauth", "access":access, "refresh":refresh, "expires":expires}),
            };
            (provider.clone(), value)
        })
        .collect();
    let temp = path.with_extension(format!("json.tmp-{}", std::process::id()));
    fs::write(&temp, serde_json::to_vec_pretty(&Value::Object(value))?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temp, fs::Permissions::from_mode(0o600))?;
    }
    fs::rename(temp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "lunar-auth-{}-{}-{:?}",
            std::process::id(),
            crate::auth::http::now_ms(),
            std::thread::current().id()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn round_trip_both_types() {
        let path = scratch().join("auth.json");
        let mut values = BTreeMap::new();
        values.insert(
            "xai".into(),
            Credential::ApiKey {
                key: "xai-test".into(),
            },
        );
        save(&path, &values).unwrap();
        assert_eq!(load(&path).unwrap(), values);
        values.insert(
            "openai".into(),
            Credential::OAuth {
                access: "a".into(),
                refresh: "r".into(),
                expires: 42,
            },
        );
        save(&path, &values).unwrap();
        assert_eq!(load(&path).unwrap(), values);
    }

    #[cfg(unix)]
    #[test]
    fn file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let path = scratch().join("auth.json");
        save(&path, &BTreeMap::new()).unwrap();
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
