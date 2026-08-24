//! Parse the table returned by `init.lua` into guest definitions.

use std::collections::BTreeMap;

use mlua::{Table, Value};

use crate::protocol::{Api, Thinking};

#[derive(Default)]
pub(super) struct Guest {
    pub(super) models: BTreeMap<String, ModelDef>,
    pub(super) providers: BTreeMap<String, ProviderDef>,
    pub(super) defaults: Option<RawDefaults>,
    pub(super) model_notices: Vec<String>,
    pub(super) provider_notices: Vec<String>,
}

pub(super) struct RawDefaults {
    pub(super) provider: Option<String>,
    pub(super) model: Option<String>,
}

#[derive(Clone)]
pub(super) struct ModelDef {
    pub(super) id: String,
    pub(super) window: Option<u32>,
    pub(super) api: Api,
    pub(super) thinking: Option<Thinking>,
}

pub(super) struct ProviderDef {
    pub(super) base_url: Option<String>,
    pub(super) base_url_cmd: Option<String>,
    pub(super) key_name: Option<String>,
    pub(super) key_cmd: Option<String>,
    pub(super) key_in: String,
    pub(super) auth_provider: Option<String>,
    pub(super) thinking: Option<Thinking>,
    pub(super) models: Vec<Listed>,
}

pub(super) enum Listed {
    Alias(String),
    Local(ModelDef),
}

pub(super) fn parse(table: &Table) -> Result<Guest, String> {
    let (models, model_notices) = match table.get::<Value>("models") {
        Ok(Value::Table(models)) => parse_models(&models),
        Ok(Value::Nil) => (BTreeMap::new(), Vec::new()),
        _ => return Err("init.lua models is not a table".into()),
    };
    let (providers, provider_notices) = match table.get::<Value>("providers") {
        Ok(Value::Table(providers)) => parse_providers(&providers),
        Ok(Value::Nil) => (BTreeMap::new(), Vec::new()),
        _ => return Err("init.lua providers is not a table".into()),
    };
    let defaults = match table.get::<Value>("defaults") {
        Ok(Value::Table(defaults)) => Some(RawDefaults {
            provider: field_string(&defaults, "provider"),
            model: field_string(&defaults, "model"),
        }),
        Ok(Value::Nil) => None,
        _ => return Err("init.lua defaults is not a table".into()),
    };
    Ok(Guest {
        models,
        providers,
        defaults,
        model_notices,
        provider_notices,
    })
}

fn parse_models(table: &Table) -> (BTreeMap<String, ModelDef>, Vec<String>) {
    let mut models = BTreeMap::new();
    let mut notices = Vec::new();
    for pair in table.pairs::<Value, Value>() {
        let Ok((k, v)) = pair else {
            continue;
        };
        let Some(alias) = value_string(&k) else {
            continue;
        };
        match v {
            Value::Table(t) => match model_def(&t) {
                Ok(def) => {
                    models.insert(alias, def);
                }
                Err(err) => notices.push(def_error(&format!("model {alias}"), err)),
            },
            _ => notices.push(format!("model {alias} has no id")),
        }
    }
    (models, notices)
}

fn parse_providers(table: &Table) -> (BTreeMap<String, ProviderDef>, Vec<String>) {
    let mut providers = BTreeMap::new();
    let mut notices = Vec::new();
    for pair in table.pairs::<Value, Value>() {
        let Ok((k, v)) = pair else {
            continue;
        };
        let Some(name) = value_string(&k) else {
            continue;
        };
        let Value::Table(t) = v else {
            notices.push(format!("{name} is not a table"));
            continue;
        };
        let (models, extra) = match t.get::<Value>("models") {
            Ok(Value::Table(m)) => parse_listed(&m),
            Ok(Value::Nil) => (Vec::new(), Vec::new()),
            Ok(_) => {
                notices.push(format!("{name} models is not a list"));
                (Vec::new(), Vec::new())
            }
            Err(_) => (Vec::new(), Vec::new()),
        };
        notices.extend(extra);
        let key_in = field_string(&t, "key_in").unwrap_or_else(|| "env".into());
        let thinking = parse_thinking(&t, "thinking", &format!("provider {name}"), &mut notices);
        providers.insert(
            name,
            ProviderDef {
                base_url: field_string(&t, "base_url"),
                base_url_cmd: field_string(&t, "base_url_cmd"),
                key_name: field_string(&t, "key_name"),
                key_cmd: field_string(&t, "key_cmd"),
                key_in,
                auth_provider: field_string(&t, "auth_provider"),
                thinking,
                models,
            },
        );
    }
    (providers, notices)
}

fn parse_listed(table: &Table) -> (Vec<Listed>, Vec<String>) {
    let mut out = Vec::new();
    let mut notices = Vec::new();
    for (i, value) in table.sequence_values::<Value>().enumerate() {
        let n = i + 1;
        match value {
            Ok(Value::String(s)) => out.push(Listed::Alias(s.to_string_lossy())),
            Ok(Value::Table(t)) => match model_def(&t) {
                Ok(def) => out.push(Listed::Local(def)),
                Err(err) => notices.push(def_error(&format!("provider model #{n}"), err)),
            },
            Ok(_) => notices.push(format!("provider model #{n} skipped")),
            Err(err) => notices.push(err.to_string()),
        }
    }
    (out, notices)
}

enum DefError {
    NoId,
    UnknownApi(Option<String>),
    UnknownThinking(Option<String>),
}

fn model_def(table: &Table) -> Result<ModelDef, DefError> {
    let id = field_string(table, "id")
        .filter(|s| !s.is_empty())
        .ok_or(DefError::NoId)?;
    let window = match table.get::<Value>("window") {
        Ok(v) => opt_u32(v),
        Err(_) => None,
    };
    let api = match table.get::<Value>("api") {
        Ok(Value::Nil) | Err(_) => Api::Completions,
        Ok(v) => match value_string(&v) {
            Some(raw) => Api::parse(&raw).ok_or(DefError::UnknownApi(Some(raw)))?,
            None => return Err(DefError::UnknownApi(None)),
        },
    };
    let thinking = match table.get::<Value>("thinking") {
        Ok(Value::Nil) | Err(_) => None,
        Ok(v) => match value_string(&v).and_then(|raw| Thinking::parse(&raw)) {
            Some(level) => Some(level),
            None => return Err(DefError::UnknownThinking(value_string(&v))),
        },
    };
    Ok(ModelDef {
        id,
        window,
        api,
        thinking,
    })
}

fn parse_thinking(
    table: &Table,
    key: &str,
    prefix: &str,
    notices: &mut Vec<String>,
) -> Option<Thinking> {
    match table.get::<Value>(key) {
        Ok(Value::Nil) | Err(_) => None,
        Ok(value) => match value_string(&value).and_then(|raw| Thinking::parse(&raw)) {
            Some(level) => Some(level),
            None => {
                notices.push(match value_string(&value) {
                    Some(raw) => format!("{prefix} has unknown thinking: {raw}"),
                    None => format!("{prefix} has unknown thinking"),
                });
                None
            }
        },
    }
}

fn def_error(prefix: &str, err: DefError) -> String {
    match err {
        DefError::NoId => format!("{prefix} has no id"),
        DefError::UnknownApi(Some(raw)) => format!("{prefix} has unknown api: {raw}"),
        DefError::UnknownApi(None) => format!("{prefix} has unknown api"),
        DefError::UnknownThinking(Some(raw)) => {
            format!("{prefix} has unknown thinking: {raw}")
        }
        DefError::UnknownThinking(None) => format!("{prefix} has unknown thinking"),
    }
}

fn field_string(table: &Table, key: &str) -> Option<String> {
    match table.get::<Value>(key) {
        Ok(v) => value_string(&v),
        Err(_) => None,
    }
}

fn value_string(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.to_string_lossy()),
        _ => None,
    }
}

fn opt_u32(value: Value) -> Option<u32> {
    match value {
        Value::Integer(i) if i >= 0 && i <= i64::from(u32::MAX) => Some(i as u32),
        Value::Number(n) if n.is_finite() && (0.0..=f64::from(u32::MAX)).contains(&n) => {
            Some(n as u32)
        }
        _ => None,
    }
}
