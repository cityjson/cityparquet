"""The scripts derive megabytes with the one decimal-unit helper, `benchviz.units`.

A second copy of "bytes / 10^6" is how a binary megabyte crept in before; the
scripts import the plotting project's helper instead of restating it.
"""

import sys
import unittest
from pathlib import Path

SCRIPTS = Path(__file__).parents[1]
sys.path.insert(0, str(SCRIPTS))
import compression_contribution
import measure_sizes
from benchviz import units


class SharedUnitsTests(unittest.TestCase):
    def test_measure_sizes_uses_the_shared_helper(self):
        self.assertIs(measure_sizes.units, units)
        self.assertFalse(hasattr(measure_sizes, "BYTES_PER_MB"))

    def test_compression_contribution_uses_the_shared_helper(self):
        self.assertIs(compression_contribution.units, units)
        self.assertFalse(hasattr(compression_contribution, "BYTES_PER_MB"))
        self.assertFalse(hasattr(compression_contribution, "mb_decimal"))

    def test_the_csv_spelling_is_decimal(self):
        self.assertEqual(units.mb_decimal(999_999), "0.999999")
        self.assertEqual(units.mb_decimal(1_000_000), "1.000000")


if __name__ == "__main__":
    unittest.main()
