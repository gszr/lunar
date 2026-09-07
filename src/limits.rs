//! OpenAI subscription rate-limit snapshots.

use std::sync::mpsc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::app::App;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Limits {
    pub(crate) primary: Option<Window>,
    pub(crate) secondary: Option<Window>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Window {
    used_percent: f64,
    duration_seconds: u64,
    reset_at: u64,
}

pub(crate) fn refresh(app: &mut App) {
    let Some(config) = app
        .config
        .as_ref()
        .filter(|config| config.auth_provider.as_deref() == Some("openai"))
    else {
        app.limits = None;
        app.limits_rx = None;
        return;
    };
    if app.limits_rx.is_some() {
        return;
    }
    let base_url = config.base_url.clone();
    let (tx, rx) = mpsc::channel();
    app.limits_rx = Some(rx);
    std::thread::spawn(move || {
        if let Ok(limits) = fetch(&base_url) {
            let _ = tx.send(limits);
        }
    });
}

pub(crate) fn drain(app: &mut App) {
    let Some(rx) = app.limits_rx.as_ref() else {
        return;
    };
    match rx.try_recv() {
        Ok(limits) => {
            if app
                .config
                .as_ref()
                .is_some_and(|config| config.auth_provider.as_deref() == Some("openai"))
            {
                app.limits = Some(limits);
            }
            app.limits_rx = None;
        }
        Err(mpsc::TryRecvError::Disconnected) => app.limits_rx = None,
        Err(mpsc::TryRecvError::Empty) => {}
    }
}

fn fetch(base_url: &str) -> Result<Limits, String> {
    let access = crate::auth::resolve("openai")?;
    let account = crate::auth::chatgpt_account_id(&access)?;
    let url = format!("{}/wham/usage", base_url.trim_end_matches('/'));
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into();
    let mut response = agent
        .get(&url)
        .header("Authorization", &format!("Bearer {access}"))
        .header("chatgpt-account-id", &account)
        .call()
        .map_err(|err| err.to_string())?;
    let status = response.status().as_u16();
    let text = response
        .body_mut()
        .read_to_string()
        .map_err(|err| err.to_string())?;
    if !response.status().is_success() {
        return Err(format!("OpenAI limits HTTP {status}"));
    }
    let value: Value = serde_json::from_str(&text).map_err(|err| err.to_string())?;
    parse(&value).ok_or_else(|| "OpenAI limits response has no windows".into())
}

fn parse(value: &Value) -> Option<Limits> {
    let limits = value.get("rate_limit")?;
    let primary = limits.get("primary_window").and_then(parse_window);
    let secondary = limits.get("secondary_window").and_then(parse_window);
    (primary.is_some() || secondary.is_some()).then_some(Limits { primary, secondary })
}

fn parse_window(value: &Value) -> Option<Window> {
    let used_percent = value.get("used_percent")?.as_f64()?;
    let duration_seconds = value.get("limit_window_seconds")?.as_u64()?;
    let reset_at = value.get("reset_at").and_then(Value::as_u64).or_else(|| {
        value
            .get("reset_after_seconds")
            .and_then(Value::as_u64)
            .map(|after| now().saturating_add(after))
    })?;
    Some(Window {
        used_percent,
        duration_seconds,
        reset_at,
    })
}

pub(crate) fn label(limits: &Limits) -> String {
    [&limits.primary, &limits.secondary]
        .into_iter()
        .flatten()
        .map(window_label)
        .collect::<Vec<_>>()
        .join(" · ")
}

fn window_label(window: &Window) -> String {
    let remaining = (100.0 - window.used_percent).clamp(0.0, 100.0).round() as u32;
    let reset = window.reset_at.saturating_sub(now());
    format!(
        "{} {remaining}% ↻{}",
        duration_label(window.duration_seconds),
        reset_label(reset)
    )
}

fn duration_label(seconds: u64) -> String {
    if seconds >= 86_400 && seconds.is_multiple_of(86_400) {
        format!("{}d", seconds / 86_400)
    } else if seconds >= 3_600 && seconds.is_multiple_of(3_600) {
        format!("{}h", seconds / 3_600)
    } else {
        format!("{}m", seconds.div_ceil(60))
    }
}

fn reset_label(seconds: u64) -> String {
    if seconds == 0 {
        "now".into()
    } else if seconds < 3_600 {
        format!("{}m", seconds.div_ceil(60))
    } else if seconds < 86_400 {
        format!("{}h", seconds.div_ceil(3_600))
    } else {
        format!("{}d", seconds.div_ceil(86_400))
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_subscription_windows() {
        let reset_at = now() + 7_200;
        let limits = parse(&json!({
            "rate_limit": {
                "primary_window": {
                    "used_percent": 27,
                    "limit_window_seconds": 18_000,
                    "reset_after_seconds": 7_200,
                    "reset_at": reset_at
                },
                "secondary_window": {
                    "used_percent": 59,
                    "limit_window_seconds": 604_800,
                    "reset_after_seconds": 345_600,
                    "reset_at": now() + 345_600
                }
            }
        }))
        .unwrap();

        assert_eq!(label(&limits), "5h 73% ↻2h · 7d 41% ↻4d");
    }
}
