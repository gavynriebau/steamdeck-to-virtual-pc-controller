#!/usr/bin/env python3
"""Add the Deck virtual controller as a non-Steam game (run on the Deck).

Steam rewrites shortcuts.vdf on exit and only reads it at startup, so this
shuts Steam down first (Game Mode relaunches it), backs the file up, then
appends the entry. Does nothing if the entry already exists.
"""
import glob
import os
import re
import shutil
import subprocess
import time
import zlib

NAME = "Deck Virtual Controller"
EXE = '"' + os.path.expanduser("~/.local/bin/deck-controller-app") + '"'
START = '"' + os.path.expanduser("~/.local/bin/") + '"'

paths = glob.glob(os.path.expanduser("~/.local/share/Steam/userdata/*/config/shortcuts.vdf"))
if not paths:
    raise SystemExit("no shortcuts.vdf found")
path = paths[0]


def entry(idx):
    appid = (zlib.crc32((EXE + NAME).encode()) | 0x80000000).to_bytes(4, "little")
    s = lambda k, v: b"\x01" + k + b"\x00" + v.encode() + b"\x00"
    i = lambda k, v: b"\x02" + k + b"\x00" + v.to_bytes(4, "little")
    return (b"\x00" + str(idx).encode() + b"\x00"
            + b"\x02appid\x00" + appid
            + s(b"AppName", NAME) + s(b"Exe", EXE) + s(b"StartDir", START)
            + s(b"icon", "") + s(b"ShortcutPath", "") + s(b"LaunchOptions", "")
            + i(b"IsHidden", 0) + i(b"AllowDesktopConfig", 1) + i(b"AllowOverlay", 1)
            + i(b"OpenVR", 0) + i(b"Devkit", 0) + s(b"DevkitGameID", "")
            + i(b"DevkitOverrideAppID", 0) + i(b"LastPlayTime", 0) + s(b"FlatpakAppID", "")
            + b"\x00tags\x00\x08" + b"\x08")


if NAME.encode() in open(path, "rb").read():
    print("shortcut already present")
    raise SystemExit(0)

if subprocess.run(["pgrep", "-x", "steam"], capture_output=True).returncode == 0:
    print("shutting Steam down…")
    subprocess.run([os.path.expanduser("~/.local/share/Steam/ubuntu12_32/steam"), "-shutdown"],
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    for _ in range(60):
        if subprocess.run(["pgrep", "-x", "steam"], capture_output=True).returncode != 0:
            break
        time.sleep(1)
    else:
        raise SystemExit("Steam did not exit; nothing changed")

data = open(path, "rb").read()
backup = f"{path}.bak-{time.strftime('%Y%m%d-%H%M%S')}"
shutil.copy2(path, backup)
print("backup:", backup)
indices = [int(m) for m in re.findall(rb"\x00(\d+)\x00\x02appid\x00", data)]
idx = max(indices, default=-1) + 1
if not data.endswith(b"\x08\x08"):
    raise SystemExit("unexpected shortcuts.vdf layout; nothing changed")
open(path, "wb").write(data[:-2] + entry(idx) + b"\x08\x08")
print(f"added '{NAME}' as shortcut #{idx}")
