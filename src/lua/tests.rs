use super::*;
use crate::protocol::{Api, Thinking};
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
    const KEYS: &[&str] = &["LUNAR_HOME", "XAI_API_KEY"];
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
return {
  models = {
  grok46 = { id = "grok-4.6", window = 500000, api = "completions" },
},
  providers = {
  xai = {
    base_url = "https://api.x.ai/v1",
    key_name = "XAI_API_KEY",
    models = {
      "grok46",
      { id = "grok-4.5", api = "completions" },
    },
  },
},
  defaults = {
  provider = "xai",
  model = "grok46",
},
}
"#;

#[test]
fn project_config_overrides_user_config_by_key() {
    let _e = isolate(&[("XAI_API_KEY", "k")]);
    let user_dir = scratch();
    let project_dir = scratch();
    let user = write_init(
        &user_dir,
        r#"return {
  models = {
    shared = { id = "global-model" },
    global_only = { id = "global-only" },
  },
  providers = {
    global = {
      base_url = "https://global.example/v1",
      key_name = "XAI_API_KEY",
      models = { "global_only" },
    },
    shared = {
      base_url = "https://old.example/v1",
      key_name = "XAI_API_KEY",
      models = { "shared" },
    },
  },
  defaults = { provider = "global", model = "global_only" },
}"#,
    );
    let project = write_init(
        &project_dir,
        r#"return {
  models = { shared = { id = "project-model", api = "responses" } },
  providers = {
    shared = {
      base_url = "https://project.example/v1",
      key_name = "XAI_API_KEY",
      models = { "shared" },
    },
  },
  defaults = { provider = "shared", model = "shared" },
}"#,
    );

    let loaded = load_paths(&user, &project);
    let config = loaded.config.unwrap();
    assert_eq!(config.model, "project-model");
    assert_eq!(config.base_url, "https://project.example/v1");
    assert_eq!(config.api, Api::Responses);
    assert_eq!(loaded.models.len(), 2);
    assert!(
        loaded
            .models
            .iter()
            .any(|choice| choice.provider == "global")
    );
}

#[test]
fn project_without_defaults_inherits_user_defaults() {
    let _e = isolate(&[("XAI_API_KEY", "k")]);
    let user_dir = scratch();
    let project_dir = scratch();
    let user = write_init(&user_dir, SAMPLE);
    let project = write_init(
        &project_dir,
        r#"return { models = { extra = { id = "extra" } } }"#,
    );

    let loaded = load_paths(&user, &project);
    assert_eq!(loaded.config.unwrap().model, "grok-4.6");
}

#[test]
fn missing_file_is_unconfigured() {
    let _e = isolate(&[]);
    let loaded = load_path(&scratch().join("init.lua"));
    assert!(loaded.config.is_none());
    assert!(loaded.models.is_empty());
    assert_eq!(loaded.notice, None);
}

#[test]
fn syntax_error_cannot_send() {
    let _e = isolate(&[]);
    let path = write_init(&scratch(), "this is not lua {");
    let loaded = load_path(&path);
    assert!(loaded.config.is_none());
    assert!(loaded.notice.as_deref().unwrap().starts_with("init.lua:"));
}

#[test]
fn no_defaults_has_catalog_but_cannot_send() {
    let _e = isolate(&[]);
    let path = write_init(
        &scratch(),
        r#"return {
  models = { grok46 = { id = "grok-4.6" } },
  providers = {
    xai = {
      base_url = "https://api.x.ai/v1",
      key_name = "XAI_API_KEY",
      models = { "grok46" },
    },
  },
}"#,
    );
    let loaded = load_path(&path);
    assert!(loaded.config.is_none());
    assert_eq!(loaded.models.len(), 1);
}

#[test]
fn defaults_resolve_from_lua() {
    let _e = isolate(&[("XAI_API_KEY", "lua-key")]);
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
return {
  providers = {
  xai = {
    base_url = "https://api.x.ai/v1",
    key_name = "XAI_API_KEY",
    models = { { id = "grok-4.5", api = "completions" } },
  },
},
  defaults = { provider = "xai", model = "grok-4.5" },
}
"#;
    let loaded = load_path(&write_init(&scratch(), src));
    let cfg = loaded.config.unwrap();
    assert_eq!(cfg.model, "grok-4.5");
    assert_eq!(cfg.window, Some(500_000));
}

#[test]
fn model_thinking_overrides_provider_thinking() {
    let _env = isolate(&[("XAI_API_KEY", "key")]);
    let dir = scratch();
    let path = write_init(
        &dir,
        r#"
return {
  models = {
  grok = { id = "grok", thinking = "high" },
},
  providers = {
  xai = {
    base_url = "https://api.x.ai/v1",
    key_name = "XAI_API_KEY",
    thinking = "low",
    models = { "grok", { id = "other" } },
  },
},
  defaults = { provider = "xai", model = "grok" },
}
"#,
    );
    let loaded = load_path(&path);
    assert_eq!(loaded.config.unwrap().thinking, Thinking::High);
    let other = loaded.models.iter().find(|m| m.id == "other").unwrap();
    assert_eq!(other.config.as_ref().unwrap().thinking, Thinking::Low);
}

#[test]
fn omitted_api_is_completions_and_can_send() {
    let _e = isolate(&[("XAI_API_KEY", "k")]);
    let src = r#"
return {
  providers = {
  xai = {
    base_url = "https://api.x.ai/v1",
    key_name = "XAI_API_KEY",
    models = { { id = "grok-4.5" } },
  },
},
  defaults = { provider = "xai", model = "grok-4.5" },
}
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
return {
  models = {
  grok46 = { id = "grok-4.6", api = "chat" },
},
  providers = {
  xai = {
    base_url = "https://api.x.ai/v1",
    key_name = "XAI_API_KEY",
    models = { "grok46", { id = "grok-4.5", api = "completions" } },
  },
},
  defaults = { provider = "xai", model = "grok46" },
}
"#;
    let loaded = load_path(&write_init(&scratch(), src));
    assert!(loaded.config.is_none());
    assert_eq!(
        loaded.notice.as_deref(),
        Some("model grok46 has unknown api: chat\nunknown alias: grok46\nunknown model: grok46")
    );
    assert_eq!(loaded.models.len(), 1);
    assert_eq!(loaded.models[0].id, "grok-4.5");
    assert!(loaded.models[0].config.is_some());
}

#[test]
fn responses_default_resolves() {
    let _e = isolate(&[("XAI_API_KEY", "k")]);
    let src = r#"
return {
  models = {
  gpt = { id = "gpt-5", api = "responses" },
  grok46 = { id = "grok-4.6", api = "completions" },
},
  providers = {
  openai = {
    base_url = "https://api.openai.com/v1",
    key_name = "XAI_API_KEY",
    models = { "gpt", "grok46" },
  },
},
  defaults = { provider = "openai", model = "gpt" },
}
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
return {
  providers = {
  anthropic = {
    base_url = "https://api.anthropic.com",
    key_name = "XAI_API_KEY",
    models = { { id = "claude-opus-4", api = "messages" } },
  },
},
  defaults = { provider = "anthropic", model = "claude-opus-4" },
}
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
return {
  models = {
  grok46 = { id = "grok-4.6", api = "completions" },
},
  providers = {
  xai = {
    base_url = "https://api.x.ai/v1",
    key_name = "XAI_API_KEY",
    models = { "grok46" },
  },
},
  defaults = { provider = "xai", model = "grok46" },
}
"#;
    let loaded = load_path(&write_init(&scratch(), src));
    assert_eq!(loaded.config.unwrap().model, "grok-4.6");
    assert_eq!(loaded.notice, None);
}

#[test]
fn local_def_can_differ_from_catalog() {
    let _e = isolate(&[("XAI_API_KEY", "k")]);
    let src = r#"
return {
  models = {
  gpt = { id = "gpt-5", api = "responses" },
},
  providers = {
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
},
  defaults = { provider = "proxy", model = "gpt-5" },
}
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
    let _e = isolate(&[("XAI_API_KEY", "k")]);
    let src = r#"
return {
  providers = {
  xai = { base_url = "https://api.x.ai/v1", key_name = "XAI_API_KEY", models = { { id = "grok-4.6", api = "completions" } } },
},
  defaults = { provider = "xai" },
}
"#;
    let loaded = load_path(&write_init(&scratch(), src));
    assert!(loaded.config.is_none());
    assert_eq!(
        loaded.notice.as_deref(),
        Some("defaults needs provider and model")
    );
}

#[test]
fn unknown_provider() {
    let _e = isolate(&[("XAI_API_KEY", "k")]);
    let src = r#"
return {
  defaults = { provider = "nope", model = "grok-4.6" },
}
"#;
    let loaded = load_path(&write_init(&scratch(), src));
    assert!(loaded.config.is_none());
    assert_eq!(loaded.notice.as_deref(), Some("unknown provider: nope"));
}

#[test]
fn unknown_model() {
    let _e = isolate(&[("XAI_API_KEY", "k")]);
    let src = r#"
return {
  providers = {
  xai = { base_url = "https://api.x.ai/v1", key_name = "XAI_API_KEY", models = { { id = "grok-4.6", api = "completions" } } },
},
  defaults = { provider = "xai", model = "missing" },
}
"#;
    let loaded = load_path(&write_init(&scratch(), src));
    assert!(loaded.config.is_none());
    assert_eq!(loaded.notice.as_deref(), Some("unknown model: missing"));
}

#[test]
fn missing_base_url() {
    let _e = isolate(&[("XAI_API_KEY", "k")]);
    let src = r#"
return {
  providers = {
  xai = { key_name = "XAI_API_KEY", models = { { id = "grok-4.6", api = "completions" } } },
},
  defaults = { provider = "xai", model = "grok-4.6" },
}
"#;
    let loaded = load_path(&write_init(&scratch(), src));
    assert!(loaded.config.is_none());
    assert_eq!(
        loaded.notice.as_deref(),
        Some("xai has no base_url or base_url_cmd")
    );
}

#[test]
fn base_url_cmd_supplies_base_url_and_wins() {
    let _e = isolate(&[("XAI_API_KEY", "k")]);
    let src = r#"
return {
  providers = {
  xai = {
    base_url = "https://ignored.example",
    base_url_cmd = "printf 'https://api.x.ai/v1\\n'",
    key_name = "XAI_API_KEY",
    models = { { id = "grok-4.6", api = "completions" } },
  },
},
  defaults = { provider = "xai", model = "grok-4.6" },
}
"#;
    let loaded = load_path(&write_init(&scratch(), src));
    let config = loaded.config.unwrap();
    assert_eq!(config.base_url, "https://api.x.ai/v1");
    assert_eq!(loaded.notice, None);
}

#[test]
fn failing_base_url_cmd_cannot_send() {
    let _e = isolate(&[("XAI_API_KEY", "k")]);
    let src = r#"
return {
  providers = {
  xai = {
    base_url = "https://api.x.ai/v1",
    base_url_cmd = "exit 9",
    key_name = "XAI_API_KEY",
    models = { { id = "grok-4.6", api = "completions" } },
  },
},
  defaults = { provider = "xai", model = "grok-4.6" },
}
"#;
    let loaded = load_path(&write_init(&scratch(), src));
    assert!(loaded.config.is_none());
    assert_eq!(
        loaded.notice.as_deref(),
        Some("xai base_url_cmd failed with exit status: 9")
    );
}

#[test]
fn key_cmd_supplies_secret() {
    let _e = isolate(&[]);
    let src = r#"
return {
  providers = {
  xai = {
    base_url = "https://api.x.ai/v1",
    key_cmd = "printf 'command-key\\n'",
    models = { { id = "grok-4.6", api = "completions" } },
  },
},
  defaults = { provider = "xai", model = "grok-4.6" },
}
"#;
    let loaded = load_path(&write_init(&scratch(), src));
    assert_eq!(loaded.config.unwrap().api_key, "command-key");
    assert_eq!(loaded.notice, None);
}

#[test]
fn failing_key_cmd_cannot_send() {
    let _e = isolate(&[]);
    let src = r#"
return {
  providers = {
  xai = {
    base_url = "https://api.x.ai/v1",
    key_cmd = "exit 7",
    models = { { id = "grok-4.6", api = "completions" } },
  },
},
  defaults = { provider = "xai", model = "grok-4.6" },
}
"#;
    let loaded = load_path(&write_init(&scratch(), src));
    assert!(loaded.config.is_none());
    assert_eq!(
        loaded.notice.as_deref(),
        Some("xai key_cmd failed with exit status: 7")
    );
}

#[test]
fn missing_secret() {
    let _e = isolate(&[]);
    let src = r#"
return {
  providers = {
  xai = { base_url = "https://api.x.ai/v1", key_name = "XAI_API_KEY", models = { { id = "grok-4.6", api = "completions" } } },
},
  defaults = { provider = "xai", model = "grok-4.6" },
}
"#;
    let loaded = load_path(&write_init(&scratch(), src));
    assert!(loaded.config.is_none());
    assert_eq!(loaded.notice.as_deref(), Some("missing XAI_API_KEY"));
}

#[test]
fn key_in_must_be_known() {
    let _e = isolate(&[("XAI_API_KEY", "k")]);
    let src = r#"
return {
  providers = {
  xai = {
    base_url = "https://api.x.ai/v1",
    key_name = "XAI_API_KEY",
    key_in = "file",
    models = { { id = "grok-4.6", api = "completions" } },
  },
},
  defaults = { provider = "xai", model = "grok-4.6" },
}
"#;
    let loaded = load_path(&write_init(&scratch(), src));
    assert!(loaded.config.is_none());
    assert_eq!(
        loaded.notice.as_deref(),
        Some("xai key_in is not env, auth, or none")
    );
}

#[test]
fn key_in_none_allows_http_without_a_secret() {
    let _e = isolate(&[]);
    let src = r#"
return {
  providers = {
    ollama = {
      base_url = "http://localhost:11434/v1",
      key_in = "none",
      models = { { id = "qwen3", api = "completions" } },
    },
  },
  defaults = { provider = "ollama", model = "qwen3" },
}
"#;
    let loaded = load_path(&write_init(&scratch(), src));
    let config = loaded.config.unwrap();
    assert_eq!(config.base_url, "http://localhost:11434/v1");
    assert_eq!(config.api_key, "");
    assert_eq!(config.auth_provider, None);
    assert_eq!(loaded.notice, None);
}

#[test]
fn key_in_none_requires_explicit_base_url() {
    let _e = isolate(&[]);
    let src = r#"
return {
  providers = {
    ollama = {
      key_in = "none",
      models = { { id = "qwen3", api = "completions" } },
    },
  },
  defaults = { provider = "ollama", model = "qwen3" },
}
"#;
    let loaded = load_path(&write_init(&scratch(), src));
    assert!(loaded.config.is_none());
    assert_eq!(loaded.notice.as_deref(), Some("ollama has no base_url"));
}

#[test]
fn auth_provider_must_be_builtin() {
    let _e = isolate(&[]);
    let src = r#"
return {
  providers = {
  typo = {
    key_in = "auth",
    auth_provider = "opneai",
    models = { { id = "gpt-5.4", api = "responses" } },
  },
},
  defaults = { provider = "typo", model = "gpt-5.4" },
}
"#;
    let loaded = load_path(&write_init(&scratch(), src));
    assert!(loaded.config.is_none());
    assert_eq!(
        loaded.notice.as_deref(),
        Some("typo has unknown auth_provider: opneai")
    );
}

#[test]
fn missing_alias_is_skipped() {
    let _e = isolate(&[("XAI_API_KEY", "k")]);
    let src = r#"
return {
  models = {
  grok46 = { id = "grok-4.6", api = "completions" },
},
  providers = {
  xai = {
    base_url = "https://api.x.ai/v1",
    key_name = "XAI_API_KEY",
    models = { "nope", "grok46" },
  },
},
  defaults = { provider = "xai", model = "grok46" },
}
"#;
    let loaded = load_path(&write_init(&scratch(), src));
    let cfg = loaded.config.unwrap();
    assert_eq!(cfg.model, "grok-4.6");
    assert_eq!(loaded.notice.as_deref(), Some("unknown alias: nope"));
}

#[test]
fn duplicate_table_keys_use_the_last_value() {
    let _e = isolate(&[("XAI_API_KEY", "k")]);
    let src = r#"
return {
  models = { grok46 = { id = "grok-4.6", api = "completions" } },
  models = { grok45 = { id = "grok-4.5", api = "completions" } },
  providers = {
  xai = { base_url = "https://api.x.ai/v1", key_name = "XAI_API_KEY", models = { "grok45" } },
},
  defaults = { provider = "xai", model = "grok46" },
  defaults = { provider = "xai", model = "grok45" },
}
"#;
    let loaded = load_path(&write_init(&scratch(), src));
    assert_eq!(loaded.config.unwrap().model, "grok-4.5");
}

#[test]
fn auth_provider_fills_omitted_base_url() {
    assert_eq!(
        resolve::default_auth_base(Some("openai")),
        Some("https://chatgpt.com/backend-api")
    );
    assert_eq!(
        resolve::default_auth_base(Some("xai")),
        Some("https://api.x.ai/v1")
    );
    assert_eq!(resolve::default_auth_base(Some("other")), None);
    assert_eq!(resolve::default_auth_base(None), None);
}

#[test]
fn openai_auth_completions_cannot_send() {
    let _e = isolate(&[]);
    let src = r#"
return {
  providers = {
  openai = {
    base_url = "https://chatgpt.com/backend-api",
    key_in = "auth",
    auth_provider = "openai",
    models = { { id = "gpt-5.4", api = "completions" } },
  },
},
  defaults = { provider = "openai", model = "gpt-5.4" },
}
"#;
    let loaded = load_path(&write_init(&scratch(), src));
    assert!(loaded.config.is_none());
    assert_eq!(
        loaded.notice.as_deref(),
        Some("gpt-5.4 uses completions, not implemented")
    );
}

#[test]
fn non_table_return_cannot_send() {
    let _e = isolate(&[]);
    let loaded = load_path(&write_init(&scratch(), "return nil\n"));
    assert!(loaded.config.is_none());
    assert_eq!(
        loaded.notice.as_deref(),
        Some("init.lua must return a table")
    );
}

#[test]
fn registrar_form_is_not_supported() {
    let _e = isolate(&[]);
    let loaded = load_path(&write_init(
        &scratch(),
        "lunar.models { grok = { id = 'grok' } }\n",
    ));
    assert!(loaded.config.is_none());
    assert!(loaded.notice.unwrap().contains("global 'lunar'"));
}

#[test]
fn runtime_error_cannot_send() {
    let _e = isolate(&[]);
    let path = write_init(&scratch(), "error('boom')\n");
    let loaded = load_path(&path);
    assert!(loaded.config.is_none());
    assert!(loaded.notice.unwrap().contains("boom"));
}
