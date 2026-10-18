//! Process start-up shared by both binaries: load and validate the config, and
//! the one place that decides what a start-up failure looks like.

use crate::config::{self, Validate};
use serde::de::DeserializeOwned;
use std::path::PathBuf;

/// Bad arguments or configuration: the user has to fix their setup.
pub const EXIT_CONFIG: i32 = 2;
/// The environment refused something (permissions, port in use, ...).
pub const EXIT_ENV: i32 = 1;

/// What a program says about itself for `--help`, `--version` and its first log line.
pub struct AppInfo {
    pub name: &'static str,
    /// The program's crate version (pass `env!("CARGO_PKG_VERSION")`).
    pub version: &'static str,
    /// One line on what it does.
    pub about: &'static str,
}

impl AppInfo {
    /// `deck-virtual-controller-client 0.1.0 (protocol 3)`: what a bug report should quote.
    pub fn version_line(&self) -> String {
        format!(
            "{} {} (protocol {})",
            self.name,
            self.version,
            char::from(crate::proto::VERSION)
        )
    }

    pub fn help_text(&self) -> String {
        let name = self.name;
        format!(
            "{}\n{}\n\n\
             usage: {name} [--config <path>]\n       {name} --help | --version\n\n\
             options:\n  \
             --config <path>   read this config.toml instead of searching for one\n  \
             -h, --help        show this help\n  \
             -V, --version     show the version\n\n\
             Without --config, the first of ./config.toml and ~/.config/{name}/config.toml is\n\
             used; with neither, the built-in defaults apply.",
            self.version_line(),
            self.about
        )
    }

    /// The text to print and exit 0 for, if `args` ask for `--help` or `--version`.
    pub fn standard_flag_output(&self, args: &[String]) -> Option<String> {
        for arg in args {
            match arg.as_str() {
                "-h" | "--help" => return Some(self.help_text()),
                "-V" | "--version" => return Some(self.version_line()),
                _ => {}
            }
        }
        None
    }
}

/// Print `msg` and exit with `code`.
pub fn die(code: i32, msg: &str) -> ! {
    crate::error!("{msg}");
    std::process::exit(code);
}

/// Load and validate the config for `app` from `args` (the command line without
/// the program name), looking in `search`, and log which file was used.
pub fn load_config_from<T: DeserializeOwned + Default + Validate>(
    app: &str,
    args: &[String],
    search: &[PathBuf],
) -> Result<T, String> {
    let loaded = config::load_from::<T>(app, args, search)?;
    loaded.value.validate()?;
    match &loaded.path {
        Some(p) => crate::info!("using config {}", p.display()),
        None => crate::info!("no config.toml found, using defaults"),
    }
    Ok(loaded.value)
}

/// Like [`load_config`], but hands the problem back instead of exiting, for a
/// program that wants to report it before it goes (the Deck client tells the
/// Deck app). `--help` and `--version` still print and exit 0.
pub fn try_load_config<T: DeserializeOwned + Default + Validate>(
    info: &AppInfo,
) -> Result<T, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(text) = info.standard_flag_output(&args) {
        println!("{text}");
        std::process::exit(0);
    }
    crate::info!("{}", info.version_line());
    load_config_from(info.name, &args, &config::default_search(info.name))
}

/// Load `config.toml` for the program from the command line and the default
/// search path, validate it, and exit with [`EXIT_CONFIG`] on any problem.
pub fn load_config<T: DeserializeOwned + Default + Validate>(info: &AppInfo) -> T {
    try_load_config(info).unwrap_or_else(|e| die(EXIT_CONFIG, &e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::{Level, capture};
    use serde::Deserialize;

    #[derive(Deserialize, Default, Debug, PartialEq)]
    #[serde(default)]
    struct Cfg {
        port: u16,
    }

    impl Validate for Cfg {
        fn validate(&self) -> Result<(), String> {
            if self.port == 13 {
                return Err("port 13 is unlucky".into());
            }
            Ok(())
        }
    }

    fn file(name: &str, contents: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dvc-startup-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn a_valid_config_loads_and_the_file_is_logged() {
        let path = file("ok.toml", "port = 7\n");
        let (cfg, lines) =
            capture(|| load_config_from::<Cfg>("app", &[], std::slice::from_ref(&path)));
        assert_eq!(cfg, Ok(Cfg { port: 7 }));
        assert_eq!(
            lines,
            [(Level::Info, format!("using config {}", path.display()))]
        );
    }

    #[test]
    fn no_config_file_means_defaults_and_says_so() {
        let (cfg, lines) = capture(|| load_config_from::<Cfg>("app", &[], &[]));
        assert_eq!(cfg, Ok(Cfg { port: 0 }));
        assert_eq!(
            lines,
            [(
                Level::Info,
                "no config.toml found, using defaults".to_string()
            )]
        );
    }

    #[test]
    fn a_config_that_fails_validation_is_an_error_not_a_default() {
        let path = file("unlucky.toml", "port = 13\n");
        let (cfg, lines) = capture(|| load_config_from::<Cfg>("app", &[], &[path]));
        assert_eq!(cfg, Err("port 13 is unlucky".to_string()));
        assert!(
            lines.is_empty(),
            "nothing is logged as used for a rejected config"
        );
    }

    #[test]
    fn a_bad_argument_or_unreadable_file_is_an_error() {
        assert!(load_config_from::<Cfg>("app", &["--bogus".into()], &[]).is_err());
        let e = load_config_from::<Cfg>(
            "app",
            &["--config".into(), "/nonexistent/c.toml".into()],
            &[],
        )
        .unwrap_err();
        assert!(e.contains("/nonexistent/c.toml"), "{e}");
    }

    const INFO: AppInfo = AppInfo {
        name: "demo-app",
        version: "1.2.3",
        about: "Does demonstrations.",
    };

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_version_line_has_the_name_the_version_and_the_protocol() {
        assert_eq!(
            INFO.version_line(),
            format!(
                "demo-app 1.2.3 (protocol {})",
                char::from(crate::proto::VERSION)
            )
        );
    }

    #[test]
    fn the_standard_flags_produce_text_to_print() {
        for flag in ["--version", "-V"] {
            assert_eq!(
                INFO.standard_flag_output(&args(&[flag])),
                Some(INFO.version_line())
            );
        }
        for flag in ["--help", "-h"] {
            let help = INFO.standard_flag_output(&args(&[flag])).unwrap();
            assert!(help.contains("usage: demo-app [--config <path>]"), "{help}");
            assert!(
                help.contains("--version") && help.contains("Does demonstrations."),
                "{help}"
            );
            assert!(help.contains("~/.config/demo-app/config.toml"), "{help}");
            assert!(
                help.starts_with("demo-app 1.2.3"),
                "the help leads with the version"
            );
        }
    }

    #[test]
    fn other_arguments_are_not_standard_flags() {
        assert_eq!(INFO.standard_flag_output(&args(&[])), None);
        assert_eq!(
            INFO.standard_flag_output(&args(&["--config", "x.toml"])),
            None
        );
        assert_eq!(INFO.standard_flag_output(&args(&["--bogus"])), None);
    }

    #[test]
    fn a_standard_flag_wins_even_after_other_arguments() {
        let out = INFO.standard_flag_output(&args(&["--config", "x.toml", "--version"]));
        assert_eq!(out, Some(INFO.version_line()));
    }
}
