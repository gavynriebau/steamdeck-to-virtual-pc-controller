"""Tests for the binary VDF format. Run: python3 -m unittest discover -s deploy/tests"""
import os
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, ".."))
import steam_vdf as vdf  # noqa: E402

with open(os.path.join(HERE, "fixtures", "shortcuts.vdf"), "rb") as f:
    FIXTURE = f.read()


def s(key, value):
    return b"\x01" + key + b"\x00" + value + b"\x00"


def i(key, value):
    return b"\x02" + key + b"\x00" + value.to_bytes(4, "little")


class RoundTrip(unittest.TestCase):
    def test_a_real_shaped_file_round_trips_byte_for_byte(self):
        items, end = vdf.parse(FIXTURE)
        self.assertEqual(end, len(FIXTURE))
        self.assertEqual(vdf.serialize(items), FIXTURE)

    def test_nested_sections_strings_and_integers(self):
        data = b"\x00outer\x00" + b"\x00inner\x00" + s(b"name", b"Game") + i(b"n", 7) + b"\x08" + b"\x08" + b"\x08"
        items, end = vdf.parse(data)
        self.assertEqual(end, len(data))
        self.assertEqual(items, [(0, "outer", [(0, "inner", [(1, "name", "Game"), (2, "n", 7)])])])
        self.assertEqual(vdf.serialize(items), data)

    def test_non_ascii_text_survives(self):
        data = s(b"AppName", "Pokémon — ゲーム".encode("utf-8")) + b"\x08"
        items, _ = vdf.parse(data)
        self.assertEqual(items, [(1, "AppName", "Pokémon — ゲーム")])
        self.assertEqual(vdf.serialize(items), data)

    def test_empty_sections_and_empty_strings(self):
        data = b"\x00tags\x00\x08" + s(b"icon", b"") + b"\x08"
        items, _ = vdf.parse(data)
        self.assertEqual(items, [(0, "tags", []), (1, "icon", "")])
        self.assertEqual(vdf.serialize(items), data)

    def test_integers_are_unsigned_32_bit(self):
        data = i(b"appid", 0xDEADBEEF) + b"\x08"
        items, _ = vdf.parse(data)
        self.assertEqual(items, [(2, "appid", 0xDEADBEEF)])
        self.assertEqual(vdf.serialize(items), data)


class Errors(unittest.TestCase):
    def test_an_unsupported_value_type_is_rejected(self):
        with self.assertRaises(ValueError):
            vdf.parse(b"\x07key\x00\x00\x00\x00\x00\x08")

    def test_data_after_the_closing_marker_is_reported_by_the_end_position(self):
        _, end = vdf.parse(i(b"a", 1) + b"\x08" + b"trailing")
        self.assertEqual(end, len(i(b"a", 1)) + 1)


if __name__ == "__main__":
    unittest.main()
