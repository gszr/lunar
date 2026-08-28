//! Built-in slash commands and prefix completion.

pub struct Command {
    pub name: &'static str,
    pub description: &'static str,
}

/// Shown in `/` completion and `/help`. `/q` stays a hidden alias of quit.
pub const COMMANDS: &[Command] = &[
    Command {
        name: "config",
        description: "edit and reload init.lua",
    },
    Command {
        name: "context",
        description: "summarize context; /context raw shows contents",
    },
    Command {
        name: "help",
        description: "commands",
    },
    Command {
        name: "login",
        description: "sign in to a provider",
    },
    Command {
        name: "logout",
        description: "remove a saved credential",
    },
    Command {
        name: "model",
        description: "pick a configured model",
    },
    Command {
        name: "name",
        description: "name the current mission",
    },
    Command {
        name: "new",
        description: "start a new mission",
    },
    Command {
        name: "quit",
        description: "quit",
    },
    Command {
        name: "thinking",
        description: "set the model's reasoning effort",
    },
    Command {
        name: "resume",
        description: "pick a mission in this directory",
    },
    Command {
        name: "mission",
        description: "show mission path",
    },
];

const MAX_VISIBLE: usize = 5;

/// Prefix after `/` with no whitespace. `None` if this is not a command token.
pub fn typed_prefix(input: &str) -> Option<&str> {
    let rest = input.strip_prefix('/')?;
    if rest.chars().any(char::is_whitespace) {
        None
    } else {
        Some(rest)
    }
}

pub fn matches(input: &str) -> Vec<&'static Command> {
    let Some(prefix) = typed_prefix(input) else {
        return Vec::new();
    };
    let prefix = prefix.to_ascii_lowercase();
    COMMANDS
        .iter()
        .filter(|cmd| cmd.name.starts_with(&prefix))
        .collect()
}

pub fn clamp_selected(selected: usize, n: usize) -> usize {
    if n == 0 { 0 } else { selected.min(n - 1) }
}

pub fn cycle(selected: usize, n: usize, delta: isize) -> usize {
    if n == 0 {
        return 0;
    }
    let n = n as isize;
    let next = selected as isize + delta;
    ((next % n + n) % n) as usize
}

/// Window of matches around the selection, Pi-sized.
pub fn visible<'a>(
    matches: &'a [&'static Command],
    selected: usize,
) -> (usize, &'a [&'static Command]) {
    if matches.len() <= MAX_VISIBLE {
        return (0, matches);
    }
    let start = selected
        .saturating_sub(MAX_VISIBLE / 2)
        .min(matches.len() - MAX_VISIBLE);
    (start, &matches[start..start + MAX_VISIBLE])
}

pub fn apply(name: &str) -> String {
    format!("/{name} ")
}

pub fn help() -> String {
    let commands = COMMANDS
        .iter()
        .map(|command| format!("/{}", command.name))
        .collect::<Vec<_>>()
        .join("  ");
    format!("{commands}\n\ntab cycle    shift+enter / ctrl+j newline    esc abort    ctrl+c quits")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slash_lists_all() {
        let found = matches("/");
        assert_eq!(found.len(), COMMANDS.len());
        assert_eq!(found[0].name, "config");
    }

    #[test]
    fn prefix_filters() {
        let found = matches("/re");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "resume");
        assert!(matches("/xyz").is_empty());
        assert!(matches("/resume ").is_empty());
        assert!(matches("help").is_empty());
    }

    #[test]
    fn prefix_is_case_insensitive() {
        let found = matches("/Help");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "help");
    }

    #[test]
    fn cycle_wraps() {
        assert_eq!(cycle(0, 3, -1), 2);
        assert_eq!(cycle(2, 3, 1), 0);
        assert_eq!(cycle(1, 3, 1), 2);
        assert_eq!(cycle(0, 0, 1), 0);
    }

    #[test]
    fn visible_window_follows_selection() {
        let all: Vec<_> = COMMANDS.iter().collect();
        assert!(all.len() > MAX_VISIBLE);
        let (start, view) = visible(&all, 0);
        assert_eq!(start, 0);
        assert_eq!(view.len(), MAX_VISIBLE);
        let (start, view) = visible(&all, all.len() - 1);
        assert_eq!(start, all.len() - MAX_VISIBLE);
        assert_eq!(view.last().unwrap().name, all.last().unwrap().name);
    }

    #[test]
    fn help_lists_every_command() {
        let help = help();
        for command in COMMANDS {
            assert!(
                help.split_whitespace()
                    .any(|word| word == format!("/{}", command.name))
            );
        }
    }

    #[test]
    fn apply_adds_slash_and_space() {
        assert_eq!(apply("name"), "/name ");
    }
}
