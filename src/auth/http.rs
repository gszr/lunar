//! Shared OAuth HTTP and response parsing.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;

use super::Credential;

const REFRESH_SKEW_MS: u64 = 5 * 60 * 1000;

pub(super) fn post_form(
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

pub(super) fn post_json(
    url: &str,
    body: &Value,
    brand: &str,
) -> Result<(bool, u16, Value), String> {
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

pub(super) fn sleep_cancel(duration: Duration, cancel: &AtomicBool) -> Result<(), String> {
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

pub(super) fn failure(brand: &str, action: &str, status: u16, body: &Value) -> String {
    let detail = string(body, "error_description")
        .or_else(|| string(body, "error"))
        .or_else(|| json_error_code(body));
    format!(
        "{brand} OAuth {action} failed (HTTP {status}){}",
        detail.map(|d| format!(": {d}")).unwrap_or_default()
    )
}

pub(super) fn required(brand: &str, body: &Value, key: &str) -> Result<String, String> {
    string(body, key).ok_or_else(|| format!("{brand} OAuth response has no {key}"))
}

pub(super) fn string(body: &Value, key: &str) -> Option<String> {
    body.get(key)?
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub(super) fn number(body: &Value, key: &str) -> Option<u64> {
    match body.get(key)? {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

pub(super) fn json_error_code(body: &Value) -> Option<String> {
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

pub(super) fn token_credential(
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

pub(super) fn oauth_access(credential: &Credential) -> &str {
    match credential {
        Credential::OAuth { access, .. } => access,
        Credential::ApiKey { .. } => unreachable!(),
    }
}

pub(super) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
