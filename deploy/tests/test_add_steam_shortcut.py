"""Tests for the Steam shortcuts.vdf reader/writer. Run: python3 -m unittest discover -s deploy/tests"""
import contextlib
import importlib.util
import io
import os
import shutil
import sys
import tempfile
import unittest
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("shortcut", os.path.join(HERE, "..", "add_steam_shortcut.py"))
shortcut = importlib.util.module_from_spec(spec)
spec.loader.exec_module(shortcut)
steam_vdf = sys.modules["steam_vdf"]  # imported by the tool itself

with open(os.path.join(HERE, "fixtures", "shortcuts.vdf"), "rb") as f:
    FIXTURE = f.read()


def entries(items):
    """AppNames of the shortcuts in a parsed file, in order."""
    section = next(v for kind, key, v in items if kind == 0 and key.lower() == "shortcuts")
    return [dict((k, v) for _, k, v in e[2]).get("AppName") for e in section], section


class Edit(unittest.TestCase):
    def setUp(self):
        self.items, _ = steam_vdf.parse(FIXTURE)

    def test_add_appends_one_entry_with_next_index(self):
        updated = shortcut.edit(self.items, remove=False)
        names, section = entries(updated)
        self.assertEqual(names, ["Game One", "Game Two", shortcut.NAME])
        self.assertEqual(section[-1][1], "2")
        appid = dict((k, v) for _, k, v in section[-1][2])["appid"]
        self.assertEqual(appid, zlib.crc32((shortcut.EXE + shortcut.NAME).encode()) | 0x80000000)

    def test_add_keeps_existing_entries_untouched(self):
        updated = shortcut.edit(self.items, remove=False)
        _, before = entries(self.items)
        _, after = entries(updated)
        self.assertEqual(after[:2], before)

    def test_add_is_idempotent(self):
        updated = shortcut.edit(self.items, remove=False)
        self.assertIsNone(shortcut.edit(updated, remove=False))

    def test_remove_restores_the_original_bytes(self):
        added = shortcut.edit(self.items, remove=False)
        removed = shortcut.edit(added, remove=True)
        self.assertEqual(steam_vdf.serialize(removed), FIXTURE)

    def test_remove_keeps_indices_consecutive(self):
        added = shortcut.edit(self.items, remove=False)
        # Put our entry first, then remove it: the others must renumber 0, 1.
        _, section = entries(added)
        ours = section[-1]
        reordered = [(0, "shortcuts", [(0, "0", ours[2])] + [(0, str(n + 1), e[2]) for n, e in enumerate(section[:-1])])]
        removed = shortcut.edit(reordered, remove=True)
        names, section = entries(removed)
        self.assertEqual(names, ["Game One", "Game Two"])
        self.assertEqual([e[1] for e in section], ["0", "1"])

    def test_remove_when_absent_changes_nothing(self):
        self.assertIsNone(shortcut.edit(self.items, remove=True))

    def test_add_to_a_file_without_a_shortcuts_section(self):
        updated = shortcut.edit([], remove=False)
        names, _ = entries(updated)
        self.assertEqual(names, [shortcut.NAME])


class FakeProcess:
    """Stands in for subprocess.run: answers pgrep from a script and records the rest."""

    def __init__(self, pgrep_answers):
        self.pgrep_answers = list(pgrep_answers)  # True = Steam is running
        self.calls = []

    def __call__(self, cmd, **kwargs):
        self.calls.append(cmd)

        class Result:
            pass

        r = Result()
        if cmd[0] == "pgrep":
            running = self.pgrep_answers.pop(0) if len(self.pgrep_answers) > 1 else self.pgrep_answers[0]
            r.returncode = 0 if running else 1
        else:
            r.returncode = 0
        return r

    def shutdown_calls(self):
        return [c for c in self.calls if c[-1] == "-shutdown"]


def fake_steam_root(tmp):
    root = os.path.join(tmp, "Steam")
    os.makedirs(os.path.join(root, "ubuntu12_32"))
    open(os.path.join(root, "ubuntu12_32", "steam"), "w").close()
    return root


class SteamProcessTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.tmp)
        self.root = fake_steam_root(self.tmp)

    def process(self, run, wait_seconds=5):
        self.sleeps = []
        return shortcut.SteamProcess(roots=[self.root], run=run, sleep=self.sleeps.append, wait_seconds=wait_seconds)

    def test_a_steam_that_is_not_running_is_left_alone(self):
        run = FakeProcess([False])
        self.process(run).shutdown()
        self.assertEqual(run.shutdown_calls(), [])

    def test_a_running_steam_is_asked_to_quit_once_and_waited_for(self):
        run = FakeProcess([True, True, True, False])  # still up for two polls after the request
        with contextlib.redirect_stdout(io.StringIO()):
            proc = self.process(run)
            proc.shutdown()
        self.assertEqual(len(run.shutdown_calls()), 1)
        self.assertEqual(run.shutdown_calls()[0][0], os.path.join(self.root, "ubuntu12_32", "steam"))
        self.assertEqual(len(self.sleeps), 2, "it waited while Steam was still running")

    def test_a_steam_that_will_not_quit_stops_the_tool_with_a_message(self):
        run = FakeProcess([True])
        with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(SystemExit) as raised:
            self.process(run, wait_seconds=3).shutdown()
        self.assertIn("Steam did not exit", str(raised.exception))
        self.assertEqual(len(self.sleeps), 3)

    def test_it_uses_the_first_root_that_has_a_steam_binary(self):
        missing = os.path.join(self.tmp, "nothing-here")
        run = FakeProcess([True, False])
        proc = shortcut.SteamProcess(roots=[missing, self.root], run=run, sleep=lambda s: None)
        with contextlib.redirect_stdout(io.StringIO()):
            proc.shutdown()
        self.assertEqual(run.shutdown_calls()[0][0], os.path.join(self.root, "ubuntu12_32", "steam"))


class FakeSteam:
    def __init__(self):
        self.shutdowns = 0

    def shutdown(self):
        self.shutdowns += 1


class MainTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.tmp)
        self.root = os.path.join(self.tmp, "Steam")
        self.steam = FakeSteam()

    def user(self, uid, content=FIXTURE):
        config = os.path.join(self.root, "userdata", str(uid), "config")
        os.makedirs(config, exist_ok=True)
        path = os.path.join(config, "shortcuts.vdf")
        with open(path, "wb") as f:
            f.write(content)
        return path

    def run_main(self, *argv):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            shortcut.main(list(argv), steam=self.steam, roots=[self.root], stamp="T")
        return out.getvalue()

    def names(self, path):
        with open(path, "rb") as f:
            items, _ = steam_vdf.parse(f.read())
        return entries(items)[0]

    def test_adding_edits_the_file_backs_it_up_and_stops_steam_once(self):
        path = self.user(1)
        out = self.run_main()
        self.assertEqual(self.names(path), ["Game One", "Game Two", shortcut.NAME])
        with open(path + ".bak-T", "rb") as f:
            self.assertEqual(f.read(), FIXTURE, "the backup is the original")
        self.assertEqual(self.steam.shutdowns, 1)
        self.assertIn("added", out)

    def test_when_the_entry_is_already_there_nothing_happens_and_steam_keeps_running(self):
        path = self.user(1)
        self.run_main()
        self.steam.shutdowns = 0
        with open(path, "rb") as f:
            after_first = f.read()
        out = self.run_main()
        self.assertEqual(self.steam.shutdowns, 0)
        self.assertIn("already present", out)
        with open(path, "rb") as f:
            self.assertEqual(f.read(), after_first)

    def test_removing_takes_the_entry_out_again(self):
        path = self.user(1)
        self.run_main()
        out = self.run_main("--remove")
        self.assertEqual(self.names(path), ["Game One", "Game Two"])
        self.assertIn("removed", out)

    def test_removing_when_it_is_not_there_does_not_touch_steam(self):
        self.user(1)
        out = self.run_main("--remove")
        self.assertEqual(self.steam.shutdowns, 0)
        self.assertIn("already removed", out)

    def test_every_steam_user_is_handled_with_one_shutdown(self):
        first, second = self.user(1), self.user(2)
        self.run_main()
        self.assertIn(shortcut.NAME, self.names(first))
        self.assertIn(shortcut.NAME, self.names(second))
        self.assertEqual(self.steam.shutdowns, 1)

    def test_only_users_who_need_a_change_are_rewritten(self):
        done, fresh = self.user(1), self.user(2)
        self.run_main()  # both get the entry
        with open(fresh, "wb") as f:
            f.write(FIXTURE)  # the second user loses it again
        with open(done, "rb") as f:
            done_bytes = f.read()
        os.remove(done + ".bak-T")  # a rewrite of the first user would recreate this
        self.steam.shutdowns = 0

        self.run_main()
        self.assertFalse(os.path.exists(done + ".bak-T"), "the first user needed nothing")
        with open(done, "rb") as f:
            self.assertEqual(f.read(), done_bytes)
        self.assertTrue(os.path.exists(fresh + ".bak-T"), "the second user was backed up")
        self.assertIn(shortcut.NAME, self.names(fresh))
        self.assertEqual(self.steam.shutdowns, 1)

    def test_no_steam_userdata_is_an_error_when_adding_and_fine_when_removing(self):
        with self.assertRaises(SystemExit) as raised:
            self.run_main()
        self.assertIn("start Steam once", str(raised.exception))
        self.assertIn("nothing to remove", self.run_main("--remove"))
        self.assertEqual(self.steam.shutdowns, 0)

    def test_a_file_with_trailing_data_is_refused_untouched_and_steam_is_left_running(self):
        path = self.user(1, FIXTURE + b"garbage")
        with self.assertRaises(SystemExit) as raised:
            self.run_main()
        self.assertIn("trailing data", str(raised.exception))
        self.assertEqual(self.steam.shutdowns, 0)
        with open(path, "rb") as f:
            self.assertEqual(f.read(), FIXTURE + b"garbage")


if __name__ == "__main__":
    unittest.main()