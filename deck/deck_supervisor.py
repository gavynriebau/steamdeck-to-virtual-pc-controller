"""Runs the streaming client and turns what it reports into something a screen
can show. No GUI code lives here, so it is tested with a fake client.

The contract with the client (see src/status.rs, keep both in step):
  * stdout lines `status <key>=<value>` with target=<host:port>,
    controller=ok|missing, server=reachable|unreachable|incompatible and, as the
    very last line before an error exit, fatal=<why>; other lines are ignored;
  * exit status EXIT_STOPPED_BY_USER means View + Menu was pressed and the app
    should close; EXIT_NEEDS_USER means something the user must fix, so the
    client is not restarted; any other exit status is a failure and it is retried.
"""
import ctypes
import dataclasses
import signal
import subprocess
import threading

EXIT_STOPPED_BY_USER = 0
EXIT_NEEDS_USER = 2


def parse_status(line):
    """('key', 'value') for a status line, else None."""
    line = line.strip()
    if not line.startswith("status "):
        return None
    key, sep, value = line[len("status "):].partition("=")
    if not sep or not key or not value:
        return None
    return key, value


@dataclasses.dataclass(frozen=True)
class View:
    headline: str
    detail: str
    level: str  # "ok", "warn" or "error"
    # False when the client is not running, so nothing is being sent to the PC and
    # View + Menu does nothing: the screen must not promise either.
    controls_active: bool = True


def describe(state, exit_code=None):
    """What to show for the client's reported `state` (a dict of status values).

    `exit_code` is set while waiting to restart a client that exited abnormally.
    """
    target = state.get("target", "")
    why = state.get("fatal")
    if exit_code == EXIT_NEEDS_USER:
        return View(
            "Cannot start",
            f"{why or 'a setting is wrong'}\n"
            "Close this game from the Steam menu, fix it, and start it again",
            "error",
            controls_active=False,
        )
    if exit_code is not None:
        reason = f"{why}\n" if why else ""
        return View("Client stopped", f"{reason}exit status {exit_code}, retrying...", "error")
    if "target" not in state:
        return View("Starting...", "", "ok")
    if state.get("controller") == "missing":
        return View("Controller not found", target, "warn")
    if state.get("server") == "incompatible":
        return View(
            "PC speaks another version",
            f"{target}\nUpdate the PC server to match this Deck",
            "warn",
        )
    if state.get("server") == "unreachable":
        return View("PC not responding", f"{target}\nCheck the address and the PC's firewall", "warn")
    return View("Streaming to PC", target, "ok")


def die_with_parent():
    # PR_SET_PDEATHSIG = 1: the client gets SIGTERM if this app is killed.
    ctypes.CDLL("libc.so.6", use_errno=True).prctl(1, signal.SIGTERM)


class Supervisor:
    """Keeps the client running. Calls `on_view(View)` as its state changes and
    `on_stopped()` once when the user stops it with View + Menu."""

    def __init__(self, command, on_view, on_stopped, restart_delay=2.0):
        self.command = command
        self.on_view = on_view
        self.on_stopped = on_stopped
        self.restart_delay = restart_delay
        self._stopping = threading.Event()
        self._proc = None
        self._thread = threading.Thread(target=self._run, daemon=True)

    def start(self):
        self._thread.start()

    def join(self, timeout=None):
        self._thread.join(timeout)

    def stop(self):
        """Stop supervising and terminate the client."""
        self._stopping.set()
        if self._proc is not None:
            try:
                self._proc.terminate()
            except OSError:
                pass

    def _run(self):
        while not self._stopping.is_set():
            state = {}
            try:
                self._proc = subprocess.Popen(
                    self.command, stdout=subprocess.PIPE, text=True, preexec_fn=die_with_parent
                )
            except OSError as e:
                self.on_view(View("Cannot start client", str(e), "error"))
                return
            self.on_view(describe(state))
            with self._proc.stdout:
                for line in self._proc.stdout:
                    parsed = parse_status(line)
                    if parsed:
                        state[parsed[0]] = parsed[1]
                        self.on_view(describe(state))
            code = self._proc.wait()
            if self._stopping.is_set():
                return
            if code == EXIT_STOPPED_BY_USER:
                self.on_stopped()
                return
            self.on_view(describe(state, code))
            if code == EXIT_NEEDS_USER:
                return  # restarting cannot fix a setting; the screen says what to do
            self._stopping.wait(self.restart_delay)
