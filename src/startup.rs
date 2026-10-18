//! Process start-up shared by both binaries: load and validate the config, and
//! the one place that decides what a start-up failure looks like.

use crate::config::{self, Validate};
use serde::de::DeserializeOwned;

/// Bad arguments or configuration: the user has to fix their setup.
pub const EXIT_CONFIG: i32 = 2;
/// The environment refused something (permissions, port in use, ...).
pub const EXIT_ENV: i32 = 1;

/// Print `msg` and exit with `code`.
pub fn die(code: i32, msg: &str) -> ! {
    eprintln!("{msg}");
    std::process::exit(code);
}

/// Load `config.toml` for `app` from the command line and the default search
/// path, validate it, and exit with [`EXIT_CONFIG`] on any problem.
pub fn load_config<T: DeserializeOwned + Default + Validate>(app: &str) -> T {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let loaded = config::load_from::<T>(app, &args, &config::default_search(app))
        .and_then(|l| l.value.validate().map(|()| l))
        .unwrap_or_else(|e| die(EXIT_CONFIG, &e));
    match &loaded.path {
        Some(p) => eprintln!("using config {}", p.display()),
        None => eprintln!("no config.toml found, using defaults"),
    }
    loaded.value
}
