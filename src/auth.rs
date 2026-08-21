//! Lunar-managed credentials in `$LUNAR_HOME/auth.json`.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

const XAI_CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";
const XAI_SCOPE: &str = "openid profile email offline_access grok-cli:access api:access";
const XAI_DEVICE_CODE_URL: &str = "https://auth.x.ai/oauth2/device/code";
const XAI_TOKEN_URL: &str = "https://auth.x.ai/oauth2/token";
const REFRESH_SKEW_MS: u64 = 5 * 60 * 1000;

#[derive(Clone, Debug, PartialEq)]
pub enum Credential {
    ApiKey {
        key: String,
    },
    OAuth {
        access: String,
        refresh: String,
        expires: u64,
    },
}

pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub interval: u64,
    pub expires_in: u64,
}

pub fn path() -> PathBuf {
    crate::mission::home().join("auth.json")
}

pub fn resolve(provider: &str) -> Result<String, String> {
    let path = path();
    let mut credentials = load_path(&path)?;
    match credentials.remove(provider) {
        Some(Credential::ApiKey { key }) => Ok(key),
        Some(Credential::OAuth {
            access,
            refresh: _,
            expires,
        }) if now_ms() < expires => Ok(access),
        Some(Credential::OAuth { refresh, .. }) => {
            let fresh = refresh_xai(&refresh)?;
            let access = match &fresh {
                Credential::OAuth { access, .. } => access.clone(),
                Credential::ApiKey { .. } => unreachable!(),
            };
            credentials.insert(provider.to_string(), fresh);
            save_path(&path, &credentials).map_err(|e| format!("auth.json: {e}"))?;
            Ok(access)
        }
        None => Err(format!("no auth for {provider}; run /login {provider}")),
    }
}

pub fn save_api_key(provider: &str, key: &str) -> Result<(), String> {
    if key.trim().is_empty() {
        return Err("API key is empty".into());
    }
    let path = path();
    let mut credentials = load_path(&path)?;
    credentials.insert(
        provider.into(),
        Credential::ApiKey {
            key: key.trim().into(),
        },
    );
    save_path(&path, &credentials).map_err(|e| format!("auth.json: {e}"))
}

pub fn save_oauth(provider: &str, credential: Credential) -> Result<(), String> {
    let path = path();
    let mut credentials = load_path(&path)?;
    credentials.insert(provider.into(), credential);
    save_path(&path, &credentials).map_err(|e| format!("auth.json: {e}"))
}

pub fn logout(provider: &str) -> Result<bool, String> {
    let path = path();
    let mut credentials = load_path(&path)?;
    let removed = credentials.remove(provider).is_some();
    if removed {
        save_path(&path, &credentials).map_err(|e| format!("auth.json: {e}"))?;
    }
    Ok(removed)
}

pub fn request_xai_device_code() -> Result<DeviceCode, String> {
    let response = post_form(
        XAI_DEVICE_CODE_URL,
        &[
            ("client_id", XAI_CLIENT_ID),
            ("scope", XAI_SCOPE),
            ("referrer", "lunar"),
        ],
    )?;
    if !response.0 {
        return Err(failure("device authorization", response.1, &response.2));
    }
    let body = response.2;
    let verification_uri = string(&body, "verification_uri_complete")
        .or_else(|| string(&body, "verification_uri"))
        .ok_or_else(|| "xAI OAuth response has no verification_uri".to_string())?;
    if !verification_uri.starts_with("https://") {
        return Err("xAI returned an untrusted verification URI".into());
    }
    Ok(DeviceCode {
        device_code: required(&body, "device_code")?,
        user_code: required(&body, "user_code")?,
        verification_uri,
        interval: number(&body, "interval").unwrap_or(5).max(1),
        expires_in: number(&body, "expires_in").unwrap_or(900),
    })
}

pub fn poll_xai(device: &DeviceCode, cancel: &AtomicBool) -> Result<Credential, String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(device.expires_in);
    let mut interval = device.interval;
    loop {
        sleep_cancel(Duration::from_secs(interval), cancel)?;
        if std::time::Instant::now() >= deadline {
            return Err("xAI device code expired".into());
        }
        let response = post_form(
            XAI_TOKEN_URL,
            &[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("client_id", XAI_CLIENT_ID),
                ("device_code", &device.device_code),
            ],
        )?;
        if response.0 {
            return token_credential(&response.2, None);
        }
        match string(&response.2, "error").as_deref() {
            Some("authorization_pending") => {}
            Some("slow_down") => interval = number(&response.2, "interval").unwrap_or(interval + 5),
            Some("access_denied" | "authorization_denied") => {
                return Err("xAI device authorization was denied".into());
            }
            Some("expired_token") => return Err("xAI device code expired".into()),
            _ => return Err(failure("device token polling", response.1, &response.2)),
        }
    }
}

fn refresh_xai(refresh: &str) -> Result<Credential, String> {
    let response = post_form(
        XAI_TOKEN_URL,
        &[
            ("grant_type", "refresh_token"),
            ("client_id", XAI_CLIENT_ID),
            ("refresh_token", refresh),
        ],
    )?;
    if !response.0 {
        return Err(failure("token refresh", response.1, &response.2));
    }
    token_credential(&response.2, Some(refresh))
}

fn token_credential(body: &Value, previous_refresh: Option<&str>) -> Result<Credential, String> {
    let access = required(body, "access_token")?;
    let refresh = string(body, "refresh_token")
        .or_else(|| previous_refresh.map(str::to_string))
        .ok_or_else(|| "xAI OAuth response has no refresh_token".to_string())?;
    let lifetime = number(body, "expires_in").unwrap_or(3600) * 1000;
    Ok(Credential::OAuth {
        access,
        refresh,
        expires: now_ms() + lifetime.saturating_sub(REFRESH_SKEW_MS),
    })
}

fn post_form(url: &str, fields: &[(&str, &str)]) -> Result<(bool, u16, Value), String> {
    let body = fields
        .iter()
        .map(|(k, v)| format!("{}={}", form_encode(k), form_encode(v)))
        .collect::<Vec<_>>()
        .join("&");
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into();
    let mut response = agent
        .post(url)
        .header("Accept", "application/json")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .send(body)
        .map_err(|err| format!("xAI OAuth: {err}"))?;
    let status = response.status().as_u16();
    let ok = response.status().is_success();
    let text = response
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("xAI OAuth response read failed (HTTP {status}): {e}"))?;
    let value = serde_json::from_str::<Value>(&text)
        .map_err(|e| format!("xAI OAuth returned invalid JSON (HTTP {status}): {e}"))?;
    Ok((ok, status, value))
}

fn form_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn sleep_cancel(duration: Duration, cancel: &AtomicBool) -> Result<(), String> {
    let end = std::time::Instant::now() + duration;
    while std::time::Instant::now() < end {
        if cancel.load(Ordering::Relaxed) {
            return Err("login cancelled".into());
        }
        std::thread::sleep(
            Duration::from_millis(100)
                .min(end.saturating_duration_since(std::time::Instant::now())),
        );
    }
    Ok(())
}

fn failure(action: &str, status: u16, body: &Value) -> String {
    let detail = string(body, "error_description").or_else(|| string(body, "error"));
    format!(
        "xAI OAuth {action} failed (HTTP {status}){}",
        detail.map(|d| format!(": {d}")).unwrap_or_default()
    )
}

fn required(body: &Value, key: &str) -> Result<String, String> {
    string(body, key).ok_or_else(|| format!("xAI OAuth response has no {key}"))
}

fn string(body: &Value, key: &str) -> Option<String> {
    body.get(key)?
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn number(body: &Value, key: &str) -> Option<u64> {
    body.get(key)?.as_u64()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn load_path(path: &Path) -> Result<BTreeMap<String, Credential>, String> {
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

fn save_path(path: &Path, credentials: &BTreeMap<String, Credential>) -> io::Result<()> {
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
            now_ms(),
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
        save_path(&path, &values).unwrap();
        assert_eq!(load_path(&path).unwrap(), values);
        values.insert(
            "xai".into(),
            Credential::OAuth {
                access: "a".into(),
                refresh: "r".into(),
                expires: 42,
            },
        );
        save_path(&path, &values).unwrap();
        assert_eq!(load_path(&path).unwrap(), values);
    }

    #[cfg(unix)]
    #[test]
    fn file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let path = scratch().join("auth.json");
        save_path(&path, &BTreeMap::new()).unwrap();
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
