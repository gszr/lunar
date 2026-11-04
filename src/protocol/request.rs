//! Explicit, credential-bearing curl export; never sends the model request.

use super::{Api, ChatMessage, Config, completions, http, messages, responses};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Scope {
    Prompt,
    Full,
    Slim,
}

pub(crate) fn export(app: &mut crate::app::App, line: &str) {
    let result = (|| {
        let mut cfg = app.config.clone().ok_or("no model configured")?;
        if let Some(integration) = cfg.auth_provider.as_deref() {
            cfg.api_key = crate::auth::saved_profile(integration, cfg.auth_profile.as_deref())?;
        }
        if crate::shell_prompt::contains_commands(line) {
            return Err(
                "request export does not execute ! commands; expand them manually first".into(),
            );
        }
        let mut history = if app.request_dump == Some(Scope::Full) {
            let preamble = crate::prompt::preamble(&app.stack, Some(&cfg.model));
            crate::context::history(
                preamble.as_deref(),
                app.compaction
                    .as_ref()
                    .map(|c| (c.summary.as_str(), c.first_kept)),
                &app.messages,
            )
        } else {
            Vec::new()
        };
        history.push(ChatMessage::User(line.into()));
        curl(
            &cfg,
            &history,
            app.mission.as_ref().map(|m| m.id.as_str()),
            app.request_dump != Some(Scope::Slim),
        )
    })();
    match result {
        Ok(script) => {
            app.request_dump = None;
            app.input.clear();
            app.cursor = 0;
            let fence = "`".repeat(
                script
                    .split(|c| c != '`')
                    .map(str::len)
                    .max()
                    .unwrap_or(0)
                    .max(2)
                    + 1,
            );
            let mut message = crate::app::Message::assistant();
            // Diagnostic replies are transient and excluded from context and compaction.
            message.debug = true;
            message.text = format!("{fence}sh\n{script}{fence}");
            app.request_curl = Some(script);
            app.messages.push(message);
            app.notice = None;
            crate::transcript::jump_to_tail(app);
        }
        Err(err) => app.notice = Some(format!("request export: {err}")),
    }
}

pub(crate) fn copy(app: &mut crate::app::App) {
    let Some(script) = app.request_curl.as_deref() else {
        app.notice = Some("no request preview to copy; generate one with /request first".into());
        return;
    };
    app.notice = Some(match copy_to_clipboard(script) {
        Ok(()) => "Copied request with credentials. Clipboard contains your token.".into(),
        Err(err) => format!("request copy: {err}"),
    });
}

#[cfg(target_os = "macos")]
fn copy_to_clipboard(script: &str) -> Result<(), String> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let mut child = Command::new("/usr/bin/pbcopy")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| format!("could not start pbcopy: {err}"))?;
    let written = child.stdin.take().unwrap().write_all(script.as_bytes());
    let status = child.wait().map_err(|err| format!("pbcopy: {err}"))?;
    written.map_err(|err| format!("clipboard write failed: {err}"))?;
    if !status.success() {
        return Err(format!("pbcopy exited with {status}"));
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn copy_to_clipboard(_script: &str) -> Result<(), String> {
    Err("local clipboard copying is currently supported only on macOS".into())
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn curl(
    cfg: &Config,
    history: &[ChatMessage],
    session: Option<&str>,
    tools: bool,
) -> Result<String, String> {
    let session = session
        .map(responses::clamp_cache_key)
        .filter(|s| !s.is_empty());
    let (url, body, session, account) = match cfg.api {
        Api::Completions => (
            format!("{}/chat/completions", cfg.base_url.trim_end_matches('/')),
            completions::body(cfg, history, tools),
            None,
            None,
        ),
        Api::Responses => {
            let account = if cfg.auth_provider.as_deref() == Some("openai") {
                Some(crate::auth::chatgpt_account_id(&cfg.api_key)?)
            } else {
                None
            };
            (
                responses::responses_url(cfg),
                responses::body(cfg, history, session.as_deref(), tools),
                session,
                account,
            )
        }
        Api::Messages => (
            messages::messages_url(&cfg.base_url),
            messages::body(cfg, history, tools),
            None,
            None,
        ),
    };
    let mut script = format!("curl --no-buffer --request POST --url {}", quote(&url));
    for (name, value) in http::request_headers(cfg, session.as_deref(), account.as_deref()) {
        script.push_str(&format!(
            " \\\n  --header {}",
            quote(&format!("{name}: {value}"))
        ));
    }
    // stdin avoids shell argument-size limits for large conversations.
    script.push_str(" --data-binary @- <<'LUNAR_REQUEST_BODY'\n");
    script.push_str(&body);
    script.push_str("\nLUNAR_REQUEST_BODY\n");
    Ok(script)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::process::{Command, Stdio};

    #[test]
    fn displayed_scripts_replay_all_protocols() {
        for (api, tools) in [Api::Completions, Api::Responses, Api::Messages]
            .into_iter()
            .flat_map(|api| [true, false].map(|tools| (api, tools)))
        {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let cfg = Config {
                api_key: "secret'$(false)".into(),
                base_url: format!("http://{}", listener.local_addr().unwrap()),
                model: "test-model".into(),
                provider: "local".into(),
                window: None,
                api,
                auth_provider: None,
                auth_profile: None,
                thinking: "high".into(),
                thinking_levels: vec!["off".into(), "high".into()],
            };
            let text = "a 'quote' $(false)\nLUNAR_REQUEST_BODY\nlast line";
            let script = curl(
                &cfg,
                &[ChatMessage::User(text.into())],
                Some("mission-1"),
                tools,
            )
            .unwrap();
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                let mut reader = BufReader::new(&mut stream);
                let mut headers = String::new();
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    headers.push_str(&line);
                }
                let length: usize = headers
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length: ")
                            .map(|s| s.parse().unwrap())
                    })
                    .unwrap();
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
                    )
                    .unwrap();
                (
                    headers,
                    serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
                )
            });
            let mut child = Command::new("sh")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(script.as_bytes())
                .unwrap();
            let output = child.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let (headers, body) = server.join().unwrap();
            assert!(headers.contains("secret'$(false)"));
            assert_eq!(body["model"], "test-model");
            assert_eq!(body["stream"], true);
            assert_eq!(body["tools"].is_array(), tools);
            if !tools {
                assert!(body.get("tools").is_none());
            }
            match api {
                Api::Completions => {
                    assert!(headers.starts_with("POST /chat/completions "));
                    assert_eq!(body["messages"][0]["content"], text);
                }
                Api::Responses => {
                    assert!(headers.starts_with("POST /responses "));
                    assert_eq!(body["input"][0]["content"][0]["text"], text);
                    assert!(headers.contains("session-id: mission-1"));
                }
                Api::Messages => {
                    assert!(headers.starts_with("POST /v1/messages "));
                    assert!(headers.contains("x-api-key:"));
                }
            }
        }
    }
}
