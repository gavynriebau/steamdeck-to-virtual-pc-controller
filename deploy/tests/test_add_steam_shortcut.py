"""Tests for the Steam shortcuts.vdf reader/writer. Run: python3 -m unittest discover -s deploy/tests"""
import importlib.util
import os
import unittest
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("shortcut", os.path.join(HERE, "..", "add_steam_shortcut.py"))
shortcut = importlib.util.module_from_spec(spec)
spec.loader.exec_module(shortcut)

with open(os.path.join(HERE, "fixtures", "shortcuts.vdf"), "rb") as f:
    FIXTURE = f.read()


def entries(items):
    """AppNames of the shortcuts in a parsed file, in order."""
    section = next(v for kind, key, v in items if kind == 0 and key.lower() == "shortcuts")
    return [dict((k, v) for _, k, v in e[2]).get("AppName") for e in section], section


class RoundTrip(unittest.TestCase):
    def test_parse_then_serialize_is_identical(self):
        items, end = shortcut.parse(FIXTURE)
        self.assertEqual(end, len(FIXTURE))
        self.assertEqual(shortcut.serialize(items), FIXTURE)

    def test_unsupported_value_type_is_rejected(self):
        with self.assertRaises(ValueError):
            shortcut.parse(b"\x07key\x00\x00\x00\x00\x00\x08")


class Edit(unittest.TestCase):
    def setUp(self):
        self.items, _ = shortcut.parse(FIXTURE)

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
        self.assertEqual(shortcut.serialize(removed), FIXTURE)

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


if __name__ == "__main__":
    unittest.main()
