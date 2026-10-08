use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "lunar-startup-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("home/control")).unwrap();
        fs::create_dir_all(root.join("project/.lunar")).unwrap();
        Self(root)
    }

    fn rejects(&self, source: &str, expected: &str) -> String {
        let output = Command::new(env!("CARGO_BIN_EXE_lunar"))
            .current_dir(self.0.join("project"))
            .env("LUNAR_HOME", self.0.join("home"))
            .env("HOME", &self.0)
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(1), "{stderr}");
        assert!(output.stdout.is_empty());
        assert!(
            stderr.starts_with("Could not load Lunar configuration\n\n  File: "),
            "{stderr}"
        );
        assert!(
            stderr.contains(&self.0.join(source).display().to_string()),
            "{stderr}"
        );
        assert!(stderr.contains(expected), "{stderr}");
        assert!(!stderr.contains('\x1b'), "entered terminal mode: {stderr}");
        assert!(!stderr.contains("panicked"), "{stderr}");
        assert!(stderr.ends_with("Fix this file and restart Lunar.\n"));
        stderr.into_owned()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn config_errors_exit_before_terminal_initialization() {
    for source in ["home/control/init.lua", "project/.lunar/init.lua"] {
        for (lua, expected) in [
            ("return {", "syntax error"),
            ("error('broken config')", "broken config"),
            ("return require('missing_module')", "missing_module"),
            ("return false", "must return a table"),
            (
                "return { providers = { p = { key_in = 'none', base_url = 'http://localhost', models = { { id = 'm' } } } }, defaults = { provider = 'p', model = 'typo' } }",
                "unknown model",
            ),
            ("return { stack = false }", "stack is not a table"),
            ("return { models = { broken = {} } }", "has no id"),
            (
                "return { models = { broken = { id = 'm', api = 'typo' } } }",
                "unknown api",
            ),
            (
                "return { providers = { p = { models = { 'typo' } } } }",
                "unknown alias",
            ),
            (
                "return { defaults = { provider = 'p' } }",
                "defaults needs provider and model",
            ),
            (
                "return { defaults = { provider = 'typo', model = 'm' } }",
                "unknown provider",
            ),
        ] {
            let fixture = Fixture::new();
            fs::write(fixture.0.join(source), lua).unwrap();
            fixture.rejects(source, expected);
        }
    }
}

#[test]
fn unreadable_config_is_not_treated_as_missing() {
    for source in ["home/control/init.lua", "project/.lunar/init.lua"] {
        let fixture = Fixture::new();
        fs::create_dir(fixture.0.join(source)).unwrap();
        fixture.rejects(source, "directory");
    }
}

#[test]
fn valid_project_does_not_hide_broken_user_config() {
    let fixture = Fixture::new();
    fs::write(
        fixture.0.join("home/control/init.lua"),
        "return { models = { m = {} } }",
    )
    .unwrap();
    fs::write(
        fixture.0.join("project/.lunar/init.lua"),
        "return { models = { m = { id = 'ok' } } }",
    )
    .unwrap();
    fixture.rejects("home/control/init.lua", "has no id");
}

#[test]
fn syntax_error_shows_lua_line_and_source_without_repeated_paths() {
    let fixture = Fixture::new();
    let source = "project/.lunar/init.lua";
    fs::write(
        fixture.0.join(source),
        "return {\n  defaults = { provider = 'p' model = 'm' },\n}\n",
    )
    .unwrap();
    let stderr = fixture.rejects(source, "expected near 'model'");
    assert!(stderr.contains("  Line: 2\n"), "{stderr}");
    assert!(stderr.contains("  2 |   defaults = { provider = 'p' model = 'm' },"));
    assert_eq!(stderr.matches("init.lua").count(), 1, "{stderr}");
}

#[test]
fn invalid_defaults_show_field_and_available_choices() {
    let fixture = Fixture::new();
    let source = "project/.lunar/init.lua";
    for (defaults, field, choices) in [
        (
            "provider = 'typo', model = 'm'",
            "defaults.provider",
            "Available providers: local_api",
        ),
        (
            "provider = 'local_api', model = 'typo'",
            "defaults.model",
            "Available models for local_api: fast (wire-model)",
        ),
    ] {
        fs::write(fixture.0.join(source), format!("return {{ models = {{ fast = {{ id = 'wire-model' }} }}, providers = {{ local_api = {{ key_in = 'none', base_url = 'http://localhost', models = {{ 'fast' }} }} }}, defaults = {{ {defaults} }} }}")).unwrap();
        let stderr = fixture.rejects(source, choices);
        assert!(stderr.contains(&format!("  Field: {field}\n")), "{stderr}");
        assert!(!stderr.contains("Line:"));
    }
}
