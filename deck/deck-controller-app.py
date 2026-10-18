#!/usr/bin/env python3
"""Fullscreen status screen for the Deck virtual controller.

Launched from the Steam library: runs the streaming client for as long as this
window is open. Closing the game in Steam, or pressing View + Menu, ends both.
All process handling lives in deck_supervisor; this file only draws.
"""
import os
import signal
import sys
import tkinter as tk

sys.path.insert(0, os.path.dirname(os.path.realpath(__file__)))
from deck_supervisor import Supervisor  # noqa: E402

# The client is installed next to this app.
CLIENT = os.path.join(os.path.dirname(os.path.realpath(__file__)), "deck-virtual-controller-client")
COLORS = {"ok": "#6ee7a0", "warn": "#fbbf24", "error": "#f87171"}
BG = "#0b1020"


class App:
    def __init__(self):
        self.root = tk.Tk()
        self.root.configure(bg=BG, cursor="none")
        self.root.attributes("-fullscreen", True)
        self.root.bind("<Escape>", lambda e: self.quit())
        self.root.protocol("WM_DELETE_WINDOW", self.quit)
        box = tk.Frame(self.root, bg=BG)
        box.place(relx=0.5, rely=0.5, anchor="center")
        tk.Label(box, text="Steam Deck as PC Controller", fg="white", bg=BG,
                 font=("sans-serif", 44, "bold")).pack()
        self.headline = tk.Label(box, text="Starting...", fg=COLORS["ok"], bg=BG,
                                 font=("sans-serif", 30))
        self.headline.pack(pady=24)
        self.detail = tk.Label(box, text="", fg="#9aa4bf", bg=BG, font=("sans-serif", 20))
        self.detail.pack()
        # What the screen promises while the client runs; hidden when it is not.
        self.footer = tk.Frame(box, bg=BG)
        tk.Label(self.footer, text="All controls go to the PC.", fg="#9aa4bf", bg=BG,
                 font=("sans-serif", 22)).pack(pady=(40, 8))
        tk.Label(self.footer, text="Press View + Menu together to stop", fg="#5d6785", bg=BG,
                 font=("sans-serif", 22)).pack()
        self.footer.pack()
        self.supervisor = Supervisor([CLIENT], self.show, self.close_from_thread)
        self.supervisor.start()

    def show(self, view):
        def apply():
            self.headline.config(text=view.headline, fg=COLORS[view.level])
            self.detail.config(text=view.detail)
            if view.controls_active:
                self.footer.pack()
            else:
                self.footer.pack_forget()
        self.root.after(0, apply)

    def close_from_thread(self):
        self.root.after(0, self.quit)

    def quit(self):
        self.supervisor.stop()
        self.root.destroy()


if __name__ == "__main__":
    app = App()
    signal.signal(signal.SIGTERM, lambda *a: app.quit())
    app.root.mainloop()
    app.supervisor.stop()
