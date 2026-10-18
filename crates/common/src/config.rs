//! `config.toml` loading shared by both binaries.
//!
//! Lookup order: `--config <path>`, `./config.toml`,
//! `~/.config/<app>/config.toml`. Missing keys fall back to defaults; if no
//! file is found at all, the defaults are used. Nothing here prints or exits:
//! see [`crate::startup`] for the process-level wrapper.

use serde::de::DeserializeOwned;
use std::path::{Path, PathBuf};

/// Checks a loaded config makes sense before the program starts using it.
pub trait Validate {
    fn validate(&self) -> Result<(), String> {
        Ok(())
    }
}

/// A loaded config and the file it came from (`None` = built-in defaults).
#[derive(Debug)]
pub struct Loaded<T> {
    pub value: T,
    pub path: Option<PathBuf>,
}

/// The default places to look for `config.toml`, most specific first.
pub fn default_search(app: &str) -> Vec<PathBuf> {
    let mut paths = vec![PathBuf::from("config.toml")];
    if let Some(home) = std::env::var_os("HOME") {
        paths.push(
            Path::new(&home)
                .join(".config")
                .join(app)
                .join("config.toml"),
        );
    }
    paths
}

/// Load a config from `args` (the command line without the program name),
/// looking in `search` unless `--config <path>` names a file.
pub fn load_from<T: DeserializeOwned + Default>(
    app: &str,
    args: &[String],
    search: &[PathBuf],
) -> Result<Loaded<T>, String> {
    let mut explicit = None;
    let mut args = args.iter();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--config" => {
                explicit = Some(PathBuf::from(args.next().ok_or("--config needs a path")?));
            }
            other => {
                return Err(format!(
                    "unknown argument '{other}' (usage: {app} [--config <path>]; try --help)"
                ));
            }
        }
    }

    let path = match explicit.or_else(|| search.iter().find(|p| p.is_file()).cloned()) {
        Some(p) => p,
        None => {
            return Ok(Loaded {
                value: T::default(),
                path: None,
            });
        }
    };
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let value = toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(Loaded {
        value,
        path: Some(path),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize, Debug, PartialEq)]
    #[serde(default)]
    struct Cfg {
        port: u16,
        name: String,
    }

    impl Default for Cfg {
        fn default() -> Self {
            Cfg {
                port: 1,
                name: "default".into(),
            }
        }
    }

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    fn temp_file(name: &str, contents: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dvc-config-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn no_file_means_defaults() {
        let l: Loaded<Cfg> =
            load_from("app", &[], &[PathBuf::from("/nonexistent/config.toml")]).unwrap();
        assert_eq!(l.value, Cfg::default());
        assert!(l.path.is_none());
    }

    #[test]
    fn explicit_path_wins_over_the_search_list() {
        let explicit = temp_file("explicit.toml", "port = 7\n");
        let searched = temp_file("searched.toml", "port = 9\n");
        let l: Loaded<Cfg> = load_from(
            "app",
            &args(&["--config", explicit.to_str().unwrap()]),
            &[searched],
        )
        .unwrap();
        assert_eq!(l.value.port, 7);
        assert_eq!(l.path, Some(explicit));
    }

    #[test]
    fn first_existing_search_path_is_used_and_missing_keys_default() {
        let found = temp_file("found.toml", "name = \"x\"\n");
        let l: Loaded<Cfg> =
            load_from("app", &[], &[PathBuf::from("/nonexistent/a.toml"), found]).unwrap();
        assert_eq!(
            l.value,
            Cfg {
                port: 1,
                name: "x".into()
            }
        );
    }

    #[test]
    fn unknown_argument_is_an_error_naming_the_usage() {
        let e = load_from::<Cfg>("app", &args(&["--bogus"]), &[]).unwrap_err();
        assert!(e.contains("--bogus") && e.contains("usage: app"), "{e}");
    }

    #[test]
    fn config_flag_needs_a_value() {
        assert!(load_from::<Cfg>("app", &args(&["--config"]), &[]).is_err());
    }

    #[test]
    fn bad_toml_names_the_file() {
        let bad = temp_file("bad.toml", "port = \"not a number\"\n");
        let e =
            load_from::<Cfg>("app", &args(&["--config", bad.to_str().unwrap()]), &[]).unwrap_err();
        assert!(e.contains("bad.toml"), "{e}");
    }
}
