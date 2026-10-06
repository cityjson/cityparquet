"""The `--statistic median|mean` option, end to end: the default, the value
and spread it selects, the caption every timing figure prints, and the
statistic column of the factor CSV."""

import csv
import json
import shutil
from pathlib import Path

import matplotlib.pyplot as plt
import pytest
from matplotlib.text import Text

from benchviz import __main__ as cli
from benchviz import figures, prep, tables

FIXTURES = Path(__file__).parent / "fixtures"
LIVE = Path(__file__).parents[2] / "formats"


def _bench(tmp_path: Path) -> Path:
    bench = tmp_path / "benchmark" / "formats"
    shutil.copytree(FIXTURES / "benchviz", bench)
    shutil.copy(LIVE / "READ_BENCHMARK.md", bench / "READ_BENCHMARK.md")
    shutil.copytree(FIXTURES / "benchviz-databases", tmp_path / "benchmark" / "databases")
    return bench


def _texts(monkeypatch, render) -> dict[str, str]:
    """Every figure `render` saves, by name, as the concatenation of its text."""
    seen: dict[str, str] = {}

    def capture(fig, name, out):
        seen[name] = "\n".join(t.get_text() for t in fig.findobj(Text))
        plt.close(fig)
        return []

    monkeypatch.setattr(figures, "_save", capture)
    render()
    return seen


def test_the_cli_defaults_to_the_median():
    for command in ("prep", "all", "summary"):
        assert cli.build_parser().parse_args([command]).statistic == "median"
    args = cli.build_parser().parse_args(["summary", "--statistic", "mean"])
    assert args.statistic == "mean"
    with pytest.raises(SystemExit):
        cli.build_parser().parse_args(["prep", "--statistic", "mode"])


def test_build_records_the_statistic_and_its_caption(tmp_path: Path):
    bench = _bench(tmp_path)
    median, _ = prep.build(prep.Inputs(bench))
    mean, _ = prep.build(prep.Inputs(bench), statistic="mean")
    assert (median["statistic"], mean["statistic"]) == ("median", "mean")
    assert median["meta"]["statistic_note"].startswith("Times: median of ")
    assert "interquartile range" in median["meta"]["statistic_note"]
    assert mean["meta"]["statistic_note"].startswith("Times: mean of ")
    assert "standard deviation" in mean["meta"]["statistic_note"]


def _skew_means(bench: Path) -> None:
    """The fixtures carry mean == median; double every mean so the two differ."""
    for path in bench.rglob("*.csv"):
        rows = list(csv.DictReader(path.open(encoding="utf-8", newline="")))
        if not rows or "time_mean_s" not in rows[0]:
            continue
        for row in rows:
            if row["time_mean_s"]:
                row["time_mean_s"] = repr(2 * float(row["time_mean_s"]))
        with path.open("w", encoding="utf-8", newline="") as handle:
            writer = csv.DictWriter(handle, fieldnames=list(rows[0]))
            writer.writeheader()
            writer.writerows(rows)


def test_mean_switches_the_plotted_value_and_spread(tmp_path: Path):
    bench = _bench(tmp_path)
    _skew_means(bench)
    median, _ = prep.build(prep.Inputs(bench))
    mean, _ = prep.build(prep.Inputs(bench), statistic="mean")
    pairs = [
        (m, a)
        for m, a in zip(median["read"], mean["read"], strict=True)
        if m["time_s"] is not None and m["time_lo_s"] is not None
    ]
    assert pairs
    assert any(m["time_s"] != a["time_s"] for m, a in pairs)
    for _m, a in pairs:
        std = a["time_hi_s"] - a["time_s"]
        assert a["time_s"] - a["time_lo_s"] == pytest.approx(std)


def test_every_timing_figure_prints_the_statistic(tmp_path: Path, monkeypatch):
    bench = _bench(tmp_path)
    for statistic in prep.STATISTICS:
        data, _ = prep.build(prep.Inputs(bench), statistic=statistic)
        note = data["meta"]["statistic_note"]
        seen = _texts(
            monkeypatch,
            lambda data=data: (
                figures.format_figures(data, tmp_path),
                figures._axis_main(data, "bloom", tmp_path),
                figures.databases(data, tmp_path),
            ),
        )
        assert {"databases", "bloom"} <= set(seen)
        for name, text in seen.items():
            # Storage and memory carry no timing statistic; a placeholder plots nothing.
            if name in ("sizes", "rss") or "Not rendered" in text:
                continue
            assert note in text.replace("\n", " "), (statistic, name)
        assert "Mean query time" not in seen["databases"] or statistic == "mean"
        assert f"{statistic.capitalize()} query time" in seen["databases"]


def test_the_query_factor_csv_carries_the_statistic(tmp_path: Path):
    bench = _bench(tmp_path)
    for statistic in prep.STATISTICS:
        data, _ = prep.build(prep.Inputs(bench), statistic=statistic)
        written = tables.write_tables(data, tmp_path / statistic)
        path = next(p for p in written if p.name == tables.QUERY_TABLE)
        rows = list(csv.DictReader(path.open(encoding="utf-8")))
        assert rows and {r["statistic"] for r in rows} == {statistic}


def test_the_cli_threads_the_statistic_into_bench_data(tmp_path: Path):
    bench = _bench(tmp_path)
    args = cli.build_parser().parse_args(
        ["prep", "--bench-dir", str(bench), "--out", str(tmp_path / "out"), "--statistic", "mean"]
    )
    cli._cmd_prep(args)
    payload = json.loads((tmp_path / "out" / "bench_data.json").read_text())
    assert payload["statistic"] == "mean"
    from benchviz import html

    page = html.main(
        data_path=tmp_path / "out" / "bench_data.json",
        out_path=tmp_path / "out" / "page.html",
        figures_dir=tmp_path / "out" / "figures",
    )
    assert payload["meta"]["statistic_note"] in page.read_text(encoding="utf-8")
