#!/usr/bin/env python3
"""Measure format serialisation as isolated child processes.

Each sample creates a fresh artefact directory, so the idempotent preparation
script cannot turn a timed conversion into a cache hit. The raw samples are
kept beside the aggregate read CSVs; the aggregate uses median time and maximum
RSS, matching the readbench coordinator convention.
"""
from __future__ import annotations
import argparse, csv, os, shutil, statistics, subprocess, tempfile, time
from pathlib import Path

FORMATS = ("citygml", "cityjson", "cityjsonseq", "flatcitybuf", "cityparquet-hilbert")
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


def run_child(command: list[str], stdout: Path | None = None) -> tuple[float, int]:
    """Run one converter and return wall time plus its own peak RSS."""
    stream = stdout.open("wb") if stdout is not None else None
    try:
        started = time.perf_counter()
        process = subprocess.Popen(command, stdout=stream)
        _, status, usage = os.wait4(process.pid, 0)
        elapsed = time.perf_counter() - started
    finally:
        if stream is not None:
            stream.close()
    status = os.waitstatus_to_exitcode(status)
    process.returncode = status
    if status != 0:
        raise subprocess.CalledProcessError(status, command)
    return elapsed, usage.ru_maxrss * 1024


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--canonical-seq", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--raw-out", type=Path, required=True)
    parser.add_argument("--scratch", type=Path, required=True)
    parser.add_argument("--repeat", type=int, default=3)
    args = parser.parse_args()
    if args.repeat < 1: raise SystemExit("--repeat must be >= 1")
    args.scratch.mkdir(parents=True, exist_ok=True)
    dataset = base(args.input)
    # `/usr/bin/time -f` is used directly here so elapsed seconds and RSS share
    # the exact same fresh conversion process.
    aggregates: list[list[str]] = []
    raw: list[list[str]] = []
    for fmt in FORMATS:
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
                stages = [(["cat", str(args.canonical_seq)], target)]
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
        aggregates.append([dataset, fmt, "write", "", "0", f"{centre:.6f}", f"{mad(times, centre):.6f}", "", str(max(value[1] for value in warm)), str(args.repeat), "canonical-cityjsonseq;citygml=seq-to-json+json-to-gml", "", ""])
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
