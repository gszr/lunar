//! OpenAI device-code login, refresh, and ChatGPT account identity.

use std::sync::atomic::AtomicBool;
use std::time::Duration;

use serde_json::{Value, json};

use super::http::{
    failure, json_error_code, number, oauth_access, post_form, post_json, required, sleep_cancel,
    token_credential,
};
use super::{Credential, DeviceCode};

const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const DEVICE_USER_CODE_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/usercode";
const DEVICE_TOKEN_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/token";
const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const DEVICE_REDIRECT_URI: &str = "https://auth.openai.com/deviceauth/callback";
const VERIFICATION_URI: &str = "https://auth.openai.com/codex/device";
const DEVICE_TIMEOUT_SECS: u64 = 15 * 60;
const JWT_AUTH: &str = "https://api.openai.com/auth";

pub fn request_device_code() -> Result<DeviceCode, String> {
    let response = post_json(
        DEVICE_USER_CODE_URL,
        &json!({ "client_id": CLIENT_ID }),
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
    let interval = number(&body, "interval")
        .ok_or_else(|| format!("OpenAI OAuth response has no interval: {body}"))?;
    Ok(DeviceCode {
        device_code: required("OpenAI", &body, "device_auth_id")?,
        user_code: required("OpenAI", &body, "user_code")?,
        verification_uri: VERIFICATION_URI.into(),
        interval: interval.max(1),
        expires_in: DEVICE_TIMEOUT_SECS,
    })
}

pub(super) fn poll(device: &DeviceCode, cancel: &AtomicBool) -> Result<Credential, String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(device.expires_in);
    let mut interval = device.interval;
    loop {
        sleep_cancel(Duration::from_secs(interval), cancel)?;
        if std::time::Instant::now() >= deadline {
            return Err("OpenAI device code expired".into());
        }
        let response = post_json(
            DEVICE_TOKEN_URL,
            &json!({
                "device_auth_id": device.device_code,
                "user_code": device.user_code,
            }),
            "OpenAI",
        )?;
        if response.0 {
            let authorization_code = required("OpenAI", &response.2, "authorization_code")?;
            let code_verifier = required("OpenAI", &response.2, "code_verifier")?;
            return exchange(&authorization_code, &code_verifier);
        }
        if matches!(response.1, 403 | 404) {
            continue;
        }
        match json_error_code(&response.2).as_deref() {
            Some("deviceauth_authorization_pending") => {}
            Some("slow_down") => interval = number(&response.2, "interval").unwrap_or(interval + 5),
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

fn exchange(code: &str, verifier: &str) -> Result<Credential, String> {
    let response = post_form(
        TOKEN_URL,
        &[
            ("grant_type", "authorization_code"),
            ("client_id", CLIENT_ID),
            ("code", code),
            ("code_verifier", verifier),
            ("redirect_uri", DEVICE_REDIRECT_URI),
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

pub(super) fn refresh(refresh: &str) -> Result<Credential, String> {
    let response = post_form(
        TOKEN_URL,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh),
            ("client_id", CLIENT_ID),
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

pub(super) fn chatgpt_account_id(access: &str) -> Result<String, String> {
    let payload =
        jwt_payload(access).ok_or_else(|| "OpenAI token has no account id".to_string())?;
    payload
        .get(JWT_AUTH)
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn chatgpt_account_id_from_jwt() {
        let token = jwt(&json!({ JWT_AUTH: { "chatgpt_account_id": "acct_1" } }));
        assert_eq!(chatgpt_account_id(&token).unwrap(), "acct_1");
        assert!(chatgpt_account_id("not-a-jwt").is_err());
        let empty = jwt(&json!({ JWT_AUTH: { "chatgpt_account_id": "" } }));
        assert!(chatgpt_account_id(&empty).is_err());
    }
}
