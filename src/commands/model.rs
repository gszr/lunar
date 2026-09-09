use crate::app::{App, Mode};
use crate::turn::persist_value;
use crate::{lua, mission};

pub(crate) fn open_model(app: &mut App) {
    if app.models.is_empty() {
        app.notice = Some("no configured models".into());
        return;
    }
    let cursor = app
        .config
        .as_ref()
        .and_then(|cfg| {
            app.models
                .iter()
                .position(|m| m.provider == cfg.provider && m.id == cfg.model)
        })
        .unwrap_or(0);
    app.mode = Mode::Model {
        items: app.models.clone(),
        cursor,
        query: None,
    };
}

pub(crate) fn matches_query(item: &lua::ModelChoice, query: &str) -> bool {
    let query = query.to_lowercase();
    query.is_empty()
        || item.provider.to_lowercase().contains(&query)
        || item.id.to_lowercase().contains(&query)
        || item
            .alias
            .as_deref()
            .is_some_and(|alias| alias.to_lowercase().contains(&query))
}

pub(crate) fn select_model(app: &mut App, item: lua::ModelChoice, persist: bool) {
    let Some(config) = item.config else {
        app.notice = Some(item.error.unwrap_or_else(|| "model is unavailable".into()));
        return;
    };
    app.thinking_override = None;
    let default_thinking = config.thinking.clone();
    app.config = Some(config);
    crate::limits::refresh(app);
    app.notice = Some(format!("model: {} / {}", item.provider, item.id));
    if persist {
        persist_value(app, &mission::model_line(&item.provider, &item.id));
        persist_value(app, &mission::thinking_line(&default_thinking));
    }
}

pub(crate) fn restore_model(app: &mut App, provider: &str, id: &str) -> bool {
    let choice = app
        .models
        .iter()
        .find(|m| m.provider == provider && m.id == id)
        .cloned();
    if let Some(item) = choice.filter(|m| m.config.is_some()) {
        select_model(app, item, false);
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::matches_query;
    use crate::lua::ModelChoice;

    fn choice() -> ModelChoice {
        ModelChoice {
            provider: "xAI".into(),
            alias: Some("grok46".into()),
            id: "grok-4.6".into(),
            config: None,
            error: None,
        }
    }

    #[test]
    fn model_search_matches_provider_alias_and_id_case_insensitively() {
        let item = choice();
        assert!(matches_query(&item, "XAI"));
        assert!(matches_query(&item, "GROK46"));
        assert!(matches_query(&item, "4.6"));
        assert!(!matches_query(&item, "openai"));
    }
}
