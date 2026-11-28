//! The Client's command line (#167): the switches that exist, and a short error for anything else (a typo of
//! `--demo` mustn't quietly start a normal Client).

/// What the command line asks for.
#[derive(Debug, PartialEq)]
pub enum Start {
    /// run, in demo mode or not (D90)
    Run { demo: bool },
    Version,
    Help,
    /// an argument that isn't a switch, or a `SINGULARITY_DEMO` that isn't a yes: what's wrong
    Wrong(String),
}

/// What `--help` prints.
pub const HELP: &str = "\
Usage: singularity [--demo]

  --demo         start in demo mode, for recording videos (or SINGULARITY_DEMO=1, true or yes)
  -V, --version  print the version
  --help         print this list
";

/// `args` without the program's name; `demo_env` is `SINGULARITY_DEMO` (empty or unset: no demo mode).
pub fn read(args: &[String], demo_env: Option<&str>) -> Start {
    let mut demo = false;
    for a in args {
        match a.as_str() {
            "--demo" => demo = true,
            "--version" | "-V" => return Start::Version,
            "--help" => return Start::Help,
            _ => return Start::Wrong(format!("unknown argument '{a}'")),
        }
    }
    match demo_env {
        None | Some("") => {}
        Some(v) if ["1", "true", "yes"].iter().any(|y| v.eq_ignore_ascii_case(y)) => demo = true,
        Some(v) => return Start::Wrong(format!("SINGULARITY_DEMO='{v}' isn't 1, true or yes")),
    }
    Start::Run { demo }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn demo_mode_is_the_switch_or_a_yes_in_the_environment() {
        assert_eq!(read(&args(&[]), None), Start::Run { demo: false });
        assert_eq!(read(&args(&["--demo"]), None), Start::Run { demo: true });
        for yes in ["1", "true", "yes", "TRUE", "Yes"] {
            assert_eq!(read(&args(&[]), Some(yes)), Start::Run { demo: true }, "{yes}");
        }
        assert_eq!(read(&args(&[]), Some("")), Start::Run { demo: false }, "empty is unset");
        for bad in ["0", "no", "false", "on", "2", " 1"] {
            assert_eq!(read(&args(&[]), Some(bad)), Start::Wrong(format!("SINGULARITY_DEMO='{bad}' isn't 1, true or yes")), "{bad}");
            assert!(matches!(read(&args(&["--demo"]), Some(bad)), Start::Wrong(_)), "{bad} with --demo");
        }
    }

    #[test]
    fn an_unknown_argument_is_an_error() {
        for typo in ["--dmeo", "-demo", "demo", "--demo=1", "-h", ""] {
            assert_eq!(read(&args(&[typo]), None), Start::Wrong(format!("unknown argument '{typo}'")), "{typo}");
            assert_eq!(read(&args(&["--demo", typo]), Some("1")), Start::Wrong(format!("unknown argument '{typo}'")), "{typo} after --demo");
        }
    }

    #[test]
    fn help_and_version_answer_before_anything_else() {
        assert_eq!(read(&args(&["--help"]), None), Start::Help);
        assert_eq!(read(&args(&["--demo", "--help"]), Some("maybe")), Start::Help, "a bad SINGULARITY_DEMO waits");
        assert_eq!(read(&args(&["--version"]), None), Start::Version);
        assert_eq!(read(&args(&["-V"]), None), Start::Version);
        // the list names exactly the switches `read` knows
        for s in ["--demo", "-V, --version", "--help", "SINGULARITY_DEMO=1, true or yes"] {
            assert!(HELP.contains(s), "{s}");
        }
        assert_eq!(HELP.matches("  -").count(), 3, "{HELP}");
    }
}
