//! `config.toml` loading shared by both binaries.
//!
//! Lookup order: `--config <path>`, `./config.toml`,
//! `~/.config/<app>/config.toml`. Missing keys fall back to defaults; if no
//! file is found at all, the defaults are used.

use serde::de::DeserializeOwned;
use std::path::PathBuf;

pub fn load<T: DeserializeOwned + Default>(app: &str) -> Result<T, String> {
    let mut args = std::env::args().skip(1);
    let mut explicit = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--config" => {
                explicit = Some(PathBuf::from(args.next().ok_or("--config needs a path")?))
            }
            other => {
                return Err(format!(
                    "unknown argument '{other}' (usage: {app} [--config <path>])"
                ));
            }
        }
    }

    let path = match explicit {
        Some(p) => p,
        None => {
            let mut candidates = vec![PathBuf::from("config.toml")];
            if let Some(home) = std::env::var_os("HOME") {
                candidates.push(
                    PathBuf::from(home)
                        .join(".config")
                        .join(app)
                        .join("config.toml"),
                );
            }
            match candidates.into_iter().find(|p| p.is_file()) {
                Some(p) => p,
                None => {
                    eprintln!("no config.toml found, using defaults");
                    return Ok(T::default());
                }
            }
        }
    };
    eprintln!("using config {}", path.display());
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}
