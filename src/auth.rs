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
const OPENAI_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const OPENAI_DEVICE_USER_CODE_URL: &str =
    "https://auth.openai.com/api/accounts/deviceauth/usercode";
const OPENAI_DEVICE_TOKEN_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/token";
const OPENAI_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const OPENAI_DEVICE_REDIRECT_URI: &str = "https://auth.openai.com/deviceauth/callback";
const OPENAI_VERIFICATION_URI: &str = "https://auth.openai.com/codex/device";
const OPENAI_DEVICE_TIMEOUT_SECS: u64 = 15 * 60;
const OPENAI_JWT_AUTH: &str = "https://api.openai.com/auth";
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
        }) if now_ms() < expires => {
            if provider == "openai" {
                chatgpt_account_id(&access)?;
            }
            Ok(access)
        }
        Some(Credential::OAuth { refresh, .. }) => {
            let fresh = refresh_oauth(provider, &refresh)?;
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
        "xAI",
    )?;
    if !response.0 {
        return Err(failure(
            "xAI",
            "device authorization",
            response.1,
            &response.2,
        ));
    }
    let body = response.2;
    let verification_uri = string(&body, "verification_uri_complete")
        .or_else(|| string(&body, "verification_uri"))
        .ok_or_else(|| "xAI OAuth response has no verification_uri".to_string())?;
    if !verification_uri.starts_with("https://") {
        return Err("xAI returned an untrusted verification URI".into());
    }
    Ok(DeviceCode {
        device_code: required("xAI", &body, "device_code")?,
        user_code: required("xAI", &body, "user_code")?,
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
            "xAI",
        )?;
        if response.0 {
            return token_credential("xAI", &response.2, None);
        }
        match string(&response.2, "error").as_deref() {
            Some("authorization_pending") => {}
            Some("slow_down") => interval = number(&response.2, "interval").unwrap_or(interval + 5),
            Some("access_denied" | "authorization_denied") => {
                return Err("xAI device authorization was denied".into());
            }
            Some("expired_token") => return Err("xAI device code expired".into()),
            _ => {
                return Err(failure(
                    "xAI",
                    "device token polling",
                    response.1,
                    &response.2,
                ));
            }
        }
    }
}

pub fn request_openai_device_code() -> Result<DeviceCode, String> {
    let response = post_json(
        OPENAI_DEVICE_USER_CODE_URL,
        &json!({ "client_id": OPENAI_CLIENT_ID }),
        "OpenAI",
    )?;
    if !response.0 {
        if response.1 == 404 {
            return Err("OpenAI device code login is not enabled".into());
        }
        return Err(failure(
            "OpenAI",
            "device authorization",
            response.1,
            &response.2,
        ));
    }
    let body = response.2;
    let interval = json_interval(&body)
        .ok_or_else(|| format!("OpenAI OAuth response has no interval: {body}"))?;
    Ok(DeviceCode {
        device_code: required("OpenAI", &body, "device_auth_id")?,
        user_code: required("OpenAI", &body, "user_code")?,
        verification_uri: OPENAI_VERIFICATION_URI.into(),
        interval: interval.max(1),
        expires_in: OPENAI_DEVICE_TIMEOUT_SECS,
    })
}

pub fn poll_openai(device: &DeviceCode, cancel: &AtomicBool) -> Result<Credential, String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(device.expires_in);
    let mut interval = device.interval;
    loop {
        sleep_cancel(Duration::from_secs(interval), cancel)?;
        if std::time::Instant::now() >= deadline {
            return Err("OpenAI device code expired".into());
        }
        let response = post_json(
            OPENAI_DEVICE_TOKEN_URL,
            &json!({
                "device_auth_id": device.device_code,
                "user_code": device.user_code,
            }),
            "OpenAI",
        )?;
        if response.0 {
            let authorization_code = required("OpenAI", &response.2, "authorization_code")?;
            let code_verifier = required("OpenAI", &response.2, "code_verifier")?;
            return exchange_openai(&authorization_code, &code_verifier);
        }
        if matches!(response.1, 403 | 404) {
            continue;
        }
        match json_error_code(&response.2).as_deref() {
            Some("deviceauth_authorization_pending") => {}
            Some("slow_down") => interval = json_interval(&response.2).unwrap_or(interval + 5),
            _ => {
                return Err(failure(
                    "OpenAI",
                    "device token polling",
                    response.1,
                    &response.2,
                ));
            }
        }
    }
}

fn exchange_openai(code: &str, verifier: &str) -> Result<Credential, String> {
    let response = post_form(
        OPENAI_TOKEN_URL,
        &[
            ("grant_type", "authorization_code"),
            ("client_id", OPENAI_CLIENT_ID),
            ("code", code),
            ("code_verifier", verifier),
            ("redirect_uri", OPENAI_DEVICE_REDIRECT_URI),
        ],
        "OpenAI",
    )?;
    if !response.0 {
        return Err(failure("OpenAI", "code exchange", response.1, &response.2));
    }
    let credential = token_credential("OpenAI", &response.2, None)?;
    chatgpt_account_id(oauth_access(&credential))?;
    Ok(credential)
}

fn refresh_oauth(provider: &str, refresh: &str) -> Result<Credential, String> {
    match provider {
        "xai" => refresh_xai(refresh),
        "openai" => refresh_openai(refresh),
        _ => Err(format!("unknown auth provider: {provider}")),
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
        "xAI",
    )?;
    if !response.0 {
        return Err(failure("xAI", "token refresh", response.1, &response.2));
    }
    token_credential("xAI", &response.2, Some(refresh))
}

fn refresh_openai(refresh: &str) -> Result<Credential, String> {
    let response = post_form(
        OPENAI_TOKEN_URL,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh),
            ("client_id", OPENAI_CLIENT_ID),
        ],
        "OpenAI",
    )?;
    if !response.0 {
        return Err(failure("OpenAI", "token refresh", response.1, &response.2));
    }
    let credential = token_credential("OpenAI", &response.2, Some(refresh))?;
    chatgpt_account_id(oauth_access(&credential))?;
    Ok(credential)
}

pub fn chatgpt_account_id(access: &str) -> Result<String, String> {
    let payload =
        jwt_payload(access).ok_or_else(|| "OpenAI token has no account id".to_string())?;
    payload
        .get(OPENAI_JWT_AUTH)
        .and_then(|auth| auth.get("chatgpt_account_id"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "OpenAI token has no account id".into())
}

fn jwt_payload(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    let bytes = b64url_decode(payload)?;
    serde_json::from_slice(&bytes).ok()
}

fn b64url_decode(input: &str) -> Option<Vec<u8>> {
    let mut s = input.replace('-', "+").replace('_', "/");
    while !s.len().is_multiple_of(4) {
        s.push('=');
    }
    let mut out = Vec::new();
    let table = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut buf = 0u32;
    let mut bits = 0;
    for b in s.bytes() {
        if b == b'=' {
            break;
        }
        let val = table.iter().position(|&c| c == b)? as u32;
        buf = (buf << 6) | val;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    Some(out)
}

fn oauth_access(credential: &Credential) -> &str {
    match credential {
        Credential::OAuth { access, .. } => access,
        Credential::ApiKey { .. } => unreachable!(),
    }
}

fn token_credential(
    brand: &str,
    body: &Value,
    previous_refresh: Option<&str>,
) -> Result<Credential, String> {
    let access = required(brand, body, "access_token")?;
    let refresh = string(body, "refresh_token")
        .or_else(|| previous_refresh.map(str::to_string))
        .ok_or_else(|| format!("{brand} OAuth response has no refresh_token"))?;
    let lifetime = number(body, "expires_in").unwrap_or(3600) * 1000;
    Ok(Credential::OAuth {
        access,
        refresh,
        expires: now_ms() + lifetime.saturating_sub(REFRESH_SKEW_MS),
    })
}

fn post_form(
    url: &str,
    fields: &[(&str, &str)],
    brand: &str,
) -> Result<(bool, u16, Value), String> {
    let body = fields
        .iter()
        .map(|(k, v)| format!("{}={}", form_encode(k), form_encode(v)))
        .collect::<Vec<_>>()
        .join("&");
    post_body(
        url,
        "application/x-www-form-urlencoded",
        body.into_bytes(),
        brand,
    )
}

fn post_json(url: &str, body: &Value, brand: &str) -> Result<(bool, u16, Value), String> {
    post_body(
        url,
        "application/json",
        serde_json::to_vec(body).unwrap_or_default(),
        brand,
    )
}

fn post_body(
    url: &str,
    content_type: &str,
    body: Vec<u8>,
    brand: &str,
) -> Result<(bool, u16, Value), String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into();
    let mut response = agent
        .post(url)
        .header("Accept", "application/json")
        .header("Content-Type", content_type)
        .send(body)
        .map_err(|err| format!("{brand} OAuth: {err}"))?;
    let status = response.status().as_u16();
    let ok = response.status().is_success();
    let text = response
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("{brand} OAuth response read failed (HTTP {status}): {e}"))?;
    let value = serde_json::from_str::<Value>(&text)
        .map_err(|e| format!("{brand} OAuth returned invalid JSON (HTTP {status}): {e}"))?;
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

fn failure(brand: &str, action: &str, status: u16, body: &Value) -> String {
    let detail = string(body, "error_description")
        .or_else(|| string(body, "error"))
        .or_else(|| json_error_code(body));
    format!(
        "{brand} OAuth {action} failed (HTTP {status}){}",
        detail.map(|d| format!(": {d}")).unwrap_or_default()
    )
}

fn required(brand: &str, body: &Value, key: &str) -> Result<String, String> {
    string(body, key).ok_or_else(|| format!("{brand} OAuth response has no {key}"))
}

fn string(body: &Value, key: &str) -> Option<String> {
    body.get(key)?
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn number(body: &Value, key: &str) -> Option<u64> {
    match body.get(key)? {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn json_interval(body: &Value) -> Option<u64> {
    number(body, "interval")
}

fn json_error_code(body: &Value) -> Option<String> {
    match body.get("error")? {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Object(obj) => obj
            .get("code")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        _ => None,
    }
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

    fn jwt(payload: &Value) -> String {
        let json = serde_json::to_vec(payload).unwrap();
        format!("aaa.{}.ccc", b64url_encode(&json))
    }

    fn b64url_encode(bytes: &[u8]) -> String {
        const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let a = chunk[0] as u32;
            let b = chunk.get(1).copied().unwrap_or(0) as u32;
            let c = chunk.get(2).copied().unwrap_or(0) as u32;
            let n = (a << 16) | (b << 8) | c;
            out.push(TABLE[((n >> 18) & 63) as usize] as char);
            out.push(TABLE[((n >> 12) & 63) as usize] as char);
            if chunk.len() > 1 {
                out.push(TABLE[((n >> 6) & 63) as usize] as char);
            }
            if chunk.len() > 2 {
                out.push(TABLE[(n & 63) as usize] as char);
            }
        }
        out.replace('+', "-").replace('/', "_")
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
            "openai".into(),
            Credential::OAuth {
                access: "a".into(),
                refresh: "r".into(),
                expires: 42,
            },
        );
        save_path(&path, &values).unwrap();
        assert_eq!(load_path(&path).unwrap(), values);
    }

    #[test]
    fn chatgpt_account_id_from_jwt() {
        let token = jwt(&json!({
            "https://api.openai.com/auth": { "chatgpt_account_id": "acct_1" }
        }));
        assert_eq!(chatgpt_account_id(&token).unwrap(), "acct_1");
        assert!(chatgpt_account_id("not-a-jwt").is_err());
        let empty = jwt(&json!({ "https://api.openai.com/auth": { "chatgpt_account_id": "" } }));
        assert!(chatgpt_account_id(&empty).is_err());
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
