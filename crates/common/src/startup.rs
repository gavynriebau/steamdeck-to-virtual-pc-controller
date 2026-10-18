//! Process start-up shared by both binaries: load and validate the config, and
//! the one place that decides what a start-up failure looks like.

use crate::config::{self, Validate};
use serde::de::DeserializeOwned;
use std::path::PathBuf;

/// Bad arguments or configuration: the user has to fix their setup.
pub const EXIT_CONFIG: i32 = 2;
/// The environment refused something (permissions, port in use, ...).
pub const EXIT_ENV: i32 = 1;

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
/// Deck app).
pub fn try_load_config<T: DeserializeOwned + Default + Validate>(app: &str) -> Result<T, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    load_config_from(app, &args, &config::default_search(app))
}

/// Load `config.toml` for `app` from the command line and the default search
/// path, validate it, and exit with [`EXIT_CONFIG`] on any problem.
pub fn load_config<T: DeserializeOwned + Default + Validate>(app: &str) -> T {
    try_load_config(app).unwrap_or_else(|e| die(EXIT_CONFIG, &e))
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
}
