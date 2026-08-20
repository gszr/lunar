use std::ffi::OsString;

pub enum Action {
    Open { continue_last: bool },
    Help,
}

pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Action, String> {
    let mut continue_last = false;

    for arg in args {
        match arg.to_str() {
            Some("-c" | "--continue") => continue_last = true,
            Some("-h" | "--help") => return Ok(Action::Help),
            Some(arg) => return Err(format!("unexpected argument '{arg}'")),
            None => return Err("argument is not valid UTF-8".into()),
        }
    }

    Ok(Action::Open { continue_last })
}

pub const HELP: &str = concat!(
    "Lunar — a terminal coding harness\n\n",
    "Usage:\n",
    "  lunar [OPTIONS]\n\n",
    "Options:\n",
    "  -c, --continue  Continue the latest mission for the current directory\n",
    "  -h, --help      Print help\n\n",
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
            Action::Open {
                continue_last: false
            }
        ));
    }

    #[test]
    fn continue_flags_resume_latest_mission() {
        for flag in ["-c", "--continue"] {
            assert!(matches!(
                parse_strs(&[flag]).unwrap(),
                Action::Open {
                    continue_last: true
                }
            ));
        }
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
        assert!(HELP.contains("-h, --help"));
    }
}
