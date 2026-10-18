"""Tests for the client supervisor. Run: python3 -m unittest discover -s deck/tests"""
import os
import sys
import threading
import time
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
import deck_supervisor as ds  # noqa: E402


def contract_rows(kind):
    """Rows of contract/status_contract.txt (shared with the Rust client's tests)."""
    path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "contract", "status_contract.txt")
    with open(path) as f:
        lines = f.read().split("\n")
    return [l.split("|")[1:] for l in lines if not l.startswith("#") and l.startswith(kind + "|")]


class Contract(unittest.TestCase):
    def test_valid_lines_parse_to_the_documented_key_and_value(self):
        rows = contract_rows("valid")
        self.assertGreaterEqual(len(rows), 5)
        for line, key, value in rows:
            self.assertEqual(ds.parse_status(line), (key, value), line)

    def test_ignored_lines_are_not_statuses(self):
        rows = contract_rows("ignored")
        self.assertTrue(rows)
        for (line,) in rows:
            self.assertIsNone(ds.parse_status(line), repr(line))

    def test_exit_status_matches(self):
        stopped = [r for r in contract_rows("exit") if r[0] == "stopped-by-user"][0]
        self.assertEqual(int(stopped[1]), ds.EXIT_STOPPED_BY_USER)

    def test_every_documented_value_has_a_screen(self):
        # The app must show something sensible for each status the contract lists.
        state = {}
        for line, key, value in contract_rows("valid"):
            state[key] = value
            self.assertTrue(ds.describe(state).headline)


class Parse(unittest.TestCase):
    def test_status_lines(self):
        self.assertEqual(ds.parse_status("status server=reachable"), ("server", "reachable"))
        self.assertEqual(ds.parse_status("  status target=1.2.3.4:5\n"), ("target", "1.2.3.4:5"))

    def test_other_lines_are_ignored(self):
        for line in ["", "sending to 1.2.3.4:5", "status", "status server", "status server=", "status =x"]:
            self.assertIsNone(ds.parse_status(line), line)


class Describe(unittest.TestCase):
    def test_before_the_client_reports_anything(self):
        self.assertEqual(ds.describe({}).headline, "Starting...")

    def test_streaming(self):
        v = ds.describe({"target": "pc:24800", "controller": "ok", "server": "reachable"})
        self.assertEqual((v.headline, v.detail, v.level), ("Streaming to PC", "pc:24800", "ok"))

    def test_streaming_while_the_server_has_not_answered_yet(self):
        self.assertEqual(ds.describe({"target": "pc:24800"}).headline, "Streaming to PC")

    def test_unreachable_server_is_a_warning(self):
        v = ds.describe({"target": "pc:24800", "server": "unreachable"})
        self.assertEqual((v.headline, v.level), ("PC not responding", "warn"))
        self.assertIn("pc:24800", v.detail)

    def test_missing_controller_is_a_warning_and_wins_over_the_server(self):
        v = ds.describe({"target": "pc:1", "controller": "missing", "server": "unreachable"})
        self.assertEqual((v.headline, v.level), ("Controller not found", "warn"))

    def test_a_crashed_client_shows_its_exit_status(self):
        v = ds.describe({"target": "pc:1"}, exit_code=2)
        self.assertEqual((v.headline, v.level), ("Client stopped", "error"))
        self.assertIn("2", v.detail)


def wait_for(condition, timeout=5):
    """Poll until `condition()` is true; fail the test if it never is."""
    end = time.monotonic() + timeout
    while not condition():
        if time.monotonic() > end:
            raise AssertionError("timed out waiting for the condition")
        time.sleep(0.01)


def run_fake(script, restart_delay=0.05, timeout=5):
    """Run the supervisor over `python -c script`; collect views and stop events."""
    views, stopped = [], threading.Event()
    sup = ds.Supervisor([sys.executable, "-c", script], views.append, stopped.set, restart_delay)
    sup.start()
    return sup, views, stopped


class Supervise(unittest.TestCase):
    def test_status_lines_become_views_and_exit_zero_stops_the_app(self):
        sup, views, stopped = run_fake(
            "print('log noise', flush=True)\n"
            "print('status target=pc:24800', flush=True)\n"
            "print('status controller=ok', flush=True)\n"
            "print('status server=reachable', flush=True)\n"
            "raise SystemExit(0)\n"
        )
        self.assertTrue(stopped.wait(5), "on_stopped was not called")
        sup.join(5)
        self.assertEqual(views[-1], ds.View("Streaming to PC", "pc:24800", "ok"))
        self.assertEqual(views[0].headline, "Starting...")

    def test_an_abnormal_exit_restarts_the_client(self):
        sup, views, stopped = run_fake(
            "print('status target=pc:1', flush=True)\n"
            "raise SystemExit(2)\n"
        )
        wait_for(lambda: len([v for v in views if v.headline == "Client stopped"]) >= 2)
        sup.stop()
        sup.join(5)
        self.assertFalse(stopped.is_set(), "a failure must not close the app")

    def test_stop_terminates_the_client(self):
        sup, views, stopped = run_fake("import time\nprint('status target=pc:1', flush=True)\ntime.sleep(60)\n")
        wait_for(lambda: any(v.headline == "Streaming to PC" for v in views))
        sup.stop()
        sup.join(5)
        self.assertFalse(sup._thread.is_alive())
        self.assertFalse(stopped.is_set())

    def test_a_missing_client_binary_is_reported(self):
        views = []
        sup = ds.Supervisor(["/nonexistent/client"], views.append, lambda: None)
        sup.start()
        sup.join(5)
        self.assertEqual(views[-1].headline, "Cannot start client")


if __name__ == "__main__":
    unittest.main()
