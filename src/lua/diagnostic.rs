//! Presentation only: Lua still parses the configuration and supplies diagnostics.

use std::fmt::Write as _;
use std::path::Path;

pub(super) fn config(path: &Path, field: Option<&str>, message: &str) -> String {
    render(path, field, None, message, None)
}

pub(super) fn lua(path: &Path, source: &str, error: mlua::Error) -> String {
    let (kind, message) = match error {
        mlua::Error::SyntaxError { message, .. } => ("syntax error", message),
        mlua::Error::RuntimeError(message) => ("runtime error", message),
        other => return config(path, None, &other.to_string()),
    };
    match location(path, &message) {
        Some((line, detail)) => render(
            path,
            None,
            Some(line),
            &format!("{kind}: {detail}"),
            source.lines().nth(line - 1),
        ),
        None => config(path, None, &format!("{kind}: {message}")),
    }
}

fn render(
    path: &Path,
    field: Option<&str>,
    line: Option<usize>,
    message: &str,
    source_line: Option<&str>,
) -> String {
    let mut out = format!(
        "Could not load Lunar configuration\n\n  File: {}",
        path.display()
    );
    if let Some(field) = field {
        let _ = write!(out, "\n  Field: {field}");
    }
    if let Some(line) = line {
        let _ = write!(out, "\n  Line: {line}");
    }
    let _ = write!(out, "\n  Error: {}", message.replace('\n', "\n         "));
    if let (Some(line), Some(source_line)) = (line, source_line) {
        let _ = write!(out, "\n\n  {line} | {source_line}");
    }
    out
}

// Only recognize a location at the start of Lua's message for this file.
// A module location or an unfamiliar diagnostic stays intact, not misattributed.
fn location<'a>(path: &Path, message: &'a str) -> Option<(usize, &'a str)> {
    let first_line = message.lines().next()?;
    let path = path.to_string_lossy();
    for (index, _) in first_line.match_indices(':') {
        let reported = &first_line[..index];
        let matches = reported == path
            || reported
                .strip_prefix("...")
                .is_some_and(|suffix| suffix.contains('/') && path.ends_with(suffix));
        if !matches {
            continue;
        }
        let (number, detail) = message[index + 1..].split_once(':')?;
        let line = number.parse::<usize>().ok().filter(|line| *line > 0)?;
        return Some((line, detail.trim_start()));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortened_lua_path_shows_the_actual_source_line() {
        let path = Path::new("/long/home/control/init.lua");
        let error = mlua::Error::SyntaxError {
            message: ".../control/init.lua:2: '}' expected near 'model'".into(),
            incomplete_input: false,
        };
        let output = lua(path, "return {\n  provider = 'p' model = 'm'\n}", error);
        assert!(output.contains("File: /long/home/control/init.lua"));
        assert!(output.contains("Line: 2"));
        assert!(output.contains("Error: syntax error: '}' expected near 'model'"));
        assert!(output.contains("2 |   provider = 'p' model = 'm'"));
        assert!(!output.contains(".../control"));
    }

    #[test]
    fn unfamiliar_and_module_locations_preserve_the_original_diagnostic() {
        for message in [
            "custom failure:42: details",
            "/other/module.lua:2: broken module\nstack traceback:\n  init.lua:1",
            "/config/init.lua:0: unknown line",
        ] {
            let output = lua(
                Path::new("/config/init.lua"),
                "return {}",
                mlua::Error::RuntimeError(message.into()),
            );
            assert!(output.contains(&message.replace('\n', "\n         ")));
            assert!(!output.contains("Line:"));
            assert!(!output.contains(" | "));
        }
    }

    #[test]
    fn eof_location_does_not_invent_a_source_line() {
        let output = lua(
            Path::new("/config/init.lua"),
            "return {\n",
            mlua::Error::SyntaxError {
                message: "/config/init.lua:2: '}' expected near <eof>".into(),
                incomplete_input: true,
            },
        );
        assert!(output.contains("Line: 2"));
        assert!(!output.contains(" | "));
    }
}
