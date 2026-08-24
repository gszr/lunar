//! User `~/.lunar/init.lua`. The returned table is the configuration.

use std::path::Path;

use mlua::{Lua, Value};

use crate::protocol::Config;

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
            config: None,
            models: Vec::new(),
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
    let lua = Lua::new();
    let value = match lua
        .load(src)
        .set_name(format!("@{}", path.display()))
        .eval::<Value>()
    {
        Ok(value) => value,
        Err(err) => {
            return Loaded {
                config: None,
                models: Vec::new(),
                notice: Some(format!("init.lua: {err}")),
            };
        }
    };
    let Value::Table(table) = value else {
        return Loaded {
            config: None,
            models: Vec::new(),
            notice: Some("init.lua must return a table".into()),
        };
    };
    match guest::parse(&table) {
        Ok(guest) => resolve::loaded(&guest),
        Err(notice) => Loaded {
            config: None,
            models: Vec::new(),
            notice: Some(notice),
        },
    }
}

mod guest;
mod resolve;
#[cfg(test)]
mod tests;
