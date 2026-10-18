# Steam Deck as PC controller

Streams the Deck's controls over UDP (LAN) to a virtual controller on the PC: an emulated Steam Controller
(buttons, sticks, trackpads, gyro — Steam adopts it as a real one) or an Xbox 360 pad.
(The Deck's USB-C port can't act as a USB gadget, so the USB cable only powers it.)

- **Server** (PC): `cargo run --release --bin deck-virtual-controller-server` — needs write access to `/dev/uhid` (`steam` type) or `/dev/uinput` (`xbox` type).
  Config: `./config.toml` or `~/.config/deck-virtual-controller-server/config.toml` (see `deploy/server/config.toml`), or `--config <path>`.
  `controller_type = "steam"` (default) or `"xbox"` picks the type of virtual pad.
- **Deck**: `./install.sh [deck@deck] [pc-ip]` builds a static client, installs it plus a fullscreen status app,
  and adds a **Deck Virtual Controller** entry to the Steam library (this restarts Steam on the Deck once).
  Open it from the library to start streaming; press View + Menu together on the Deck (or close the game from the Steam menu) to stop.
  Client config: `~/.config/deck-virtual-controller-client/config.toml` (see `deploy/client/config.toml`).

Port 24800/udp must be reachable on the PC.
