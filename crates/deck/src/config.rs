//! The Deck client's `config.toml`.

use dvc_common::config::Validate;
use dvc_common::proto;
use serde::Deserialize;

/// Where Deck input is read from.
#[derive(Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    /// The Deck hardware directly (works in Game Mode with any Steam layout).
    Hidraw,
    /// A Linux gamepad device.
    Evdev,
}

#[derive(Deserialize, Debug, PartialEq)]
#[serde(default)]
pub struct Config {
    pub source: Source,
    /// Address of the PC running deck-virtual-controller-server.
    pub server_addr: String,
    /// (evdev source) evdev path (e.g. /dev/input/event10) or device name; auto-detected if empty.
    pub device: String,
    /// Resend interval so the server knows we are alive.
    pub heartbeat_ms: u64,
    /// Stick deadzone, in raw units (axes are -32768..32767).
    pub deadzone: i32,
    /// Stick movement smaller than this, in raw units, since the last value sent
    /// is treated as jitter and not sent.
    pub stick_noise: i32,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            source: Source::Hidraw,
            server_addr: format!("127.0.0.1:{}", proto::DEFAULT_PORT),
            device: String::new(),
            heartbeat_ms: 100,
            deadzone: 0,
            stick_noise: 128,
        }
    }
}

/// The text in the shipped example that the installer replaces with the PC's address.
pub const ADDRESS_PLACEHOLDER: &str = "PC_ADDRESS";

impl Validate for Config {
    fn validate(&self) -> Result<(), String> {
        if self.server_addr.contains(ADDRESS_PLACEHOLDER) {
            return Err(format!(
                "set server_addr in your config.toml (it still holds the placeholder '{}')",
                self.server_addr
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = include_str!("../../../deploy/client/config.toml");
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

    fn example() -> Config {
        toml::from_str(EXAMPLE).expect("the shipped example config must parse")
    }

    #[test]
    fn the_example_uses_the_installers_placeholder_and_fails_validation_until_replaced() {
        let cfg = example();
        assert!(cfg.server_addr.contains(ADDRESS_PLACEHOLDER));
        assert!(cfg.validate().unwrap_err().contains(ADDRESS_PLACEHOLDER));
    }

    #[test]
    fn the_example_is_valid_once_the_installer_has_filled_in_the_address() {
        let filled = EXAMPLE.replace(ADDRESS_PLACEHOLDER, "192.168.1.20");
        let cfg: Config = toml::from_str(&filled).unwrap();
        assert_eq!(cfg.server_addr, "192.168.1.20:24800");
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn the_example_agrees_with_the_defaults_except_the_address() {
        let (example, default) = (example(), Config::default());
        assert_eq!(example.source, default.source);
        assert_eq!(example.device, default.device);
        assert_eq!(example.heartbeat_ms, default.heartbeat_ms);
        assert_eq!(example.deadzone, default.deadzone);
        assert_eq!(example.stick_noise, default.stick_noise);
    }

    #[test]
    fn the_default_address_is_valid_so_a_missing_config_file_still_starts() {
        assert!(Config::default().validate().is_ok());
    }

    #[test]
    fn an_unknown_source_is_rejected() {
        assert!(toml::from_str::<Config>("source = \"usb\"").is_err());
    }

    #[test]
    fn the_readme_documents_the_defaults() {
        let d = Config::default();
        assert_eq!(readme_default("source"), "`hidraw`");
        assert_eq!(
            readme_default("heartbeat_ms"),
            format!("`{}`", d.heartbeat_ms)
        );
        assert_eq!(readme_default("deadzone"), format!("`{}`", d.deadzone));
        assert_eq!(
            readme_default("stick_noise"),
            format!("`{}`", d.stick_noise)
        );
        // These two have no meaningful literal default.
        assert_eq!(readme_default("server_addr"), "none, required");
        assert_eq!(readme_default("device"), "auto");
        assert!(
            d.device.is_empty(),
            "'auto' in the README means an empty device"
        );
    }

    #[test]
    fn every_key_in_the_example_is_a_real_setting() {
        let keys: Vec<_> = EXAMPLE
            .lines()
            .filter(|l| !l.trim_start().starts_with('#'))
            .filter_map(|l| l.split_once('=').map(|(k, _)| k.trim().to_string()))
            .collect();
        for k in [
            "source",
            "server_addr",
            "device",
            "heartbeat_ms",
            "deadzone",
        ] {
            assert!(keys.contains(&k.to_string()), "example is missing `{k}`");
        }
        assert_eq!(
            keys.len(),
            6,
            "an example key with no matching setting: {keys:?}"
        );
    }
}
