//! Parse the table returned by `init.lua` into guest definitions.

use std::collections::BTreeMap;

use mlua::{Table, Value};

use crate::protocol::Api;

#[derive(Default)]
pub(super) struct Guest {
    pub(super) models: BTreeMap<String, ModelDef>,
    pub(super) providers: BTreeMap<String, ProviderDef>,
    pub(super) defaults: Option<RawDefaults>,
    pub(super) stack: BTreeMap<String, crate::stack::Component>,
}

pub(super) struct RawDefaults {
    pub(super) source: std::path::PathBuf,
    pub(super) provider: String,
    pub(super) model: String,
    pub(super) thinking: Option<String>,
}

#[derive(Clone)]
pub(super) struct ThinkingDef {
    pub(super) levels: Vec<String>,
    pub(super) default: String,
}

#[derive(Clone)]
pub(super) struct ModelDef {
    pub(super) id: String,
    pub(super) window: Option<u32>,
    pub(super) api: Api,
    pub(super) thinking: ThinkingDef,
}

pub(super) struct ProviderDef {
    pub(super) source: std::path::PathBuf,
    pub(super) base_url: Option<String>,
    pub(super) base_url_cmd: Option<String>,
    pub(super) key_name: Option<String>,
    pub(super) key_cmd: Option<String>,
    pub(super) key_in: String,
    pub(super) auth_provider: Option<String>,
    pub(super) models: Vec<Listed>,
}

pub(super) enum Listed {
    Alias(String),
    Local(ModelDef),
}

impl Guest {
    pub(super) fn merge(&mut self, project: Self) {
        self.models.extend(project.models);
        self.providers.extend(project.providers);
        self.stack.extend(project.stack);
        if project.defaults.is_some() {
            self.defaults = project.defaults;
        }
    }
}

pub(super) fn parse(
    table: &Table,
    base: &std::path::Path,
    source: &std::path::Path,
) -> Result<Guest, String> {
    let (models, model_notices) = match table.get::<Value>("models") {
        Ok(Value::Table(models)) => parse_models(&models),
        Ok(Value::Nil) => (BTreeMap::new(), Vec::new()),
        _ => return Err("init.lua models is not a table".into()),
    };
    let (providers, provider_notices) = match table.get::<Value>("providers") {
        Ok(Value::Table(providers)) => parse_providers(&providers, source),
        Ok(Value::Nil) => (BTreeMap::new(), Vec::new()),
        _ => return Err("init.lua providers is not a table".into()),
    };
    let defaults = match table.get::<Value>("defaults") {
        Ok(Value::Table(defaults)) => Some(RawDefaults {
            source: source.into(),
            provider: required_string(&defaults, "provider")?,
            model: required_string(&defaults, "model")?,
            thinking: optional_string(&defaults, "thinking")
                .map_err(|err| format!("defaults: {err}"))?,
        }),
        Ok(Value::Nil) => None,
        _ => return Err("init.lua defaults is not a table".into()),
    };
    let errors: Vec<_> = model_notices.into_iter().chain(provider_notices).collect();
    if !errors.is_empty() {
        return Err(errors.join("\n"));
    }
    Ok(Guest {
        models,
        providers,
        defaults,
        stack: crate::stack::parse(table, base)?,
    })
}

fn parse_models(table: &Table) -> (BTreeMap<String, ModelDef>, Vec<String>) {
    let mut models = BTreeMap::new();
    let mut notices = Vec::new();
    for pair in table.pairs::<Value, Value>() {
        let Ok((k, v)) = pair else {
            notices.push("invalid model entry".into());
            continue;
        };
        let Some(alias) = value_string(&k).filter(|s| !s.trim().is_empty()) else {
            notices.push("model name must be a non-empty string".into());
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

fn parse_providers(
    table: &Table,
    source: &std::path::Path,
) -> (BTreeMap<String, ProviderDef>, Vec<String>) {
    let mut providers = BTreeMap::new();
    let mut notices = Vec::new();
    for pair in table.pairs::<Value, Value>() {
        let Ok((k, v)) = pair else {
            notices.push("invalid provider entry".into());
            continue;
        };
        let Some(name) = value_string(&k).filter(|s| !s.trim().is_empty()) else {
            notices.push("provider name must be a non-empty string".into());
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
            Err(err) => {
                notices.push(format!("{name} models: {err}"));
                (Vec::new(), Vec::new())
            }
        };
        notices.extend(extra.into_iter().map(|err| format!("{name}: {err}")));
        let key_in = field_string(&t, "key_in").unwrap_or_else(|| "env".into());
        for field in [
            "base_url",
            "base_url_cmd",
            "key_name",
            "key_cmd",
            "key_in",
            "auth_provider",
        ] {
            if key_in == "none"
                && matches!(
                    field,
                    "key_name" | "key_cmd" | "auth_provider" | "base_url_cmd"
                )
            {
                continue;
            }
            if let Err(err) = optional_string(&t, field) {
                notices.push(format!("provider {name}: {err}"));
            }
        }
        if !matches!(t.get::<Value>("thinking"), Ok(Value::Nil) | Err(_)) {
            notices.push(format!("provider {name} thinking is not supported"));
        }
        providers.insert(
            name,
            ProviderDef {
                source: source.into(),
                base_url: field_string(&t, "base_url"),
                base_url_cmd: field_string(&t, "base_url_cmd"),
                key_name: field_string(&t, "key_name"),
                key_cmd: field_string(&t, "key_cmd"),
                key_in,
                auth_provider: field_string(&t, "auth_provider"),
                models,
            },
        );
    }
    (providers, notices)
}

fn parse_listed(table: &Table) -> (Vec<Listed>, Vec<String>) {
    let mut out = Vec::new();
    let mut notices = Vec::new();
    if !is_list(table) {
        return (out, vec!["models must be a contiguous list".into()]);
    }
    for (i, value) in table.sequence_values::<Value>().enumerate() {
        let n = i + 1;
        match value {
            Ok(Value::String(s)) => out.push(Listed::Alias(s.to_string_lossy())),
            Ok(Value::Table(t)) => match model_def(&t) {
                Ok(def) => out.push(Listed::Local(def)),
                Err(err) => notices.push(def_error(&format!("provider model #{n}"), err)),
            },
            Ok(_) => notices.push(format!(
                "provider model #{n} must be an alias or model table"
            )),
            Err(err) => notices.push(err.to_string()),
        }
    }
    (out, notices)
}

enum DefError {
    NoId,
    InvalidWindow,
    UnknownApi(Option<String>),
    InvalidThinking,
    EmptyThinking,
    MissingThinkingDefault,
    UnknownThinkingDefault(String),
}

fn model_def(table: &Table) -> Result<ModelDef, DefError> {
    let id = field_string(table, "id")
        .filter(|s| !s.trim().is_empty())
        .ok_or(DefError::NoId)?;
    let window = match table.get::<Value>("window") {
        Ok(Value::Nil) => None,
        Ok(v) => Some(opt_u32(v).ok_or(DefError::InvalidWindow)?),
        Err(_) => return Err(DefError::InvalidWindow),
    };
    let api = match table.get::<Value>("api") {
        Ok(Value::Nil) => Api::Completions,
        Err(_) => return Err(DefError::UnknownApi(None)),
        Ok(v) => match value_string(&v) {
            Some(raw) => Api::parse(&raw).ok_or(DefError::UnknownApi(Some(raw)))?,
            None => return Err(DefError::UnknownApi(None)),
        },
    };
    let thinking = match table.get::<Value>("thinking") {
        Ok(Value::Nil) => ThinkingDef {
            levels: vec!["off".into()],
            default: "off".into(),
        },
        Ok(Value::Table(value)) => parse_thinking(&value)?,
        _ => return Err(DefError::InvalidThinking),
    };
    Ok(ModelDef {
        id,
        window,
        api,
        thinking,
    })
}

fn parse_thinking(table: &Table) -> Result<ThinkingDef, DefError> {
    let mut levels = Vec::new();
    for value in table.sequence_values::<Value>() {
        let Ok(value) = value else {
            return Err(DefError::InvalidThinking);
        };
        let Some(level) = value_string(&value).filter(|level| !level.is_empty()) else {
            return Err(DefError::InvalidThinking);
        };
        if !levels.contains(&level) {
            levels.push(level);
        }
    }
    if levels.is_empty() {
        return Err(DefError::EmptyThinking);
    }
    let default = field_string(table, "default")
        .filter(|level| !level.is_empty())
        .ok_or(DefError::MissingThinkingDefault)?;
    if !levels.contains(&default) {
        return Err(DefError::UnknownThinkingDefault(default));
    }
    Ok(ThinkingDef { levels, default })
}

fn def_error(prefix: &str, err: DefError) -> String {
    match err {
        DefError::InvalidWindow => format!("{prefix} window must be a non-negative integer"),
        DefError::NoId => format!("{prefix} has no id"),
        DefError::UnknownApi(Some(raw)) => format!("{prefix} has unknown api: {raw}"),
        DefError::UnknownApi(None) => format!("{prefix} has unknown api"),
        DefError::InvalidThinking => format!("{prefix} thinking is not a table of strings"),
        DefError::EmptyThinking => format!("{prefix} thinking has no levels"),
        DefError::MissingThinkingDefault => format!("{prefix} thinking has no default"),
        DefError::UnknownThinkingDefault(raw) => {
            format!("{prefix} thinking default is not listed: {raw}")
        }
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
        Value::Number(n)
            if n.is_finite() && n.fract() == 0.0 && (0.0..=f64::from(u32::MAX)).contains(&n) =>
        {
            Some(n as u32)
        }
        _ => None,
    }
}

fn optional_string(table: &Table, key: &str) -> Result<Option<String>, String> {
    match table.get::<Value>(key) {
        Ok(Value::Nil) => Ok(None),
        Ok(Value::String(value)) if !value.to_string_lossy().trim().is_empty() => {
            Ok(Some(value.to_string_lossy()))
        }
        _ => Err(format!("{key} must be a non-empty string")),
    }
}

fn required_string(table: &Table, key: &str) -> Result<String, String> {
    optional_string(table, key)
        .map_err(|err| format!("defaults: {err}"))?
        .ok_or_else(|| "defaults needs provider and model".to_string())
}

fn is_list(table: &Table) -> bool {
    let mut count = 0;
    for pair in table.pairs::<Value, Value>() {
        match pair {
            Ok((Value::Integer(index), _)) if index > 0 && index as usize <= table.raw_len() => {
                count += 1
            }
            _ => return false,
        }
    }
    count == table.raw_len()
}
