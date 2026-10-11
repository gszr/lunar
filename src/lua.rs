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

pub fn load() -> Loaded {
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

fn load_paths(user_path: &Path, project_path: &Path, module_root: &Path) -> Loaded {
    let user = match parse_path(
        user_path,
        module_root,
        user_path.parent().unwrap_or(module_root),
    ) {
        Ok(guest) => guest,
        Err(notice) => return failed(notice),
    };
    let project = match parse_path(
        project_path,
        module_root,
        project_path
            .parent()
            .and_then(Path::parent)
            .unwrap_or(module_root),
    ) {
        Ok(guest) => guest,
        Err(notice) => return failed(notice),
    };
    match (user, project) {
        (None, None) => empty(),
        (Some(guest), None) | (None, Some(guest)) => resolve::loaded(&guest),
        (Some(mut user), Some(project)) => {
            user.merge(project);
            resolve::loaded(&user)
        }
    }
}

#[cfg(test)]
fn load_path(path: &Path) -> Loaded {
    match parse_path(
        path,
        path.parent().unwrap_or_else(|| Path::new(".")),
        path.parent().unwrap_or_else(|| Path::new(".")),
    ) {
        Ok(Some(guest)) => resolve::loaded(&guest),
        Ok(None) => empty(),
        Err(notice) => failed(notice),
    }
}

fn parse_path(path: &Path, module_root: &Path, base: &Path) -> Result<Option<Guest>, String> {
    if !path.is_file() {
        return Ok(None);
    }
    let src = std::fs::read_to_string(path).map_err(|err| format!("init.lua: {err}"))?;
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

fn failed(notice: String) -> Loaded {
    Loaded {
        config: None,
        models: Vec::new(),
        stack: String::new(),
        notice: Some(notice),
    }
}

fn run(path: &Path, module_root: &Path, base: &Path, src: &str) -> Result<Guest, String> {
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
        return Err("init.lua must return a table".into());
    };
    guest::parse(&table, base)
}

fn lua_error(err: mlua::Error) -> String {
    format!("init.lua: {err}")
}

mod guest;
mod resolve;
#[cfg(test)]
mod tests;
