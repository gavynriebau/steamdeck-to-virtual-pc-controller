"""The Deck's file layout is defined once, in deploy/deck-layout.env. These tests
fail if anything else repeats it or disagrees with it.
Run: python3 -m unittest discover -s deploy/tests"""
import importlib.util
import os
import re
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", ".."))

spec = importlib.util.spec_from_file_location("shortcut", os.path.join(ROOT, "deploy", "add_steam_shortcut.py"))
shortcut = importlib.util.module_from_spec(spec)
spec.loader.exec_module(shortcut)

LAYOUT = shortcut.read_layout(os.path.join(ROOT, "deploy", "deck-layout.env"))


def read(*path):
    with open(os.path.join(ROOT, *path)) as f:
        return f.read()


class Manifest(unittest.TestCase):
    def test_it_defines_every_name_the_tools_need(self):
        for key in ["DECK_CLIENT", "DECK_APP", "DECK_SUPERVISOR", "DECK_BIN_DIR", "DECK_APPS_DIR"]:
            self.assertTrue(LAYOUT.get(key), f"{key} missing from deck-layout.env")

    def test_the_files_it_names_exist_in_the_repo(self):
        self.assertTrue(os.path.exists(os.path.join(ROOT, "deck", LAYOUT["DECK_SUPERVISOR"])))
        self.assertTrue(os.path.exists(os.path.join(ROOT, "deck", LAYOUT["DECK_APP"] + ".py")))
        self.assertTrue(os.path.exists(os.path.join(ROOT, "crates", "deck", "src", "bin", LAYOUT["DECK_CLIENT"] + ".rs")),
                        "DECK_CLIENT must be the client binary's name")


class Agreement(unittest.TestCase):
    def test_the_desktop_file_runs_the_installed_app(self):
        desktop = read("deploy", LAYOUT["DECK_CLIENT"] + ".desktop")
        self.assertIn(f"Exec=%h/{LAYOUT['DECK_BIN_DIR']}/{LAYOUT['DECK_APP']}\n", desktop)

    def test_the_steam_shortcut_points_at_the_installed_app(self):
        expected = '"' + os.path.expanduser(f"~/{LAYOUT['DECK_BIN_DIR']}/{LAYOUT['DECK_APP']}") + '"'
        self.assertEqual(shortcut.EXE, expected)

    def test_the_app_looks_for_the_client_under_the_manifest_name(self):
        app = read("deck", LAYOUT["DECK_APP"] + ".py")
        self.assertIn(f'"{LAYOUT["DECK_CLIENT"]}"', app)


class NoRepeats(unittest.TestCase):
    """Files that must take the layout from the manifest, not spell it out."""

    FILES = [
        ("scripts", "lib.sh"),
        ("scripts", "install-deck.sh"),
        ("scripts", "uninstall-deck.sh"),
        ("deploy", "deck-ctl.sh"),
        ("deploy", "add_steam_shortcut.py"),
    ]

    def test_install_directories_are_not_spelled_out(self):
        for f in self.FILES:
            text = read(*f)
            for key in ["DECK_BIN_DIR", "DECK_APPS_DIR"]:
                self.assertNotIn(LAYOUT[key], text, f"{'/'.join(f)} repeats {LAYOUT[key]}")

    def test_the_app_and_client_names_are_not_spelled_out(self):
        for f in self.FILES:
            text = read(*f)
            # The repo's own source file (deck-controller-app.py) is not the installed name.
            without_source_name = text.replace(LAYOUT["DECK_APP"] + ".py", "")
            self.assertNotIn(LAYOUT["DECK_APP"], without_source_name, f"{'/'.join(f)} repeats the app name")
            self.assertNotIn(LAYOUT["DECK_CLIENT"], text, f"{'/'.join(f)} repeats the client name")

    def test_the_helper_refuses_to_run_without_the_manifest_values(self):
        helper = read("deploy", "deck-ctl.sh")
        for key in ["DECK_CLIENT", "DECK_APP", "DECK_SUPERVISOR", "DECK_BIN_DIR", "DECK_APPS_DIR"]:
            self.assertRegex(helper, rf'\$\{{{key}:\?')


if __name__ == "__main__":
    unittest.main()
