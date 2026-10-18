//! What the emulated controller claims to be: its USB identity, the report
//! descriptor and the fixed replies, all captured from a real 2026 Steam
//! Controller.

use crate::uhid::{BUS_USB, Identity};

/// Report descriptor dumped from a real controller (hex, one line).
const REPORT_DESCRIPTOR_HEX: &str = include_str!("ibex_report_descriptor.hex");

fn report_descriptor() -> Vec<u8> {
    let hex = REPORT_DESCRIPTOR_HEX.trim();
    (0..hex.len() / 2)
        .filter_map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok())
        .collect()
}

/// Captured reply to the `0x83` (get attributes) feature command.
pub(super) const ATTRIBUTES_REPLY: [u8; 33] = [
    0x01, 0x83, 0x1e, 0x01, 0x02, 0x13, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x0a, 0x2e, 0xf9,
    0xd2, 0x68, 0x04, 0x23, 0xf3, 0xcf, 0x6a, 0x09, 0x48, 0x00, 0x00, 0x00, 0x0b, 0xa0, 0x0f, 0x00,
    0x00,
];

pub(super) const SERIAL: &[u8] = b"DECKVC0000001";

/// What the Steam Controller claims to be.
pub fn identity(name: &str) -> Identity {
    Identity {
        name: name.to_string(),
        bus: BUS_USB,
        vendor: 0x28de,
        product: 0x1302,
        version: 0x0404,
        report_descriptor: report_descriptor(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_is_the_wired_2026_controller() {
        let id = identity("x");
        assert_eq!((id.vendor, id.product, id.bus), (0x28de, 0x1302, BUS_USB));
        assert_eq!(id.report_descriptor.len(), 372);
    }
}
