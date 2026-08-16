"""One format vocabulary, one colour map, and a GML-native dataset that is visible.

Three defects are pinned here, all of them ways a chart can come out quietly
wrong rather than loudly broken:

1. `FORMAT_ORDER` used to exist twice — once in `plot.py` (six entries) and
   once in `sizes.py` (five, missing `duckdb-parquet`). A copy of a vocabulary
   is exactly how this benchmark's CSV header contract drifted into three
   incompatible versions, so the canonical list is read out of
   `crates/cityparquet-readbench/src/format.rs` here, the same trick
   `test_csv_contract.py` uses for the CSV header and
   `scripts/tests/readbench_prepare_test.sh` case 9 uses for the prepare
   script's tag list.
2. `plot.py` had no colour map at all, so bars took matplotlib's default
   cycle *by draw order* — adding one format silently recoloured every
   previously published chart. The colours are therefore pinned BY NAME
   below: this test is the tripwire that makes changing a published figure's
   colours a deliberate act.
3. `sizes.py` seeded dataset discovery from `.fcb`/`.jsonl.gz` siblings only
   and derived its baseline from the gzip ISIZE trailer, an artefact that
   exists only if the input was CityJSON. A dataset prepared from CityGML has
   neither, so it was invisible and its ratio uncomputable.
"""

import gzip
import re
from pathlib import Path

import readbench_plot
from readbench_plot import FORMAT_COLORS, FORMAT_ORDER, bar_style
from readbench_plot import plot as plot_mod
from readbench_plot import sizes as sizes_mod

REPO = Path(__file__).resolve().parents[3]


def _rust_format_tags() -> list[str]:
    """The canonical tag list, read from `Format::as_str`'s match arms."""
    src = (REPO / "crates/cityparquet-readbench/src/format.rs").read_text()
    m = re.search(r"pub fn as_str\(self\).*?\n    \}", src, re.S)
    assert m, "could not find Format::as_str in format.rs"
    tags = re.findall(r'=> "([a-z0-9-]+)",', m.group(0))
    assert tags, "could not read the tag list out of format.rs"
    return tags


# ---------------------------------------------------------------------------
# Defect 1: one vocabulary
# ---------------------------------------------------------------------------


def test_format_order_matches_the_rust_enum():
    # Rust first: `Format::ALL` is the authority, so it reads as the
    # expectation (and ruff's SIM300 rejects the other order).
    assert _rust_format_tags() == FORMAT_ORDER


def test_both_modules_share_one_format_order():
    """Not merely equal — the same object, so the copies cannot be reborn."""
    assert plot_mod.FORMAT_ORDER is readbench_plot.FORMAT_ORDER
    assert sizes_mod.FORMAT_ORDER is readbench_plot.FORMAT_ORDER


# ---------------------------------------------------------------------------
# Defect 2: colours keyed by name, not by draw order
# ---------------------------------------------------------------------------

# The published assignment. The five formats `sizes.py` already charted keep
# the exact colours they had when it derived them from tab10 by position, so
# no figure rendered before this change moves; the three formats added since
# take the unused tab10 slots. Changing a value here changes a figure in the
# paper — that is what this pin is for.
PUBLISHED_COLORS = {
    "citygml": "#8c564b",
    "cityjson": "#e377c2",
    "cityjsonseq": "#1f77b4",
    "cityjsonseq-gz": "#ff7f0e",
    "flatcitybuf": "#2ca02c",
    "cityparquet": "#d62728",
    "cityparquet-hilbert": "#9467bd",
    "duckdb-parquet": "#7f7f7f",
}


def test_every_format_has_a_colour():
    missing = [f for f in _rust_format_tags() if f not in FORMAT_COLORS]
    assert missing == []


def test_colours_are_pinned_by_name():
    """Adding a format must not shift any existing format's colour."""
    assert dict(FORMAT_COLORS) == PUBLISHED_COLORS


def test_colours_are_distinguishable():
    assert len(set(FORMAT_COLORS.values())) == len(FORMAT_COLORS)


def test_both_modules_share_one_colour_map():
    assert plot_mod.FORMAT_COLORS is readbench_plot.FORMAT_COLORS
    assert sizes_mod.FORMAT_COLORS is readbench_plot.FORMAT_COLORS


def test_an_unknown_format_is_drawn_but_never_steals_a_colour():
    style = bar_style("some-future-format")
    assert style["color"] not in set(FORMAT_COLORS.values())
    # Hatching, so an unknown format is obvious in print, not merely a
    # slightly different grey.
    assert style["hatch"]


def test_a_known_format_is_drawn_plainly():
    style = bar_style("cityparquet-hilbert")
    assert style["color"] == PUBLISHED_COLORS["cityparquet-hilbert"]
    assert not style["hatch"]


def test_an_unknown_format_is_ordered_last_not_dropped():
    assert plot_mod._ordered(
        ["some-future-format", "cityparquet"], FORMAT_ORDER
    ) == ["cityparquet", "some-future-format"]


# ---------------------------------------------------------------------------
# Defect 3: a GML-native dataset is discoverable and measurable
# ---------------------------------------------------------------------------


def _write(path: Path, size: int) -> Path:
    path.write_bytes(b"x" * size)
    return path


def test_discovers_a_dataset_whose_only_artefacts_are_gml_and_cityjson(tmp_path):
    _write(tmp_path / "alkmaar.gml", 4000)
    _write(tmp_path / "alkmaar.city.json", 2000)
    assert sizes_mod.discover_datasets(tmp_path) == ["alkmaar"]


def test_discovers_a_dataset_whose_only_artefact_is_a_gml(tmp_path):
    """The narrowest GML-native case: a prepare run given `--formats citygml`."""
    _write(tmp_path / "alkmaar.gml", 4000)
    assert sizes_mod.discover_datasets(tmp_path) == ["alkmaar"]


def test_discovers_a_dataset_whose_only_artefact_is_a_cityjson(tmp_path):
    _write(tmp_path / "alkmaar.city.json", 2000)
    assert sizes_mod.discover_datasets(tmp_path) == ["alkmaar"]


def test_hilbert_package_is_not_a_dataset_of_its_own(tmp_path):
    _write(tmp_path / "delft.gml", 10)
    (tmp_path / "delft.parquet").mkdir()
    _write(tmp_path / "delft.parquet" / "building.parquet", 10)
    (tmp_path / "delft-hilbert.parquet").mkdir()
    _write(tmp_path / "delft-hilbert.parquet" / "building.parquet", 10)
    assert sizes_mod.discover_datasets(tmp_path) == ["delft"]


def test_measures_citygml_and_cityjson_artefacts(tmp_path):
    _write(tmp_path / "alkmaar.gml", 4000)
    _write(tmp_path / "alkmaar.city.json", 2000)
    rows = {r["format"]: r for r in sizes_mod.measure_dataset("alkmaar", tmp_path)}
    assert rows["citygml"]["bytes"] == 4000
    assert rows["cityjson"]["bytes"] == 2000


def test_gml_native_dataset_gets_a_ratio_against_its_source(tmp_path):
    _write(tmp_path / "alkmaar.gml", 4000)
    _write(tmp_path / "alkmaar.city.json", 2000)
    rows = {r["format"]: r for r in sizes_mod.measure_dataset("alkmaar", tmp_path)}
    assert rows["cityjson"]["baseline_format"] == "citygml"
    assert rows["cityjson"]["ratio_vs_baseline"] == 2.0
    assert rows["citygml"]["ratio_vs_baseline"] == 1.0


def test_raw_cityjsonseq_is_measured_from_the_file_when_there_is_one(tmp_path):
    """A prepared `.city.jsonl` beats guessing the size from a gzip trailer."""
    _write(tmp_path / "alkmaar.gml", 4000)
    _write(tmp_path / "alkmaar.city.jsonl", 3000)
    _write(tmp_path / "alkmaar.fcb", 1500)
    rows = {r["format"]: r for r in sizes_mod.measure_dataset("alkmaar", tmp_path)}
    assert rows["cityjsonseq"]["bytes"] == 3000
    # CityJSONSeq is back, so the baseline is CityJSONSeq again, not the GML.
    assert rows["flatcitybuf"]["baseline_format"] == "cityjsonseq"
    assert rows["flatcitybuf"]["ratio_vs_baseline"] == 2.0
    assert rows["flatcitybuf"]["ratio_vs_cityjsonseq"] == 2.0


def test_gzip_isize_still_supplies_the_raw_size_when_it_is_all_there_is(tmp_path):
    gz = tmp_path / "delft.jsonl.gz"
    gz.write_bytes(gzip.compress(b"y" * 3000))
    _write(tmp_path / "delft.fcb", 1500)
    rows = {r["format"]: r for r in sizes_mod.measure_dataset("delft", tmp_path)}
    assert rows["cityjsonseq"]["bytes"] == 3000
    assert rows["flatcitybuf"]["ratio_vs_cityjsonseq"] == 2.0


def test_report_carries_the_baseline_it_actually_used(tmp_path):
    _write(tmp_path / "alkmaar.gml", 4000)
    _write(tmp_path / "alkmaar.city.json", 2000)
    df = sizes_mod.build_report(tmp_path)
    assert list(df.columns) == [
        "dataset",
        "format",
        "bytes",
        "mb",
        "ratio_vs_cityjsonseq",
        "baseline_format",
        "ratio_vs_baseline",
    ]
    # No CityJSONSeq artefact anywhere, so the honestly-named column is empty
    # while the self-describing pair still carries a usable ratio.
    assert df["ratio_vs_cityjsonseq"].isna().all()
    assert set(df["baseline_format"]) == {"citygml"}


def _timings_with_an_unknown_format():
    import pandas as pd

    return pd.DataFrame(
        [
            {"dataset": "delft", "scenario": "full-read", "format": "cityparquet",
             "time_s": 1.0, "peak_heap_bytes": 10.0, "notes": ""},
            {"dataset": "delft", "scenario": "full-read", "format": "some-future-format",
             "time_s": 2.0, "peak_heap_bytes": 20.0, "notes": ""},
        ]
    )


def test_charts_render_with_an_unknown_format_present(tmp_path):
    """An unrecognised tag in a CSV must still plot, not blank the figure."""
    plot_mod.plot_dataset("delft", _timings_with_an_unknown_format(), tmp_path)
    assert (tmp_path / "delft-time.png").is_file()
    assert (tmp_path / "delft-mem.png").is_file()


def _spy_on_bar_style(monkeypatch, module) -> list[str]:
    """Record which formats a module asks for a bar style, keeping the real one."""
    seen: list[str] = []
    real = module.bar_style

    def spy(fmt: str):
        seen.append(fmt)
        return real(fmt)

    monkeypatch.setattr(module, "bar_style", spy)
    return seen


def test_timing_charts_colour_every_bar_by_format_name(tmp_path, monkeypatch):
    """plot.py used to pass no colour at all — the whole of defect 2."""
    seen = _spy_on_bar_style(monkeypatch, plot_mod)
    plot_mod.plot_dataset("delft", _timings_with_an_unknown_format(), tmp_path)
    assert set(seen) == {"cityparquet", "some-future-format"}


def test_size_charts_colour_every_bar_by_format_name(tmp_path, monkeypatch):
    _write(tmp_path / "alkmaar.gml", 4000)
    _write(tmp_path / "alkmaar.city.json", 2000)
    df = sizes_mod.build_report(tmp_path)
    seen = _spy_on_bar_style(monkeypatch, sizes_mod)
    sizes_mod.plot_sizes(df, tmp_path)
    assert set(seen) == {"citygml", "cityjson"}
