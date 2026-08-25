//! User and project `init.lua`. Each returns a configuration table.

use std::path::Path;

use mlua::{Lua, Value};

use crate::protocol::Config;

use self::guest::Guest;

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
    load_paths(
        &crate::storage::control("init.lua"),
        &std::env::current_dir()
            .unwrap_or_default()
            .join(".lunar/init.lua"),
    )
}

fn load_paths(user_path: &Path, project_path: &Path) -> Loaded {
    let user = match parse_path(user_path) {
        Ok(guest) => guest,
        Err(notice) => return failed(notice),
    };
    let project = match parse_path(project_path) {
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
    match parse_path(path) {
        Ok(Some(guest)) => resolve::loaded(&guest),
        Ok(None) => empty(),
        Err(notice) => failed(notice),
    }
}

fn parse_path(path: &Path) -> Result<Option<Guest>, String> {
    if !path.is_file() {
        return Ok(None);
    }
    let src = std::fs::read_to_string(path).map_err(|err| format!("init.lua: {err}"))?;
    run(path, &src).map(Some)
}

fn empty() -> Loaded {
    Loaded {
        config: None,
        models: Vec::new(),
        notice: None,
    }
}

fn failed(notice: String) -> Loaded {
    Loaded {
        config: None,
        models: Vec::new(),
        notice: Some(notice),
    }
}

fn run(path: &Path, src: &str) -> Result<Guest, String> {
    let lua = Lua::new();
    let value = lua
        .load(src)
        .set_name(format!("@{}", path.display()))
        .eval::<Value>()
        .map_err(|err| format!("init.lua: {err}"))?;
    let Value::Table(table) = value else {
        return Err("init.lua must return a table".into());
    };
    guest::parse(&table)
}

mod guest;
mod resolve;
#[cfg(test)]
mod tests;
