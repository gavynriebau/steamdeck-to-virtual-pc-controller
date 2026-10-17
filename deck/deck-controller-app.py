#!/usr/bin/env python3
"""Fullscreen status screen for the Deck virtual controller.

Launched from the Steam library: runs the streaming client for as long as this
window is open. Closing the game in Steam kills this process and the client
with it.
"""
import ctypes
import os
import signal
import subprocess
import threading
import tkinter as tk
import tomllib

APP = "deck-virtual-controller-client"
BIN = os.path.expanduser(f"~/.local/bin/{APP}")
CONF = os.path.expanduser(f"~/.config/{APP}/config.toml")


def server_addr():
    try:
        with open(CONF, "rb") as f:
            return tomllib.load(f).get("server_addr", "?")
    except OSError:
        return "?"


def die_with_parent():
    # PR_SET_PDEATHSIG = 1: the client gets SIGTERM if this app is killed.
    ctypes.CDLL("libc.so.6", use_errno=True).prctl(1, signal.SIGTERM)


class App:
    def __init__(self):
        self.root = tk.Tk()
        self.root.configure(bg="#0b1020", cursor="none")
        self.root.attributes("-fullscreen", True)
        self.root.bind("<Escape>", lambda e: self.quit())
        self.root.protocol("WM_DELETE_WINDOW", self.quit)
        box = tk.Frame(self.root, bg="#0b1020")
        box.place(relx=0.5, rely=0.5, anchor="center")
        tk.Label(box, text="Deck virtual controller", fg="white", bg="#0b1020",
                 font=("sans-serif", 44, "bold")).pack()
        self.status = tk.Label(box, text="Starting…", fg="#6ee7a0", bg="#0b1020",
                               font=("sans-serif", 30))
        self.status.pack(pady=24)
        self.detail = tk.Label(box, text="", fg="#9aa4bf", bg="#0b1020", font=("sans-serif", 20))
        self.detail.pack()
        tk.Label(box, text="All controls go to the PC.  Press View + Menu together to stop.",
                 fg="#5d6785", bg="#0b1020", font=("sans-serif", 16)).pack(pady=40)
        self.addr = server_addr()
        self.proc = None
        self.stopping = False
        threading.Thread(target=self.run_client, daemon=True).start()

    def set(self, status, detail=None, color="#6ee7a0"):
        def apply():
            self.status.config(text=status, fg=color)
            if detail is not None:
                self.detail.config(text=detail)
        self.root.after(0, apply)

    def run_client(self):
        while not self.stopping:
            try:
                self.proc = subprocess.Popen(
                    [BIN], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
                    preexec_fn=die_with_parent)
            except OSError as e:
                self.set("Cannot start client", str(e), "#f87171")
                return
            self.set(f"Streaming to PC", f"{self.addr}")
            for line in self.proc.stdout:
                line = line.strip()
                if line.startswith("no controller data"):
                    self.set("Streaming to PC", f"{self.addr}\n{line}", "#fbbf24")
                elif line.startswith("controller data"):
                    self.set("Streaming to PC", self.addr)
            if self.stopping:
                return
            if self.proc.wait() == 0:  # View + Menu chord pressed
                self.root.after(0, self.quit)
                return
            self.set("Client stopped, restarting…", "", "#fbbf24")
            threading.Event().wait(2)

    def quit(self):
        self.stopping = True
        if self.proc:
            self.proc.terminate()
        self.root.destroy()


if __name__ == "__main__":
    app = App()
    signal.signal(signal.SIGTERM, lambda *a: app.quit())
    app.root.mainloop()
    app.stopping = True
    if app.proc:
        app.proc.terminate()
