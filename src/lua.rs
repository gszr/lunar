//! User and project `init.lua`. Each returns a configuration table.

use std::path::Path;

use mlua::{Lua, Table, Value};

use crate::protocol::Config;

use self::guest::Guest;

pub(crate) struct Loaded {
    pub config: Option<Config>,
    pub models: Vec<ModelChoice>,
    pub stack: String,
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

pub fn load() -> Result<Loaded, String> {
    let user_path = crate::storage::control("init.lua");
    let project_path = std::env::current_dir()
        .unwrap_or_default()
        .join(".lunar/init.lua");
    load_paths(
        &user_path,
        &project_path,
        &crate::storage::home().join("control"),
    )
}

fn load_paths(user_path: &Path, project_path: &Path, module_root: &Path) -> Result<Loaded, String> {
    let user = parse_path(
        user_path,
        module_root,
        user_path.parent().unwrap_or(module_root),
    )?;
    let project = parse_path(
        project_path,
        module_root,
        project_path
            .parent()
            .and_then(Path::parent)
            .unwrap_or(module_root),
    )?;
    match (user, project) {
        (None, None) => Ok(empty()),
        (Some(guest), None) | (None, Some(guest)) => resolve::loaded(&guest),
        (Some(mut user), Some(project)) => {
            user.merge(project);
            resolve::loaded(&user)
        }
    }
}

#[cfg(test)]
fn load_path(path: &Path) -> Result<Loaded, String> {
    let base = path.parent().unwrap_or_else(|| Path::new("."));
    match parse_path(path, base, base)? {
        Some(guest) => resolve::loaded(&guest),
        None => Ok(empty()),
    }
}

fn parse_path(path: &Path, module_root: &Path, base: &Path) -> Result<Option<Guest>, String> {
    let src = match std::fs::read_to_string(path) {
        Ok(src) => src,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(diagnostic::config(path, None, &err.to_string())),
    };
    run(path, module_root, base, &src).map(Some)
}

fn empty() -> Loaded {
    Loaded {
        config: None,
        models: Vec::new(),
        stack: String::new(),
        notice: None,
    }
}

fn run(path: &Path, module_root: &Path, base: &Path, src: &str) -> Result<Guest, String> {
    let lua_error = |err| diagnostic::lua(path, src, err);
    let lua = Lua::new();
    let package: Table = lua.globals().get("package").map_err(lua_error)?;
    let root = module_root.to_string_lossy();
    package
        .set("path", format!("{root}/?.lua;{root}/?/init.lua"))
        .map_err(lua_error)?;
    let value = lua
        .load(src)
        .set_name(format!("@{}", path.display()))
        .eval::<Value>()
        .map_err(lua_error)?;
    let Value::Table(table) = value else {
        return Err(diagnostic::config(
            path,
            None,
            "init.lua must return a table",
        ));
    };
    guest::parse(&table, base, path).map_err(|err| {
        diagnostic::config(path, None, err.strip_prefix("init.lua ").unwrap_or(&err))
    })
}

mod diagnostic;
mod guest;
mod resolve;
#[cfg(test)]
mod tests;
