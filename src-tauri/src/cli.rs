//! Command-line parsing for emede.
//!
//! emede is primarily a windowed markdown reader, but it also exposes a handful of
//! batch/headless modes. Mode flags are parsed here, up front, so headless modes
//! (`--share`, `--export`, `--list`) can do their work and exit **before** the
//! Tauri event loop is built — mirroring the pre-existing `--help`/`--version`
//! handling and guaranteeing no window ever flashes open. `--print` is the one
//! mode that still needs the WebView, so it enters a hidden-window Tauri app.

/// The single mode a given invocation resolves to.
pub enum Mode {
    /// Default: open each file in its own window (empty = home screen).
    Open(Vec<String>),
    /// Serve the given notes over the LAN, headless, until Ctrl+C.
    Share(Vec<String>),
    /// Write a self-contained HTML file (`out` = None → default path, "-" = stdout).
    Export { file: String, out: Option<String> },
    /// Render a PDF via the bundled WebView (`out` = None → default path).
    Print { file: String, out: Option<String> },
    /// List notes shared by any running emede instance.
    List { json: bool },
    /// Print help and exit.
    Help,
    /// Print version and exit.
    Version,
    /// A usage error: the message is already formatted for stderr.
    Error(String),
}

/// Parse `args` (the process arguments, *excluding* argv[0]) into a [`Mode`].
pub fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Mode {
    let name = env!("CARGO_PKG_NAME");

    let mut mode: Option<&'static str> = None;
    let mut files: Vec<String> = Vec::new();
    let mut out: Option<String> = None;
    let mut json = false;
    let mut positional_only = false;

    let mut it = args.into_iter();
    while let Some(arg) = it.next() {
        if positional_only {
            files.push(arg);
            continue;
        }
        match arg.as_str() {
            // `--` ends option parsing; everything after is a path (allows paths
            // that begin with '-').
            "--" => positional_only = true,
            "-h" | "--help" => return Mode::Help,
            "-v" | "-V" | "--version" => return Mode::Version,
            "--share" | "--export" | "--print" | "--list" => {
                let kw: &'static str = match arg.as_str() {
                    "--share" => "share",
                    "--export" => "export",
                    "--print" => "print",
                    _ => "list",
                };
                match mode {
                    Some(existing) if existing != kw => {
                        return Mode::Error(format!(
                            "{name}: --{existing} and --{kw} cannot be combined\n\nTry '{name} --help' for more information."
                        ));
                    }
                    _ => mode = Some(kw),
                }
            }
            "-o" | "--output" => match it.next() {
                Some(v) => out = Some(v),
                None => {
                    return Mode::Error(format!("{name}: option '{arg}' requires a value"))
                }
            },
            "--json" => json = true,
            // Any other dash-led token (that isn't a bare '-') is an unknown flag.
            other if other.starts_with('-') && other != "-" => {
                return Mode::Error(format!(
                    "{name}: unknown option '{other}'\n\nUSAGE:\n    {name} [OPTIONS] [FILE]...\n\nTry '{name} --help' for more information."
                ));
            }
            // Positional argument (a file path or URL).
            _ => files.push(arg),
        }
    }

    // Reject flags that don't apply to the resolved mode.
    if out.is_some() && !matches!(mode, Some("export") | Some("print")) {
        return Mode::Error(format!(
            "{name}: -o/--output is only valid with --export or --print"
        ));
    }
    if json && mode != Some("list") {
        return Mode::Error(format!("{name}: --json is only valid with --list"));
    }

    match mode {
        Some("share") => {
            if files.is_empty() {
                return Mode::Error(format!("{name}: --share requires at least one file"));
            }
            Mode::Share(files)
        }
        Some("export") | Some("print") => {
            let is_print = mode == Some("print");
            let verb = if is_print { "--print" } else { "--export" };
            if files.len() != 1 {
                return Mode::Error(format!("{name}: {verb} requires exactly one file"));
            }
            let file = files.into_iter().next().expect("one file");
            if is_print {
                Mode::Print { file, out }
            } else {
                Mode::Export { file, out }
            }
        }
        Some("list") => Mode::List { json },
        _ => Mode::Open(files),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Mode {
        parse_args(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn no_args_is_empty_open() {
        assert!(matches!(parse(&[]), Mode::Open(f) if f.is_empty()));
    }

    #[test]
    fn single_file_opens() {
        assert!(matches!(parse(&["a.md"]), Mode::Open(f) if f == vec!["a.md"]));
    }

    #[test]
    fn multiple_files_open() {
        assert!(matches!(parse(&["a.md", "b.md"]), Mode::Open(f) if f.len() == 2));
    }

    #[test]
    fn share_collects_files() {
        assert!(matches!(parse(&["--share", "a.md", "b.md"]), Mode::Share(f) if f.len() == 2));
    }

    #[test]
    fn share_requires_a_file() {
        assert!(matches!(parse(&["--share"]), Mode::Error(_)));
    }

    #[test]
    fn export_with_output() {
        match parse(&["--export", "a.md", "-o", "out.html"]) {
            Mode::Export { file, out } => {
                assert_eq!(file, "a.md");
                assert_eq!(out.as_deref(), Some("out.html"));
            }
            _ => panic!("expected export"),
        }
    }

    #[test]
    fn export_output_before_file() {
        match parse(&["--export", "-o", "out.html", "a.md"]) {
            Mode::Export { file, out } => {
                assert_eq!(file, "a.md");
                assert_eq!(out.as_deref(), Some("out.html"));
            }
            _ => panic!("expected export"),
        }
    }

    #[test]
    fn export_needs_exactly_one_file() {
        assert!(matches!(parse(&["--export", "a.md", "b.md"]), Mode::Error(_)));
        assert!(matches!(parse(&["--export"]), Mode::Error(_)));
    }

    #[test]
    fn print_mode() {
        assert!(matches!(parse(&["--print", "a.md"]), Mode::Print { .. }));
    }

    #[test]
    fn list_plain_and_json() {
        assert!(matches!(parse(&["--list"]), Mode::List { json: false }));
        assert!(matches!(parse(&["--list", "--json"]), Mode::List { json: true }));
    }

    #[test]
    fn json_only_with_list() {
        assert!(matches!(parse(&["--export", "a.md", "--json"]), Mode::Error(_)));
    }

    #[test]
    fn output_only_with_export_or_print() {
        assert!(matches!(parse(&["a.md", "-o", "out.html"]), Mode::Error(_)));
    }

    #[test]
    fn help_and_version() {
        assert!(matches!(parse(&["--help"]), Mode::Help));
        assert!(matches!(parse(&["-h"]), Mode::Help));
        assert!(matches!(parse(&["--version"]), Mode::Version));
        assert!(matches!(parse(&["-v"]), Mode::Version));
    }

    #[test]
    fn double_dash_allows_dash_paths() {
        assert!(matches!(parse(&["--", "-weird.md"]), Mode::Open(f) if f == vec!["-weird.md"]));
    }

    #[test]
    fn unknown_flag_errors() {
        assert!(matches!(parse(&["--bogus"]), Mode::Error(_)));
    }

    #[test]
    fn conflicting_modes_error() {
        assert!(matches!(parse(&["--share", "--list", "a.md"]), Mode::Error(_)));
    }
}
