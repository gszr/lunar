//! Shared POST retry, SSE helpers, and usage parsing.

use std::io::{self, BufRead};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use ureq::Agent;

use super::Usage;

const MAX_RETRIES: u32 = 3;

pub(super) fn post_retry(
    url: &str,
    api_key: &str,
    body: &str,
    cancel: &AtomicBool,
    session: Option<&str>,
    account: Option<&str>,
) -> Result<ureq::http::Response<ureq::Body>, String> {
    let mut attempt = 0;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("aborted".into());
        }
        let mut headers = json!({
            "authorization": "Bearer [REDACTED]",
            "content-type": "application/json"
        });
        if let Some(session) = session {
            headers["session_id"] = json!(session);
            headers["x-client-request-id"] = json!(session);
        }
        if let Some(account) = account {
            headers["chatgpt-account-id"] = json!(account);
            headers["originator"] = json!("lunar");
            headers["openai-beta"] = json!("responses=experimental");
        }
        crate::debug::event(
            "request",
            json!({
                "attempt": attempt + 1,
                "method": "POST",
                "url": url,
                "headers": headers,
                "body": serde_json::from_str::<Value>(body).unwrap_or_else(|_| Value::String(body.into())),
            }),
        );
        let mut request = agent()
            .post(url)
            .header("Authorization", &format!("Bearer {api_key}"))
            .header("Content-Type", "application/json");
        if let Some(session) = session {
            request = request
                .header("session_id", session)
                .header("x-client-request-id", session);
        }
        if let Some(account) = account {
            request = request
                .header("chatgpt-account-id", account)
                .header("originator", "lunar")
                .header("OpenAI-Beta", "responses=experimental");
        }
        let response = request.send(body);
        match response {
            Ok(response) if response.status().is_success() => {
                crate::debug::event(
                    "response_start",
                    json!({
                        "attempt": attempt + 1,
                        "status": response.status().as_u16(),
                    }),
                );
                return Ok(response);
            }
            Ok(response) => {
                let status = response.status().as_u16();
                let retry = attempt < MAX_RETRIES && should_retry_status(status);
                let delay = retry.then(|| retry_delay(attempt));
                let body = response.into_body().read_to_string().unwrap_or_default();
                crate::debug::event(
                    "response_error",
                    json!({
                        "attempt": attempt + 1,
                        "status": status,
                        "body": body,
                        "retry": retry,
                        "retry_delay_ms": delay.map(|d| d.as_millis()),
                    }),
                );
                if !retry {
                    return Err(error_response(status, &body));
                }
                sleep_cancel(delay.unwrap(), cancel)?;
                attempt += 1;
            }
            Err(err) => {
                let retry = attempt < MAX_RETRIES && should_retry(&err);
                let delay = retry.then(|| retry_delay(attempt));
                crate::debug::event(
                    "request_error",
                    json!({
                        "attempt": attempt + 1,
                        "error": err.to_string(),
                        "retry": retry,
                        "retry_delay_ms": delay.map(|d| d.as_millis()),
                    }),
                );
                if !retry {
                    return Err(err.to_string());
                }
                sleep_cancel(delay.unwrap(), cancel)?;
                attempt += 1;
            }
        }
    }
}

/// Usage often arrives after finish_reason. Wait briefly for it, then keep
/// reading in the background so the socket can return to the pool.
pub(super) fn collect_tail(
    reader: impl BufRead + Send + 'static,
    usage: &mut Option<Usage>,
    saw_done: &mut bool,
    cancel: &AtomicBool,
) {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut lines = reader.lines();
        while let Some(line) = lines.next() {
            let Ok(line) = line else {
                return;
            };
            let done = is_done_line(&line);
            if tx.send(line).is_err() {
                if done {
                    return;
                }
                for line in lines {
                    let Ok(line) = line else {
                        return;
                    };
                    if is_done_line(&line) {
                        return;
                    }
                }
                return;
            }
        }
    });
    if usage.is_some() {
        return;
    }
    let deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(line) => {
                if is_done_line(&line) {
                    *saw_done = true;
                    break;
                }
                if let Some(data) = sse_payload(&line)
                    && let Ok(value) = serde_json::from_str::<Value>(data)
                    && let Some(parsed) = parse_usage(&value)
                {
                    *usage = Some(parsed);
                    break;
                }
            }
            Err(_) => break,
        }
    }
}

fn is_done_line(line: &str) -> bool {
    line.strip_prefix("data:")
        .is_some_and(|d| d.trim() == "[DONE]")
}

pub(super) fn sse_payload(line: &str) -> Option<&str> {
    let data = line.strip_prefix("data:")?.trim();
    if data.is_empty() || data == "[DONE]" {
        None
    } else {
        Some(data)
    }
}

fn should_retry_status(code: u16) -> bool {
    matches!(code, 408 | 409 | 429) || code >= 500
}

fn error_response(status: u16, body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .pointer("/error/message")
                .or_else(|| value.get("message"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .filter(|message| !message.is_empty())
        .unwrap_or_else(|| format!("http status: {status}"))
}

fn should_retry(err: &ureq::Error) -> bool {
    match err {
        ureq::Error::StatusCode(code) => should_retry_status(*code),
        ureq::Error::Io(e) => matches!(
            e.kind(),
            io::ErrorKind::ConnectionReset
                | io::ErrorKind::ConnectionAborted
                | io::ErrorKind::TimedOut
                | io::ErrorKind::BrokenPipe
                | io::ErrorKind::UnexpectedEof
                | io::ErrorKind::Interrupted
                | io::ErrorKind::NotConnected
        ),
        ureq::Error::ConnectionFailed | ureq::Error::Timeout(_) => true,
        _ => false,
    }
}

fn retry_delay(attempt: u32) -> Duration {
    Duration::from_millis(500 * (1u64 << attempt.min(4)))
}

fn sleep_cancel(total: Duration, cancel: &AtomicBool) -> Result<(), String> {
    let start = Instant::now();
    while start.elapsed() < total {
        if cancel.load(Ordering::Relaxed) {
            return Err("aborted".into());
        }
        let left = total.saturating_sub(start.elapsed());
        thread::sleep(left.min(Duration::from_millis(50)));
    }
    Ok(())
}

fn agent() -> Agent {
    static AGENT: OnceLock<Agent> = OnceLock::new();
    AGENT
        .get_or_init(|| {
            Agent::config_builder()
                .http_status_as_error(false)
                .timeout_global(None)
                .timeout_connect(Some(Duration::from_secs(30)))
                .build()
                .into()
        })
        .clone()
}

pub(super) fn parse_usage(value: &Value) -> Option<Usage> {
    let usage = value.get("usage")?;
    if usage.is_null() {
        return None;
    }
    let num = |v: &Value, keys: &[&str]| -> u32 {
        for key in keys {
            if let Some(n) = v.get(*key).and_then(Value::as_u64) {
                return n as u32;
            }
        }
        0
    };
    let detail_num = |keys: &[&str]| {
        ["prompt_tokens_details", "input_tokens_details"]
            .iter()
            .filter_map(|name| usage.get(*name))
            .find_map(|details| {
                keys.iter()
                    .find_map(|key| details.get(*key).and_then(Value::as_u64))
            })
    };
    let cache_read = detail_num(&["cached_tokens", "cache_read_tokens"])
        .unwrap_or_else(|| u64::from(num(usage, &["cache_read_tokens", "cached_tokens"])))
        as u32;
    let cache_write = detail_num(&["cache_write_tokens", "cache_creation_tokens"])
        .unwrap_or_else(|| u64::from(num(usage, &["cache_write_tokens", "cache_creation_tokens"])))
        as u32;
    let prompt = num(usage, &["prompt_tokens", "input_tokens"]);
    Some(Usage {
        input: prompt.saturating_sub(cache_read.saturating_add(cache_write)),
        output: num(usage, &["completion_tokens", "output_tokens"]),
        cache_read,
        cache_write,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_usage_splits_cache_out_of_prompt() {
        let value = json!({
            "usage": {
                "prompt_tokens": 1000,
                "completion_tokens": 20,
                "prompt_tokens_details": { "cached_tokens": 800 }
            }
        });
        let u = parse_usage(&value).unwrap();
        assert_eq!(u.input, 200);
        assert_eq!(u.cache_read, 800);
        assert_eq!(u.output, 20);
        assert_eq!(u.prompt(), 1000);
    }

    #[test]
    fn parse_responses_usage_splits_cache_write_out_of_input() {
        let value = json!({
            "usage": {
                "input_tokens": 5889,
                "input_tokens_details": {
                    "cache_write_tokens": 5886,
                    "cached_tokens": 0
                },
                "output_tokens": 130
            }
        });
        let u = parse_usage(&value).unwrap();
        assert_eq!(u.input, 3);
        assert_eq!(u.cache_read, 0);
        assert_eq!(u.cache_write, 5886);
        assert_eq!(u.output, 130);
        assert_eq!(u.prompt(), 5889);
    }

    #[test]
    fn retries_transient_status_not_auth() {
        assert!(should_retry_status(429));
        assert!(should_retry_status(503));
        assert!(should_retry_status(408));
        assert!(!should_retry_status(401));
        assert!(!should_retry_status(400));
    }

    #[test]
    fn error_response_uses_json_message() {
        assert_eq!(
            error_response(400, r#"{"error":{"message":"bad model"}}"#),
            "bad model"
        );
        assert_eq!(error_response(400, "not json"), "http status: 400");
    }

    #[test]
    fn retry_delay_grows_then_caps() {
        assert_eq!(retry_delay(0), Duration::from_millis(500));
        assert_eq!(retry_delay(1), Duration::from_millis(1000));
        assert_eq!(retry_delay(2), Duration::from_millis(2000));
        assert_eq!(retry_delay(4), Duration::from_millis(8000));
        assert_eq!(retry_delay(9), Duration::from_millis(8000));
    }
}
