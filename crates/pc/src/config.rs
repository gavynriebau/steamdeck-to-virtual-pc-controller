//! The PC server's `config.toml`.

use dvc_common::config::Validate;
use dvc_common::proto;
use serde::Deserialize;

#[derive(Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum ControllerType {
    /// uinput Xbox 360 gamepad.
    Xbox,
    /// Emulated 2026 Steam Controller (28de:1302) over /dev/uhid: buttons, sticks, trackpads, gyro.
    Steam,
}

#[derive(Deserialize, Debug, PartialEq)]
#[serde(default)]
pub struct Config {
    pub bind_addr: String,
    /// Release all inputs if nothing is heard from the client for this long.
    pub timeout_ms: u64,
    /// "xbox" (Xbox 360 pad) or "steam" (Valve Steam Controller).
    pub controller_type: ControllerType,
    /// Name of the virtual gamepad.
    pub name: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            bind_addr: format!("0.0.0.0:{}", proto::DEFAULT_PORT),
            timeout_ms: 1000,
            controller_type: ControllerType::Steam,
            name: "Steam Deck Virtual Controller".into(),
        }
    }
}

impl Validate for Config {}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = include_str!("../../../deploy/server/config.toml");
    const README: &str = include_str!("../../../README.md");

    /// The default cell of the README's row for `key` (the table's second column).
    fn readme_default(key: &str) -> String {
        let prefix = format!("| `{key}` |");
        let row = README
            .lines()
            .find(|l| l.starts_with(&prefix))
            .unwrap_or_else(|| panic!("README has no row for `{key}`"));
        row.split('|').nth(2).unwrap().trim().to_string()
    }

    #[test]
    fn the_example_parses_and_is_exactly_the_defaults() {
        let example: Config = toml::from_str(EXAMPLE).expect("the shipped example must parse");
        assert_eq!(example, Config::default());
    }

    #[test]
    fn the_readme_documents_the_defaults() {
        let d = Config::default();
        assert_eq!(readme_default("bind_addr"), format!("`{}`", d.bind_addr));
        assert_eq!(readme_default("timeout_ms"), format!("`{}`", d.timeout_ms));
        assert_eq!(readme_default("controller_type"), "`steam`");
        assert_eq!(d.controller_type, ControllerType::Steam);
        assert_eq!(readme_default("name"), format!("`{}`", d.name));
    }

    #[test]
    fn an_unknown_controller_type_is_rejected() {
        assert!(toml::from_str::<Config>("controller_type = \"ps5\"").is_err());
    }

    #[test]
    fn the_default_bind_address_uses_the_protocol_port() {
        assert!(
            Config::default()
                .bind_addr
                .ends_with(&proto::DEFAULT_PORT.to_string())
        );
    }

    #[test]
    fn every_key_in_the_example_is_a_real_setting() {
        let keys: Vec<_> = EXAMPLE
            .lines()
            .filter(|l| !l.trim_start().starts_with('#'))
            .filter_map(|l| l.split_once('=').map(|(k, _)| k.trim().to_string()))
            .collect();
        assert_eq!(
            keys.len(),
            4,
            "an example key with no matching setting: {keys:?}"
        );
    }
}
