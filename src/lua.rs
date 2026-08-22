//! User `~/.lunar/init.lua`. Dump-table registrars; last call wins.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::Path;
use std::rc::Rc;

use mlua::{Lua, Table, Value};

use crate::protocol::{self, Api, Config};

pub(crate) struct Loaded {
    pub config: Option<Config>,
    pub models: Vec<ModelChoice>,
    pub notice: Option<String>,
}

#[derive(Clone)]
pub struct ModelChoice {
    pub provider: String,
    pub alias: Option<String>,
    pub id: String,
    pub config: Option<Config>,
    pub error: Option<String>,
}

pub fn load() -> Loaded {
    load_path(&crate::mission::home().join("init.lua"))
}

fn load_path(path: &Path) -> Loaded {
    if !path.is_file() {
        return Loaded {
            config: Config::from_env(),
            models: env_choices(),
            notice: None,
        };
    }
    match std::fs::read_to_string(path) {
        Ok(src) => run(path, &src),
        Err(err) => Loaded {
            config: None,
            models: Vec::new(),
            notice: Some(format!("init.lua: {err}")),
        },
    }
}

fn run(path: &Path, src: &str) -> Loaded {
    let guest = Rc::new(RefCell::new(Guest::default()));
    let lua = Lua::new();
    if let Err(err) = inject(&lua, &guest).and_then(|()| {
        lua.load(src)
            .set_name(format!("@{}", path.display()))
            .exec()
    }) {
        return Loaded {
            config: None,
            models: Vec::new(),
            notice: Some(format!("init.lua: {err}")),
        };
    }
    resolve(&guest.borrow())
}

fn inject(lua: &Lua, guest: &Rc<RefCell<Guest>>) -> mlua::Result<()> {
    let lunar = lua.create_table()?;
    {
        let guest = guest.clone();
        lunar.set(
            "models",
            lua.create_function(move |_, table: Table| {
                let (models, notices) = parse_models(&table);
                let mut g = guest.borrow_mut();
                g.models = models;
                g.model_notices = notices;
                Ok(())
            })?,
        )?;
    }
    {
        let guest = guest.clone();
        lunar.set(
            "providers",
            lua.create_function(move |_, table: Table| {
                let (providers, notices) = parse_providers(&table);
                let mut g = guest.borrow_mut();
                g.providers = providers;
                g.provider_notices = notices;
                Ok(())
            })?,
        )?;
    }
    {
        let guest = guest.clone();
        lunar.set(
            "defaults",
            lua.create_function(move |_, table: Table| {
                guest.borrow_mut().defaults = Some(RawDefaults {
                    provider: field_string(&table, "provider"),
                    model: field_string(&table, "model"),
                });
                Ok(())
            })?,
        )?;
    }
    lua.globals().set("lunar", lunar)
}

fn resolve(guest: &Guest) -> Loaded {
    let Some(defaults) = &guest.defaults else {
        return Loaded {
            config: Config::from_env(),
            models: env_choices(),
            notice: None,
        };
    };
    match config_from_lua(guest, defaults) {
        Ok((config, extra)) => {
            let notice = join_notices(
                guest
                    .model_notices
                    .iter()
                    .chain(guest.provider_notices.iter())
                    .chain(extra.iter()),
            );
            Loaded {
                config: Some(config),
                models: choices(guest),
                notice,
            }
        }
        Err(notice) => {
            let combined = join_notices(
                guest
                    .model_notices
                    .iter()
                    .chain(guest.provider_notices.iter())
                    .chain(std::iter::once(&notice)),
            );
            Loaded {
                config: None,
                models: choices(guest),
                notice: combined,
            }
        }
    }
}

fn env_choices() -> Vec<ModelChoice> {
    Config::from_env()
        .map(|config| ModelChoice {
            provider: config.provider.clone(),
            alias: None,
            id: config.model.clone(),
            config: Some(config),
            error: None,
        })
        .into_iter()
        .collect()
}

fn choices(guest: &Guest) -> Vec<ModelChoice> {
    let mut out = Vec::new();
    for (provider_key, provider) in &guest.providers {
        let (models, _) = resolve_listed(&guest.models, &provider.models);
        for model in models {
            let result = provider_config(provider_key, provider, &model);
            let (config, error) = match result {
                Ok(config) => (Some(config), None),
                Err(error) => (None, Some(error)),
            };
            out.push(ModelChoice {
                provider: provider_key.clone(),
                alias: model.alias,
                id: model.id,
                config,
                error,
            });
        }
    }
    out
}

fn provider_config(
    provider_key: &str,
    provider: &ProviderDef,
    model: &ResolvedModel,
) -> Result<Config, String> {
    let base_url = provider
        .base_url
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("{provider_key} has no base_url"))?;
    let api_key = match provider.key_in.as_str() {
        "env" => {
            let key_name = provider
                .key_name
                .as_deref()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| format!("{provider_key} has no key_name"))?;
            nonempty(key_name).ok_or_else(|| format!("missing {key_name}"))?
        }
        "auth" => {
            let auth_provider = provider
                .auth_provider
                .as_deref()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| format!("{provider_key} has no auth_provider"))?;
            crate::auth::resolve(auth_provider)?
        }
        _ => return Err(format!("{provider_key} key_in is not env or auth")),
    };
    if model.api == Api::Messages {
        let name = model.alias.as_deref().unwrap_or(model.id.as_str());
        return Err(format!(
            "{name} uses {}, not implemented",
            model.api.as_str()
        ));
    }
    Ok(Config {
        api_key,
        base_url: base_url.to_string(),
        model: model.id.clone(),
        provider: provider_key.to_string(),
        window: model.window.or_else(|| protocol::guess_window(&model.id)),
        api: model.api,
    })
}

fn config_from_lua(guest: &Guest, defaults: &RawDefaults) -> Result<(Config, Vec<String>), String> {
    let provider_key = defaults
        .provider
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "lunar.defaults needs provider and model".to_string())?;
    let model_key = defaults
        .model
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "lunar.defaults needs provider and model".to_string())?;
    let provider = guest
        .providers
        .get(provider_key)
        .ok_or_else(|| format!("unknown provider: {provider_key}"))?;
    let (listed, skips) = resolve_listed(&guest.models, &provider.models);
    let chosen = match pick_model(&listed, model_key) {
        Some(model) => model,
        None => return Err(join_parts(skips, format!("unknown model: {model_key}"))),
    };
    match provider_config(provider_key, provider, chosen) {
        Ok(config) => Ok((config, skips)),
        Err(err) => Err(join_parts(skips, err)),
    }
}

fn resolve_listed(
    catalog: &BTreeMap<String, ModelDef>,
    listed: &[Listed],
) -> (Vec<ResolvedModel>, Vec<String>) {
    let mut out = Vec::new();
    let mut notices = Vec::new();
    for item in listed {
        match item {
            Listed::Alias(alias) => match catalog.get(alias) {
                Some(def) => out.push(ResolvedModel {
                    alias: Some(alias.clone()),
                    id: def.id.clone(),
                    window: def.window,
                    api: def.api,
                }),
                None => notices.push(format!("unknown alias: {alias}")),
            },
            Listed::Local(def) => out.push(ResolvedModel {
                alias: None,
                id: def.id.clone(),
                window: def.window,
                api: def.api,
            }),
        }
    }
    (out, notices)
}

fn pick_model<'a>(listed: &'a [ResolvedModel], key: &str) -> Option<&'a ResolvedModel> {
    listed
        .iter()
        .find(|m| m.alias.as_deref() == Some(key))
        .or_else(|| listed.iter().find(|m| m.id == key))
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
        providers.insert(
            name,
            ProviderDef {
                base_url: field_string(&t, "base_url"),
                key_name: field_string(&t, "key_name"),
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
    Ok(ModelDef { id, window, api })
}

fn def_error(prefix: &str, err: DefError) -> String {
    match err {
        DefError::NoId => format!("{prefix} has no id"),
        DefError::UnknownApi(Some(raw)) => format!("{prefix} has unknown api: {raw}"),
        DefError::UnknownApi(None) => format!("{prefix} has unknown api"),
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

fn join_notices<'a>(parts: impl Iterator<Item = &'a String>) -> Option<String> {
    let text = parts.cloned().collect::<Vec<_>>().join("\n");
    if text.is_empty() { None } else { Some(text) }
}

fn join_parts(mut parts: Vec<String>, last: String) -> String {
    parts.push(last);
    parts.join("\n")
}

fn nonempty(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|s| !s.is_empty())
}

#[derive(Default)]
struct Guest {
    models: BTreeMap<String, ModelDef>,
    providers: BTreeMap<String, ProviderDef>,
    defaults: Option<RawDefaults>,
    model_notices: Vec<String>,
    provider_notices: Vec<String>,
}

struct RawDefaults {
    provider: Option<String>,
    model: Option<String>,
}

enum DefError {
    NoId,
    UnknownApi(Option<String>),
}

#[derive(Clone)]
struct ModelDef {
    id: String,
    window: Option<u32>,
    api: Api,
}

struct ProviderDef {
    base_url: Option<String>,
    key_name: Option<String>,
    key_in: String,
    auth_provider: Option<String>,
    models: Vec<Listed>,
}

enum Listed {
    Alias(String),
    Local(ModelDef),
}

struct ResolvedModel {
    alias: Option<String>,
    id: String,
    window: Option<u32>,
    api: Api,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::{Mutex, MutexGuard};

    static ENV: Mutex<()> = Mutex::new(());

    struct EnvGuard {
        _lock: MutexGuard<'static, ()>,
        saved: Vec<(String, Option<String>)>,
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (k, v) in &self.saved {
                set_var(k, v.as_deref());
            }
        }
    }

    fn isolate(vars: &[(&str, &str)]) -> EnvGuard {
        let lock = ENV.lock().unwrap_or_else(|e| e.into_inner());
        const KEYS: &[&str] = &[
            "LUNAR_API_KEY",
            "LUNAR_BASE_URL",
            "LUNAR_MODEL",
            "LUNAR_PROVIDER",
            "LUNAR_CONTEXT_WINDOW",
            "XAI_API_KEY",
        ];
        let saved = KEYS
            .iter()
            .map(|k| ((*k).to_string(), std::env::var(k).ok()))
            .collect();
        for k in KEYS {
            set_var(k, None);
        }
        for (k, v) in vars {
            set_var(k, Some(v));
        }
        EnvGuard { _lock: lock, saved }
    }

    fn set_var(key: &str, val: Option<&str>) {
        unsafe {
            match val {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
        }
    }

    fn scratch() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "lunar-lua-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_init(dir: &Path, src: &str) -> std::path::PathBuf {
        let path = dir.join("init.lua");
        fs::write(&path, src).unwrap();
        path
    }

    const SAMPLE: &str = r#"
lunar.models {
  grok46 = { id = "grok-4.6", window = 500000, api = "completions" },
}

lunar.providers {
  xai = {
    base_url = "https://api.x.ai/v1",
    key_name = "XAI_API_KEY",
    models = {
      "grok46",
      { id = "grok-4.5", api = "completions" },
    },
  },
}

lunar.defaults {
  provider = "xai",
  model = "grok46",
}
"#;

    #[test]
    fn missing_file_uses_env() {
        let _e = isolate(&[
            ("LUNAR_API_KEY", "env-key"),
            ("LUNAR_BASE_URL", "https://api.x.ai/v1"),
            ("LUNAR_MODEL", "grok-env"),
            ("LUNAR_PROVIDER", "envp"),
        ]);
        let loaded = load_path(&scratch().join("init.lua"));
        let cfg = loaded.config.unwrap();
        assert_eq!(cfg.model, "grok-env");
        assert_eq!(cfg.api_key, "env-key");
        assert_eq!(cfg.provider(), "envp");
        assert_eq!(loaded.notice, None);
    }

    #[test]
    fn syntax_error_does_not_fall_back_to_env() {
        let _e = isolate(&[
            ("LUNAR_API_KEY", "env-key"),
            ("LUNAR_BASE_URL", "https://api.x.ai/v1"),
            ("LUNAR_MODEL", "grok-env"),
        ]);
        let path = write_init(&scratch(), "this is not lua {");
        let loaded = load_path(&path);
        assert!(loaded.config.is_none());
        assert!(loaded.notice.as_deref().unwrap().starts_with("init.lua:"));
    }

    #[test]
    fn no_defaults_uses_env() {
        let _e = isolate(&[
            ("LUNAR_API_KEY", "env-key"),
            ("LUNAR_BASE_URL", "https://api.x.ai/v1"),
            ("LUNAR_MODEL", "grok-env"),
        ]);
        let path = write_init(
            &scratch(),
            "lunar.models { grok46 = { id = \"grok-4.6\" } }\n",
        );
        let loaded = load_path(&path);
        assert_eq!(loaded.config.unwrap().model, "grok-env");
    }

    #[test]
    fn defaults_resolve_and_ignore_lunar_model() {
        let _e = isolate(&[
            ("LUNAR_API_KEY", "env-key"),
            ("LUNAR_BASE_URL", "https://ignored.example"),
            ("LUNAR_MODEL", "grok-env"),
            ("LUNAR_PROVIDER", "envp"),
            ("LUNAR_CONTEXT_WINDOW", "123"),
            ("XAI_API_KEY", "lua-key"),
        ]);
        let path = write_init(&scratch(), SAMPLE);
        let loaded = load_path(&path);
        let cfg = loaded.config.expect("resolved");
        assert_eq!(cfg.model, "grok-4.6");
        assert_eq!(cfg.api_key, "lua-key");
        assert_eq!(cfg.base_url, "https://api.x.ai/v1");
        assert_eq!(cfg.provider(), "xai");
        assert_eq!(cfg.window, Some(500_000));
        assert_eq!(loaded.notice, None);
    }

    #[test]
    fn model_matches_wire_id() {
        let _e = isolate(&[("XAI_API_KEY", "k")]);
        let src = r#"
lunar.providers {
  xai = {
    base_url = "https://api.x.ai/v1",
    key_name = "XAI_API_KEY",
    models = { { id = "grok-4.5", api = "completions" } },
  },
}
lunar.defaults { provider = "xai", model = "grok-4.5" }
"#;
        let loaded = load_path(&write_init(&scratch(), src));
        let cfg = loaded.config.unwrap();
        assert_eq!(cfg.model, "grok-4.5");
        assert_eq!(cfg.window, Some(500_000));
    }

    #[test]
    fn omitted_api_is_completions_and_can_send() {
        let _e = isolate(&[("XAI_API_KEY", "k")]);
        let src = r#"
lunar.providers {
  xai = {
    base_url = "https://api.x.ai/v1",
    key_name = "XAI_API_KEY",
    models = { { id = "grok-4.5" } },
  },
}
lunar.defaults { provider = "xai", model = "grok-4.5" }
"#;
        let loaded = load_path(&write_init(&scratch(), src));
        let cfg = loaded.config.unwrap();
        assert_eq!(cfg.model, "grok-4.5");
        assert_eq!(cfg.api, Api::Completions);
        assert_eq!(loaded.notice, None);
        assert!(loaded.models[0].config.is_some());
    }

    #[test]
    fn unknown_api_skips_entry() {
        let _e = isolate(&[("XAI_API_KEY", "k")]);
        let src = r#"
lunar.models {
  grok46 = { id = "grok-4.6", api = "chat" },
}
lunar.providers {
  xai = {
    base_url = "https://api.x.ai/v1",
    key_name = "XAI_API_KEY",
    models = { "grok46", { id = "grok-4.5", api = "completions" } },
  },
}
lunar.defaults { provider = "xai", model = "grok46" }
"#;
        let loaded = load_path(&write_init(&scratch(), src));
        assert!(loaded.config.is_none());
        assert_eq!(
            loaded.notice.as_deref(),
            Some(
                "model grok46 has unknown api: chat\nunknown alias: grok46\nunknown model: grok46"
            )
        );
        assert_eq!(loaded.models.len(), 1);
        assert_eq!(loaded.models[0].id, "grok-4.5");
        assert!(loaded.models[0].config.is_some());
    }

    #[test]
    fn responses_default_resolves() {
        let _e = isolate(&[("XAI_API_KEY", "k")]);
        let src = r#"
lunar.models {
  gpt = { id = "gpt-5", api = "responses" },
  grok46 = { id = "grok-4.6", api = "completions" },
}
lunar.providers {
  openai = {
    base_url = "https://api.openai.com/v1",
    key_name = "XAI_API_KEY",
    models = { "gpt", "grok46" },
  },
}
lunar.defaults { provider = "openai", model = "gpt" }
"#;
        let loaded = load_path(&write_init(&scratch(), src));
        let cfg = loaded.config.unwrap();
        assert_eq!(cfg.model, "gpt-5");
        assert_eq!(cfg.api, Api::Responses);
        assert_eq!(loaded.models.len(), 2);
        assert!(loaded.models.iter().all(|m| m.config.is_some()));
    }

    #[test]
    fn messages_api_cannot_send() {
        let _e = isolate(&[("XAI_API_KEY", "k")]);
        let src = r#"
lunar.providers {
  anthropic = {
    base_url = "https://api.anthropic.com",
    key_name = "XAI_API_KEY",
    models = { { id = "claude-opus-4", api = "messages" } },
  },
}
lunar.defaults { provider = "anthropic", model = "claude-opus-4" }
"#;
        let loaded = load_path(&write_init(&scratch(), src));
        assert!(loaded.config.is_none());
        assert_eq!(
            loaded.notice.as_deref(),
            Some("claude-opus-4 uses messages, not implemented")
        );
    }

    #[test]
    fn string_ref_inherits_catalog_api() {
        let _e = isolate(&[("XAI_API_KEY", "k")]);
        let src = r#"
lunar.models {
  grok46 = { id = "grok-4.6", api = "completions" },
}
lunar.providers {
  xai = {
    base_url = "https://api.x.ai/v1",
    key_name = "XAI_API_KEY",
    models = { "grok46" },
  },
}
lunar.defaults { provider = "xai", model = "grok46" }
"#;
        let loaded = load_path(&write_init(&scratch(), src));
        assert_eq!(loaded.config.unwrap().model, "grok-4.6");
        assert_eq!(loaded.notice, None);
    }

    #[test]
    fn local_def_can_differ_from_catalog() {
        let _e = isolate(&[("XAI_API_KEY", "k")]);
        let src = r#"
lunar.models {
  gpt = { id = "gpt-5", api = "responses" },
}
lunar.providers {
  openai = {
    base_url = "https://api.openai.com/v1",
    key_name = "XAI_API_KEY",
    models = { "gpt" },
  },
  proxy = {
    base_url = "https://proxy.example/v1",
    key_name = "XAI_API_KEY",
    models = { { id = "gpt-5", api = "completions" } },
  },
}
lunar.defaults { provider = "proxy", model = "gpt-5" }
"#;
        let loaded = load_path(&write_init(&scratch(), src));
        let cfg = loaded.config.unwrap();
        assert_eq!(cfg.model, "gpt-5");
        assert_eq!(cfg.provider(), "proxy");
        assert_eq!(cfg.api, Api::Completions);
        assert_eq!(loaded.models.len(), 2);
        let openai = loaded
            .models
            .iter()
            .find(|m| m.provider == "openai")
            .unwrap();
        assert_eq!(openai.config.as_ref().unwrap().api, Api::Responses);
    }

    #[test]
    fn partial_defaults_cannot_send() {
        let _e = isolate(&[
            ("LUNAR_API_KEY", "env-key"),
            ("LUNAR_BASE_URL", "https://api.x.ai/v1"),
            ("LUNAR_MODEL", "grok-env"),
            ("XAI_API_KEY", "k"),
        ]);
        let src = r#"
lunar.providers {
  xai = { base_url = "https://api.x.ai/v1", key_name = "XAI_API_KEY", models = { { id = "grok-4.6", api = "completions" } } },
}
lunar.defaults { provider = "xai" }
"#;
        let loaded = load_path(&write_init(&scratch(), src));
        assert!(loaded.config.is_none());
        assert_eq!(
            loaded.notice.as_deref(),
            Some("lunar.defaults needs provider and model")
        );
    }

    #[test]
    fn unknown_provider() {
        let _e = isolate(&[("XAI_API_KEY", "k")]);
        let src = r#"
lunar.defaults { provider = "nope", model = "grok-4.6" }
"#;
        let loaded = load_path(&write_init(&scratch(), src));
        assert!(loaded.config.is_none());
        assert_eq!(loaded.notice.as_deref(), Some("unknown provider: nope"));
    }

    #[test]
    fn unknown_model() {
        let _e = isolate(&[("XAI_API_KEY", "k")]);
        let src = r#"
lunar.providers {
  xai = { base_url = "https://api.x.ai/v1", key_name = "XAI_API_KEY", models = { { id = "grok-4.6", api = "completions" } } },
}
lunar.defaults { provider = "xai", model = "missing" }
"#;
        let loaded = load_path(&write_init(&scratch(), src));
        assert!(loaded.config.is_none());
        assert_eq!(loaded.notice.as_deref(), Some("unknown model: missing"));
    }

    #[test]
    fn missing_base_url() {
        let _e = isolate(&[("XAI_API_KEY", "k")]);
        let src = r#"
lunar.providers {
  xai = { key_name = "XAI_API_KEY", models = { { id = "grok-4.6", api = "completions" } } },
}
lunar.defaults { provider = "xai", model = "grok-4.6" }
"#;
        let loaded = load_path(&write_init(&scratch(), src));
        assert!(loaded.config.is_none());
        assert_eq!(loaded.notice.as_deref(), Some("xai has no base_url"));
    }

    #[test]
    fn missing_secret() {
        let _e = isolate(&[]);
        let src = r#"
lunar.providers {
  xai = { base_url = "https://api.x.ai/v1", key_name = "XAI_API_KEY", models = { { id = "grok-4.6", api = "completions" } } },
}
lunar.defaults { provider = "xai", model = "grok-4.6" }
"#;
        let loaded = load_path(&write_init(&scratch(), src));
        assert!(loaded.config.is_none());
        assert_eq!(loaded.notice.as_deref(), Some("missing XAI_API_KEY"));
    }

    #[test]
    fn key_in_must_be_env() {
        let _e = isolate(&[("XAI_API_KEY", "k")]);
        let src = r#"
lunar.providers {
  xai = {
    base_url = "https://api.x.ai/v1",
    key_name = "XAI_API_KEY",
    key_in = "file",
    models = { { id = "grok-4.6", api = "completions" } },
  },
}
lunar.defaults { provider = "xai", model = "grok-4.6" }
"#;
        let loaded = load_path(&write_init(&scratch(), src));
        assert!(loaded.config.is_none());
        assert_eq!(
            loaded.notice.as_deref(),
            Some("xai key_in is not env or auth")
        );
    }

    #[test]
    fn missing_alias_is_skipped() {
        let _e = isolate(&[("XAI_API_KEY", "k")]);
        let src = r#"
lunar.models {
  grok46 = { id = "grok-4.6", api = "completions" },
}
lunar.providers {
  xai = {
    base_url = "https://api.x.ai/v1",
    key_name = "XAI_API_KEY",
    models = { "nope", "grok46" },
  },
}
lunar.defaults { provider = "xai", model = "grok46" }
"#;
        let loaded = load_path(&write_init(&scratch(), src));
        let cfg = loaded.config.unwrap();
        assert_eq!(cfg.model, "grok-4.6");
        assert_eq!(loaded.notice.as_deref(), Some("unknown alias: nope"));
    }

    #[test]
    fn last_call_wins() {
        let _e = isolate(&[("XAI_API_KEY", "k")]);
        let src = r#"
lunar.models { grok46 = { id = "grok-4.6", api = "completions" } }
lunar.models { grok45 = { id = "grok-4.5", api = "completions" } }
lunar.providers {
  xai = { base_url = "https://api.x.ai/v1", key_name = "XAI_API_KEY", models = { "grok45" } },
}
lunar.defaults { provider = "xai", model = "grok46" }
lunar.defaults { provider = "xai", model = "grok45" }
"#;
        let loaded = load_path(&write_init(&scratch(), src));
        assert_eq!(loaded.config.unwrap().model, "grok-4.5");
    }

    #[test]
    fn runtime_error_does_not_fall_back() {
        let _e = isolate(&[
            ("LUNAR_API_KEY", "env-key"),
            ("LUNAR_BASE_URL", "https://api.x.ai/v1"),
            ("LUNAR_MODEL", "grok-env"),
        ]);
        let path = write_init(&scratch(), "error('boom')\n");
        let loaded = load_path(&path);
        assert!(loaded.config.is_none());
        assert!(loaded.notice.unwrap().contains("boom"));
    }
}
