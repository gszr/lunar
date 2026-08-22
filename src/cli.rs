use std::ffi::OsString;

#[derive(Debug, PartialEq, Eq)]
pub enum Open {
    New,
    Continue,
    Mission(Option<String>),
}

pub struct Options {
    pub open: Open,
    pub debug: bool,
}

pub enum Action {
    Open(Options),
    Help,
}

pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Action, String> {
    let mut args = args.into_iter();
    let mut open = Open::New;
    let mut debug = false;

    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("-c" | "--continue") => {
                if open != Open::New {
                    return Err("--continue and --mission are mutually exclusive".into());
                }
                open = Open::Continue;
            }
            Some("-m" | "--mission") => {
                if open != Open::New {
                    return Err("--continue and --mission are mutually exclusive".into());
                }
                let value = match args.next() {
                    Some(value) => Some(
                        value
                            .into_string()
                            .map_err(|_| "mission argument is not valid UTF-8".to_string())?,
                    ),
                    None => None,
                };
                open = Open::Mission(value);
            }
            Some("--debug") => debug = true,
            Some("-h" | "--help") => return Ok(Action::Help),
            Some(arg) => return Err(format!("unexpected argument '{arg}'")),
            None => return Err("argument is not valid UTF-8".into()),
        }
    }

    Ok(Action::Open(Options { open, debug }))
}

pub const HELP: &str = concat!(
    "Lunar — a terminal coding harness\n\n",
    "Usage:\n",
    "  lunar [OPTIONS]\n\n",
    "Options:\n",
    "  -c, --continue           Continue the latest mission for the current directory\n",
    "  -m, --mission [MISSION]  Open the mission log, or resume by filename or label\n",
    "      --debug              Log model HTTP requests and responses to $LUNAR_HOME/debug.log\n",
    "  -h, --help               Print help\n\n",
    "Running lunar without options opens the TUI.\n",
);

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_strs(args: &[&str]) -> Result<Action, String> {
        parse(args.iter().map(OsString::from))
    }

    #[test]
    fn no_arguments_opens_tui() {
        assert!(matches!(
            parse_strs(&[]).unwrap(),
            Action::Open(Options {
                open: Open::New,
                debug: false
            })
        ));
    }

    #[test]
    fn continue_flags_resume_latest_mission() {
        for flag in ["-c", "--continue"] {
            assert!(matches!(
                parse_strs(&[flag]).unwrap(),
                Action::Open(Options {
                    open: Open::Continue,
                    debug: false
                })
            ));
        }
    }

    #[test]
    fn mission_flags_optionally_take_a_selector() {
        for flag in ["-m", "--mission"] {
            assert!(matches!(
                parse_strs(&[flag, "named mission"]).unwrap(),
                Action::Open(Options { open: Open::Mission(Some(value)), debug: false }) if value == "named mission"
            ));
            assert!(matches!(
                parse_strs(&[flag]).unwrap(),
                Action::Open(Options {
                    open: Open::Mission(None),
                    debug: false
                })
            ));
        }
    }

    #[test]
    fn debug_is_orthogonal_to_open_mode() {
        assert!(matches!(
            parse_strs(&["--debug", "-c"]).unwrap(),
            Action::Open(Options {
                open: Open::Continue,
                debug: true
            })
        ));
        assert!(matches!(
            parse_strs(&["-m", "one", "--debug"]).unwrap(),
            Action::Open(Options { open: Open::Mission(Some(value)), debug: true }) if value == "one"
        ));
    }

    #[test]
    fn continue_and_mission_are_mutually_exclusive() {
        assert!(parse_strs(&["-c", "-m", "one"]).is_err());
        assert!(parse_strs(&["-m", "one", "-c"]).is_err());
        assert!(parse_strs(&["-m", "one", "-m", "two"]).is_err());
    }

    #[test]
    fn help_flags_print_help() {
        for flag in ["-h", "--help"] {
            assert!(matches!(parse_strs(&[flag]).unwrap(), Action::Help));
        }
    }

    #[test]
    fn rejects_unknown_options_and_positionals() {
        assert!(parse_strs(&["--wat"]).is_err());
        assert!(parse_strs(&["mission.jsonl"]).is_err());
        assert!(parse_strs(&["-ch"]).is_err());
    }

    #[test]
    fn help_documents_every_option() {
        assert!(HELP.contains("-c, --continue"));
        assert!(HELP.contains("-m, --mission"));
        assert!(HELP.contains("--debug"));
        assert!(HELP.contains("-h, --help"));
    }
}
