//! xAI device-code login and token refresh.

use std::sync::atomic::AtomicBool;
use std::time::Duration;

use super::http::{failure, number, post_form, required, sleep_cancel, string, token_credential};
use super::{Credential, DeviceCode};

const CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";
const SCOPE: &str = "openid profile email offline_access grok-cli:access api:access";
const DEVICE_CODE_URL: &str = "https://auth.x.ai/oauth2/device/code";
const TOKEN_URL: &str = "https://auth.x.ai/oauth2/token";

pub(super) fn request_device_code() -> Result<DeviceCode, String> {
    let response = post_form(
        DEVICE_CODE_URL,
        &[
            ("client_id", CLIENT_ID),
            ("scope", SCOPE),
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

pub(super) fn poll(device: &DeviceCode, cancel: &AtomicBool) -> Result<Credential, String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(device.expires_in);
    let mut interval = device.interval;
    loop {
        sleep_cancel(Duration::from_secs(interval), cancel)?;
        if std::time::Instant::now() >= deadline {
            return Err("xAI device code expired".into());
        }
        let response = post_form(
            TOKEN_URL,
            &[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("client_id", CLIENT_ID),
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

pub(super) fn refresh(refresh: &str) -> Result<Credential, String> {
    let response = post_form(
        TOKEN_URL,
        &[
            ("grant_type", "refresh_token"),
            ("client_id", CLIENT_ID),
            ("refresh_token", refresh),
        ],
        "xAI",
    )?;
    if !response.0 {
        return Err(failure("xAI", "token refresh", response.1, &response.2));
    }
    token_credential("xAI", &response.2, Some(refresh))
}
