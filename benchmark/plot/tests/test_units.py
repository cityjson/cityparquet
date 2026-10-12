"""Byte counts are shown in decimal units: 1 MB = 10^6 bytes, 1 GB = 10^9 bytes.

`benchviz.units` is the one place a byte count becomes megabytes or gigabytes,
for the figures, the tables and the benchmark scripts alike.
"""

from benchviz import units


def test_the_units_are_decimal():
    assert units.MB == 1_000_000
    assert units.GB == 1_000_000_000


def test_format_bytes_reads_megabytes_below_a_gigabyte():
    assert units.format_bytes(0) == "0.0 MB"
    assert units.format_bytes(999_999) == "1.0 MB"
    assert units.format_bytes(1_000_000) == "1.0 MB"
    assert units.format_bytes(1_500_000) == "1.5 MB"
    assert units.format_bytes(999_999_999) == "1000.0 MB"


def test_format_bytes_reads_gigabytes_from_one_gigabyte():
    assert units.format_bytes(1_000_000_000) == "1.00 GB"
    assert units.format_bytes(1.5e9) == "1.50 GB"


def test_format_bytes_marks_a_missing_value():
    assert units.format_bytes(None) == "—"


def test_megabytes_and_the_csv_spelling():
    assert units.megabytes(1_500_000) == 1.5
    assert units.mb_decimal(999_999) == "0.999999"
    assert units.mb_decimal(1_000_000) == "1.000000"
    assert units.mb_decimal(1_500_000_000) == "1500.000000"


def test_size_unit_picks_one_unit_for_a_panel():
    assert units.size_unit([0, 999_999_999, None]) == ("MB", units.MB)
    assert units.size_unit([1, 1_000_000_000]) == ("GB", units.GB)
    assert units.size_unit([None]) == ("MB", units.MB)
