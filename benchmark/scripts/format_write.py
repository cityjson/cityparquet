#!/usr/bin/env python3
"""Measure format serialisation as isolated child processes.

Every row measures the SAME kind of work: parse the one canonical CityJSONSeq
stream and serialise it into the row's own format. `cjseq collect` writes
CityJSON, `+ citygml-tools` writes CityGML, `fcb ser -A` writes FlatCityBuf,
`cityparquet convert` writes a CityParquet package, and
`cityparquet-readbench write-cityjsonseq` writes CityJSONSeq back out
line-by-line through cjseq's typed model.

That last one is a writer, not a copy, and it has to be. The cityjsonseq row
is the baseline divisor for every write ratio in the plots, and it used to be
`cat <canonical> > target` — which on a reflink-capable filesystem (XFS with
reflink=1, coreutils >= 9.0) is a metadata-only extent clone: nothing is
parsed, nothing is serialised, and the "write" costs a few milliseconds at any
dataset size. `cjseq filter` is no substitute either: it parses each line to a
`serde_json::Value` and writes the ORIGINAL line bytes back.

Each sample creates a fresh artefact directory, so the idempotent preparation
script cannot turn a timed conversion into a cache hit. The raw samples are
kept beside the aggregate read CSVs; the aggregate uses median time and maximum
RSS, matching the readbench coordinator convention.

Completion contract: the timer stops when the converter exits, no `fsync` is
issued, and the output is deleted afterwards — so every row measures a write
into the page cache, never a durable write to the device.

`--formats` restricts the run to a subset. It is a smoke-run convenience — the
CityGML stage is by far the slowest and needs citygml-tools on disk — and never
belongs in a published run: a partial CSV is not a comparison.
"""
from __future__ import annotations
import argparse, csv, os, shutil, statistics, subprocess, tempfile, time
from pathlib import Path

ALL_FORMATS = ("citygml", "cityjson", "cityjsonseq", "flatcitybuf", "cityparquet-hilbert")
HEADER = ["dataset","format","scenario","selectivity","result_count","time_s","time_mad_s","peak_heap_bytes","peak_rss_bytes","repeat","notes","bytes_read","http_requests"]
SAMPLES_HEADER = ["dataset","format","scenario","sample","time_s","peak_rss_bytes"]


def median(values: list[float]) -> float:
    return statistics.median(values)


def mad(values: list[float], centre: float) -> float:
    return median([abs(value-centre) for value in values])


def base(path: Path) -> str:
    for suffix in (".city.jsonl", ".city.json", ".citygml", ".jsonl", ".json", ".gml", ".xml"):
        if path.name.endswith(suffix): return path.name[:-len(suffix)]
    return path.stem


GNU_TIME = "/usr/bin/time"


def require_gnu_time() -> None:
    """Check that `/usr/bin/time` understands `-f`/`-o`, or fail loudly.

    No silent fallback to `os.wait4`: the whole point of the wrapper is to
    escape the launcher floor documented in `run_child`, and a fallback would
    quietly reinstate it on exactly the hosts nobody is watching (macOS and
    BusyBox ship a `time` that rejects `-f`).
    """
    probe = subprocess.run([GNU_TIME, "-f", "%M", "-o", os.devnull, "true"], capture_output=True)
    if probe.returncode != 0:
        raise SystemExit(
            f"{GNU_TIME} does not accept -f/-o (GNU time is required to measure a converter's "
            f"own peak RSS): {probe.stderr.decode(errors='replace').strip()}"
        )


def run_child(command: list[str], stdout: Path | None = None) -> tuple[float, int]:
    """Run one converter and return wall time plus its own peak RSS.

    RSS comes from `/usr/bin/time -f %M`, NOT from `os.wait4`'s `ru_maxrss`.
    Under CPython 3.12 a child is spawned with `posix_spawn`/`vfork`, and the
    child's `ru_maxrss` is then at least the parent's own RSS: measured here,
    `/bin/true` under a parent holding 400 MB of ballast reports 433 MB, and
    the cityjsonseq row reported a constant 14 680 064 B on every dataset —
    the Python launcher's own footprint, not the converter's. Forcing plain
    `fork` does not help and cannot: `fork` hands the child the parent's
    copy-on-write address space, so its peak still includes the parent's
    pages (425 MB in the same probe). Only re-parenting the measurement under
    a tiny process escapes the floor; GNU `time` itself costs ~1-2 MB, which
    is the floor this reports instead.

    The wrapper adds one fork/exec, ~2-3 ms, uniformly to every stage.
    """
    stream = stdout.open("wb") if stdout is not None else None
    # A fresh file per call: GNU time appends nothing, but a stale value from a
    # failed read would silently become another stage's RSS.
    handle, rss_path = tempfile.mkstemp(prefix="rss.", suffix=".txt")
    os.close(handle)
    try:
        started = time.perf_counter()
        process = subprocess.Popen([GNU_TIME, "-f", "%M", "-o", rss_path, *command], stdout=stream)
        status = process.wait()
        elapsed = time.perf_counter() - started
        if status != 0:
            raise subprocess.CalledProcessError(status, command)
        # On a non-zero status GNU time prepends its own "Command exited with
        # non-zero status N" line, so the LAST line is always the %M value.
        # That path raises above, but reading the last line keeps this honest
        # if the check ever moves.
        with open(rss_path) as handle:
            lines = [line.strip() for line in handle if line.strip()]
        if not lines:
            raise SystemExit(f"{GNU_TIME} wrote no RSS value for {command[0]}")
        return elapsed, int(lines[-1]) * 1024
    finally:
        if stream is not None:
            stream.close()
        os.unlink(rss_path)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--canonical-seq", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--raw-out", type=Path, required=True)
    parser.add_argument("--scratch", type=Path, required=True)
    parser.add_argument("--repeat", type=int, default=3)
    # A smoke-run convenience only: the CityGML stage is by far the slowest and
    # needs citygml-tools on disk, so a pipeline check can leave it out. A
    # PUBLISHED run measures every format — a partial CSV is not a comparison.
    parser.add_argument("--formats", default="", help="comma-separated subset of " + ",".join(ALL_FORMATS) + " (default: all)")
    args = parser.parse_args()
    if args.repeat < 1: raise SystemExit("--repeat must be >= 1")
    formats = ALL_FORMATS
    if args.formats:
        formats = tuple(name.strip() for name in args.formats.split(",") if name.strip())
        unknown = [name for name in formats if name not in ALL_FORMATS]
        if unknown: raise SystemExit(f"unknown --formats entries {unknown}; expected any of {list(ALL_FORMATS)}")
    require_gnu_time()
    args.scratch.mkdir(parents=True, exist_ok=True)
    dataset = base(args.input)
    aggregates: list[list[str]] = []
    raw: list[list[str]] = []
    for fmt in formats:
        values: list[tuple[float,int]] = []
        for index in range(args.repeat + 1):
            output = Path(tempfile.mkdtemp(prefix=f"{dataset}.{fmt}.", dir=args.scratch))
            cityjson = output / "model.city.json"
            target = output / "target"
            # Every target starts from the same already-prepared canonical
            # CityJSONSeq. Only temporary-directory creation is outside timing.
            if fmt == "cityjson":
                stages = [(["cjseq", "collect", "-f", str(args.canonical_seq)], target)]
            elif fmt == "cityjsonseq":
                # A real writer: the readbench harness parses the canonical
                # stream line by line into cjseq's typed model and serialises
                # it straight back out. See this module's own docstring for
                # why `cat` (and `cjseq filter`) could not stand in for one.
                stages = [([str(Path(os.environ.get("READBENCH_BIN", "benchmark/readbench/target/release/cityparquet-readbench"))), "write-cityjsonseq", "--input", str(args.canonical_seq), "--output", str(target)], None)]
            elif fmt == "flatcitybuf":
                stages = [(["fcb", "ser", "-A", str(args.canonical_seq), str(target)], None)]
            elif fmt == "cityparquet-hilbert":
                stages = [([str(Path(os.environ.get("CITYPARQUET_BIN", "lib/cityparquet-rs/target/release/cityparquet"))), "convert", str(args.canonical_seq), "--output", str(target), "--overwrite", "--ordering", "hilbert"], None)]
            else:
                # CityGML requires two sequential writers. Both stages are in
                # this sample: time is their sum; RSS is the maximum active
                # converter process, never an invented combined value.
                stages = [
                    (["cjseq", "collect", "-f", str(args.canonical_seq)], cityjson),
                    ([str(Path(os.environ.get("CITYGML_TOOLS", "benchmark/formats/tools/citygml-tools/citygml-tools"))), "from-cityjson", "-v", "2.0", "--no-pretty-print", "-o", str(target), str(cityjson)], None),
                ]
            try:
                metrics = [run_child(command, stdout) for command, stdout in stages]
                elapsed = sum(value[0] for value in metrics)
                rss = max(value[1] for value in metrics)
                values.append((elapsed, rss))
            finally:
                shutil.rmtree(output)
        warm = values[1:]
        times = [value[0] for value in warm]
        centre = median(times)
        aggregates.append([dataset, fmt, "write", "", "0", f"{centre:.6f}", f"{mad(times, centre):.6f}", "", str(max(value[1] for value in warm)), str(args.repeat), "canonical-cityjsonseq;cityjsonseq=readbench-reserialise;citygml=seq-to-json+json-to-gml", "", ""])
        raw.extend([dataset, fmt, "write", str(index + 1), f"{elapsed:.6f}", str(rss)] for index, (elapsed, rss) in enumerate(warm))
    existing = []
    if args.out.exists():
        with args.out.open(newline="") as stream:
            existing = list(csv.reader(stream))
        if existing and existing[0] != HEADER: raise SystemExit(f"unexpected results header in {args.out}")
        existing = [row for row in existing[1:] if not (row and row[0] == dataset and len(row) > 2 and row[2] == "write")]
    args.out.parent.mkdir(parents=True, exist_ok=True)
    with args.out.open("w", newline="") as stream:
        writer = csv.writer(stream); writer.writerow(HEADER); writer.writerows(existing); writer.writerows(aggregates)
    existing_raw = []
    if args.raw_out.exists():
        with args.raw_out.open(newline="") as stream: existing_raw = list(csv.reader(stream))
        if existing_raw and existing_raw[0] != SAMPLES_HEADER: raise SystemExit(f"unexpected raw-sample header in {args.raw_out}")
        existing_raw = [row for row in existing_raw[1:] if not (row and row[0] == dataset)]
    with args.raw_out.open("w", newline="") as stream:
        writer = csv.writer(stream); writer.writerow(SAMPLES_HEADER); writer.writerows(existing_raw); writer.writerows(raw)

if __name__ == "__main__": main()
