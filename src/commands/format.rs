use crate::app::App;
use crate::protocol::{Api, Config};

pub(crate) fn apply(app: &mut App, raw: &str) {
    if app.cancel.is_some() {
        app.notice = Some("cannot change format during a turn".into());
        return;
    }
    let api = match raw.trim() {
        "response" => Api::Responses,
        "chat_completions" => Api::Completions,
        _ => {
            app.notice = Some("usage: /format response|chat_completions".into());
            return;
        }
    };
    let Some(config) = app.config.as_mut() else {
        app.notice = Some("no model configured".into());
        return;
    };
    if let Err(err) = override_config(Some(api), config) {
        app.notice = Some(err);
        return;
    }
    app.format_override = Some(api);
    app.notice = Some(format!("format: {}", name(api)));
}

pub(crate) fn override_config(api: Option<Api>, config: &mut Config) -> Result<(), String> {
    let Some(api) = api else {
        return Ok(());
    };
    if api == Api::Completions && config.auth_provider.as_deref() == Some("openai") {
        return Err("OpenAI subscription auth requires response format".into());
    }
    config.api = api;
    Ok(())
}

fn name(api: Api) -> &'static str {
    match api {
        Api::Responses => "response",
        Api::Completions => "chat_completions",
        Api::Messages => "messages",
    }
}
