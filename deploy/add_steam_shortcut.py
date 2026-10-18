#!/usr/bin/env python3
"""Add (or remove) the Deck Virtual Controller entry in Steam's library.

Run on the Steam Deck. The file format lives in steam_vdf.py; this holds what a
shortcut entry is, how Steam is stopped and the command line. Steam only reads
shortcuts.vdf at startup and rewrites it on exit, so this shuts Steam down first
(Game Mode relaunches it), backs each file up, then edits it. Does nothing if
there is nothing to change.

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

# The script's own directory holds steam_vdf.py, whichever way it was loaded.
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import steam_vdf  # noqa: E402

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


class SteamProcess:
    """The running Steam client, as far as this tool needs it. `run` and `sleep`
    are injectable so the shutdown logic can be tested without a Steam."""

    def __init__(self, roots=STEAM_ROOTS, run=subprocess.run, sleep=time.sleep, wait_seconds=60):
        self.roots = roots
        self.run = run
        self.sleep = sleep
        self.wait_seconds = wait_seconds

    def is_running(self):
        return self.run(["pgrep", "-x", "steam"], capture_output=True).returncode == 0

    def shutdown(self):
        """Ask Steam to quit and wait for it to; exit if it will not."""
        if not self.is_running():
            return
        print("shutting Steam down...")
        for root in self.roots:
            binary = os.path.join(os.path.expanduser(root), "ubuntu12_32", "steam")
            if os.path.exists(binary):
                self.run([binary, "-shutdown"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                break
        for _ in range(self.wait_seconds):
            if not self.is_running():
                return
            self.sleep(1)
        raise SystemExit("Steam did not exit; nothing changed")


def find_shortcut_files(roots):
    """Every user's shortcuts.vdf under the Steam roots (real paths, deduplicated)."""
    paths = set()
    for root in roots:
        pattern = os.path.join(os.path.expanduser(root), "userdata/*/config/shortcuts.vdf")
        paths.update(os.path.realpath(p) for p in glob.glob(pattern))
    return sorted(paths)


def main(argv=None, steam=None, roots=None, stamp=None):
    """Add (or with --remove, remove) the entry in every user's shortcuts.vdf.

    `steam` (a SteamProcess), `roots` and `stamp` (the backup suffix) can be
    replaced by tests."""
    remove = "--remove" in (sys.argv[1:] if argv is None else argv)
    steam = steam or SteamProcess()
    roots = STEAM_ROOTS if roots is None else roots
    paths = find_shortcut_files(roots)
    if not paths:
        if remove:
            print("no shortcuts.vdf found; nothing to remove")
            return
        raise SystemExit("no Steam userdata found; start Steam once and log in, then retry")

    # Work out every change before touching Steam, so a no-op leaves it running.
    changes = {}
    for path in paths:
        with open(path, "rb") as f:
            data = f.read()
        items, end = steam_vdf.parse(data)
        if end != len(data):
            raise SystemExit(f"{path}: unexpected trailing data; nothing changed")
        updated = edit(items, remove)
        if updated is not None:
            changes[path] = steam_vdf.serialize(updated)
    if not changes:
        print("shortcut already removed" if remove else "shortcut already present")
        return

    steam.shutdown()
    stamp = stamp or time.strftime("%Y%m%d-%H%M%S")
    for path, new_data in changes.items():
        backup = f"{path}.bak-{stamp}"
        shutil.copy2(path, backup)
        with open(path, "wb") as f:
            f.write(new_data)
        print(f"{'removed' if remove else 'added'} '{NAME}' in {path} (backup: {backup})")


if __name__ == "__main__":
    main()
