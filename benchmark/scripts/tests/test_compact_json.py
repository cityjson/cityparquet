"""Tests for compact_json.py: whitespace outside strings goes, nothing else."""

import json
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from compact_json import compact  # noqa: E402


class CompactTest(unittest.TestCase):
    def test_whitespace_outside_strings_is_removed(self) -> None:
        pretty = b'{\n  "a" : [1, 2],\r\n\t"b": {"c": null}\n}\n'
        self.assertEqual(compact(pretty), b'{"a":[1,2],"b":{"c":null}}')

    def test_string_contents_survive_including_escaped_quotes(self) -> None:
        doc = b'{ "k": "a \\" b\\\\", "u": "\xc3\xa9 x" }'
        self.assertEqual(compact(doc), b'{"k":"a \\" b\\\\","u":"\xc3\xa9 x"}')
        self.assertEqual(json.loads(compact(doc)), json.loads(doc))

    def test_number_spellings_are_kept_byte_for_byte(self) -> None:
        doc = b'{"scale": [1e-10, 1.0e-06, 1.50], "v": [9007199254740993]}'
        self.assertEqual(compact(doc), b'{"scale":[1e-10,1.0e-06,1.50],"v":[9007199254740993]}')

    def test_a_compact_document_is_unchanged(self) -> None:
        doc = b'{"type":"CityJSON","CityObjects":{},"vertices":[]}'
        self.assertEqual(compact(doc), doc)


if __name__ == "__main__":
    unittest.main()
