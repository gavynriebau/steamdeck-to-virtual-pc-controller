#!/usr/bin/env python3
"""Add (or remove) the Deck Virtual Controller entry in Steam's library.

Run on the Steam Deck. Steam only reads shortcuts.vdf at startup and rewrites
it on exit, so this shuts Steam down first (Game Mode relaunches it), backs
each file up, then edits it. Does nothing if there is nothing to change.

    add_steam_shortcut.py            add the entry for every Steam user
    add_steam_shortcut.py --remove   remove it again
"""
import glob
import os
import shutil
import subprocess
import sys
import time
import zlib

def read_layout(path):
    """KEY=VALUE pairs of deck-layout.env (comments and blank lines skipped)."""
    layout = {}
    with open(path) as f:
        for line in f:
            line = line.strip()
            if line and not line.startswith("#") and "=" in line:
                key, _, value = line.partition("=")
                layout[key] = value
    return layout


# The Deck's file layout is defined once, in deck-layout.env next to this script.
LAYOUT = read_layout(os.path.join(os.path.dirname(os.path.abspath(__file__)), "deck-layout.env"))

NAME = "Deck Virtual Controller"
BIN_DIR = os.path.expanduser("~/" + LAYOUT["DECK_BIN_DIR"])
EXE = '"' + BIN_DIR + "/" + LAYOUT["DECK_APP"] + '"'
START = '"' + BIN_DIR + '/"'
STEAM_ROOTS = ["~/.local/share/Steam", "~/.steam/steam"]


# --- minimal binary VDF (keyvalues) reader/writer that round-trips byte for byte.
# A node is a list of (type, key, value); type is 0 (section), 1 (string) or 2 (int).

def parse(data, pos=0):
    items = []
    while True:
        kind = data[pos]
        pos += 1
        if kind == 8:
            return items, pos
        end = data.index(b"\x00", pos)
        key = data[pos:end].decode("utf-8")
        pos = end + 1
        if kind == 0:
            value, pos = parse(data, pos)
        elif kind == 1:
            end = data.index(b"\x00", pos)
            value = data[pos:end].decode("utf-8")
            pos = end + 1
        elif kind == 2:
            value = int.from_bytes(data[pos:pos + 4], "little")
            pos += 4
        else:
            raise ValueError(f"unsupported VDF value type {kind} at {pos}")
        items.append((kind, key, value))


def serialize(items):
    out = bytearray()
    for kind, key, value in items:
        out += bytes([kind]) + key.encode("utf-8") + b"\x00"
        if kind == 0:
            out += serialize(value)
        elif kind == 1:
            out += value.encode("utf-8") + b"\x00"
        else:
            out += value.to_bytes(4, "little")
    return bytes(out) + b"\x08"


def new_entry(index):
    appid = zlib.crc32((EXE + NAME).encode()) | 0x80000000
    return (0, str(index), [
        (2, "appid", appid),
        (1, "AppName", NAME),
        (1, "Exe", EXE),
        (1, "StartDir", START),
        (1, "icon", ""),
        (1, "ShortcutPath", ""),
        (1, "LaunchOptions", ""),
        (2, "IsHidden", 0),
        (2, "AllowDesktopConfig", 1),
        (2, "AllowOverlay", 1),
        (2, "OpenVR", 0),
        (2, "Devkit", 0),
        (1, "DevkitGameID", ""),
        (2, "DevkitOverrideAppID", 0),
        (2, "LastPlayTime", 0),
        (1, "FlatpakAppID", ""),
        (0, "tags", []),
    ])


def is_ours(entry):
    return any(k == "AppName" and v == NAME for _, k, v in entry[2])


def edit(items, remove):
    """Return the new item list, or None if nothing needs to change."""
    for i, (kind, key, section) in enumerate(items):
        if kind == 0 and key.lower() == "shortcuts":
            break
    else:
        # A fresh file has no shortcuts section yet.
        section, i = [], None
    present = any(is_ours(e) for e in section)
    if remove == (not present):
        return None
    if remove:
        kept = [e for e in section if not is_ours(e)]
        # Steam expects consecutive indices.
        section = [(0, str(n), e[2]) for n, e in enumerate(kept)]
    else:
        section = section + [new_entry(len(section))]
    updated = (0, "shortcuts", section)
    if i is None:
        return items + [updated]
    return items[:i] + [updated] + items[i + 1:]


def steam_running():
    return subprocess.run(["pgrep", "-x", "steam"], capture_output=True).returncode == 0


def shutdown_steam():
    if not steam_running():
        return
    print("shutting Steam down...")
    for root in STEAM_ROOTS:
        binary = os.path.join(os.path.expanduser(root), "ubuntu12_32", "steam")
        if os.path.exists(binary):
            subprocess.run([binary, "-shutdown"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            break
    for _ in range(60):
        if not steam_running():
            return
        time.sleep(1)
    raise SystemExit("Steam did not exit; nothing changed")


def main():
    remove = "--remove" in sys.argv[1:]
    paths = set()
    for root in STEAM_ROOTS:
        paths.update(os.path.realpath(p) for p in glob.glob(os.path.join(os.path.expanduser(root), "userdata/*/config/shortcuts.vdf")))
    if not paths:
        if remove:
            print("no shortcuts.vdf found; nothing to remove")
            return
        raise SystemExit("no Steam userdata found; start Steam once and log in, then retry")

    # Work out every change before touching Steam, so a no-op leaves it running.
    changes = {}
    for path in sorted(paths):
        with open(path, "rb") as f:
            data = f.read()
        items, end = parse(data)
        if end != len(data):
            raise SystemExit(f"{path}: unexpected trailing data; nothing changed")
        updated = edit(items, remove)
        if updated is not None:
            changes[path] = serialize(updated)
    if not changes:
        print("shortcut already removed" if remove else "shortcut already present")
        return

    shutdown_steam()
    for path, new_data in changes.items():
        backup = f"{path}.bak-{time.strftime('%Y%m%d-%H%M%S')}"
        shutil.copy2(path, backup)
        with open(path, "wb") as f:
            f.write(new_data)
        print(f"{'removed' if remove else 'added'} '{NAME}' in {path} (backup: {backup})")


if __name__ == "__main__":
    main()
