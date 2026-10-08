//! Lightweight repository references from Lua; never reads component contents.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use mlua::{Table, Value};

pub(crate) struct Component {
    location: PathBuf,
    notes: String,
}

pub(crate) fn parse(table: &Table, base: &Path) -> Result<BTreeMap<String, Component>, String> {
    let stack = match table.get::<Value>("stack") {
        Ok(Value::Nil) => return Ok(BTreeMap::new()),
        Ok(Value::Table(stack)) => stack,
        _ => return Err("init.lua stack is not a table".into()),
    };
    let components = match stack.get::<Value>("components") {
        Ok(Value::Nil) => return Ok(BTreeMap::new()),
        Ok(Value::Table(components)) => components,
        _ => return Err("init.lua stack.components is not a table".into()),
    };
    let mut result = BTreeMap::new();
    for pair in components.pairs::<String, Table>() {
        let (name, component) =
            pair.map_err(|_| "init.lua stack.components needs named component tables".to_string())?;
        if name.trim().is_empty() {
            return Err("init.lua stack component name is empty".into());
        }
        let location = match component.get::<Value>("location") {
            Ok(Value::String(value)) => value.to_string_lossy(),
            _ => return Err(format!("stack component {name}: location must be a string")),
        };
        if location.trim().is_empty() {
            return Err(format!("stack component {name}: location is empty"));
        }
        let notes = match component.get::<Value>("notes") {
            Ok(Value::Nil) => String::new(),
            Ok(Value::String(value)) => value.to_string_lossy(),
            _ => return Err(format!("stack component {name}: notes must be a string")),
        };
        let location = if location == "~" || location.starts_with("~/") {
            let home = std::env::var_os("HOME")
                .filter(|home| !home.is_empty())
                .ok_or_else(|| format!("stack component {name}: HOME is not set"))?;
            PathBuf::from(home).join(location.strip_prefix("~/").unwrap_or(""))
        } else {
            base.join(location)
        };
        let location = std::path::absolute(location)
            .map_err(|err| format!("stack component {name}: {err}"))?;
        result.insert(name, Component { location, notes });
    }
    Ok(result)
}

pub(crate) fn render(components: &BTreeMap<String, Component>) -> String {
    if components.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        "# Stack\n\nRepository references for tasks involving these components. Inspect only the components relevant to the task; their contents are not included here.",
    );
    for (name, component) in components {
        let _ = write!(out, "\n- {name}: `{}`", component.location.display());
        if !component.notes.is_empty() {
            let _ = write!(out, " — {}", component.notes);
        }
    }
    out
}
