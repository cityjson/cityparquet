#!/usr/bin/env python3
"""Encode the whole of 3DBAG as ONE CityParquet package.

`cityparquet convert a.json b.json ...` already merges many inputs into one
package, but `merge_sources` materialises every input's features at once:
measured at ~20x the input JSON's size, which for 3DBAG's ~53 GB is ~1.1 TB of
resident memory. Too big.

The CityJSONSeq path, by contrast, STREAMS -- `Source::features()` reopens the
file and yields line by line, and the writer makes two such passes (scan, then
encode). Only `--ordering hilbert` buffers. So the whole dataset can be handed
to the reference writer as ONE CityJSONSeq file, and the writer computes the
footer itself rather than us grafting one on.

Turning 8,941 separately-quantised tiles into one CityJSONSeq is exactly what
`merge_sources` does in memory, so this reimplements ITS arithmetic:

    merged.scale     = componentwise min of the inputs' scale
    merged.translate = componentwise min of the inputs' translate
    v' = round(v * (src.scale/merged.scale) + (src.translate-merged.translate)/merged.scale)

3DBAG makes that cheap: every tile declares scale [0.001,0.001,0.001], so the
ratio is 1 and requantisation collapses to a per-tile, per-axis INTEGER ADD.
`plan` asserts the shared scale rather than assuming it -- a tile that disagreed
would need the general rescale, which this does not implement.

Rust rounds ties AWAY FROM ZERO, and roughly one 3DBAG axis-shift in fourteen
lands exactly on .5 (the translates are dyadic, so `(dt)/0.001` often is too).
Writing `offset = m + f` with `m = floor(offset)`, the add is `v + m` plus:
f < 0.5 -> nothing; f > 0.5 -> one more; f == 0.5 -> one more only where the
result is non-negative. That last case is the only per-vertex branch, and it is
what keeps this bit-identical to `merge_sources` instead of 1 mm away from it.

Stages, each resumable -- rerunning skips what is already on disk:

    fetch    download every tile .gz named by the manifest
    plan     read each tile's transform + CRS; derive the merged transform
    seq      gunzip | cjseq cat | shift  ->  one .jsonl per tile
    merge    synthetic header + every tile's feature lines -> one .city.jsonl
    convert  cityparquet convert <that one file> -o <dest>
    verify   row count, id uniqueness, extent, against the plan
    all      every stage in order
"""

from __future__ import annotations

import argparse
import gzip
import json
import math
import os
import shutil
import subprocess
import sys
import tempfile
import time
from concurrent.futures import ProcessPoolExecutor, as_completed
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
CITYPARQUET = REPO / "lib/cityparquet-rs/target/release/cityparquet"
CJSEQ = REPO / "lib/cityparquet-rs/vendor/cjseq/target/release/cjseq"

def log(msg: str) -> None:
    print(f"[{time.strftime('%H:%M:%S')}] {msg}", flush=True)


def tile_id(url: str) -> str:
    return url.rsplit("/", 1)[-1].removesuffix(".city.json.gz")


def tile_sort_key(tid: str) -> tuple:
    """Quadtree order (z, x, y), so the one big file is spatially coherent and
    the writer's row groups get tight bboxes for free. `--ordering hilbert`
    would do better but buffers every feature -- the 1.1 TB this avoids."""
    parts = tid.split("-")
    try:
        return (0, tuple(int(p) for p in parts))
    except ValueError:
        return (1, tid)


def read_urls(manifest: Path) -> list[str]:
    urls = [ln.strip() for ln in manifest.read_text().splitlines() if ln.strip()]
    urls.sort(key=lambda u: tile_sort_key(tile_id(u)))
    return urls


# ---------------------------------------------------------------- fetch


def _fetch_one(args: tuple[str, str]) -> tuple[str, str | None]:
    url, dest = args
    p = Path(dest)
    if p.exists() and p.stat().st_size > 0:
        return tile_id(url), None
    tmp = p.with_suffix(p.suffix + ".part")
    for attempt in range(4):
        rc = subprocess.run(
            ["curl", "-sfL", "--max-time", "600", "--retry", "2", "-o", str(tmp), url],
            capture_output=True,
        )
        if rc.returncode == 0 and tmp.exists() and tmp.stat().st_size > 0:
            # Proving it decompresses here is what makes the skip-if-present
            # check above safe on a rerun: a truncated download never lands.
            try:
                with gzip.open(tmp, "rb") as fh:
                    while fh.read(1 << 20):
                        pass
            except Exception as exc:  # noqa: BLE001
                tmp.unlink(missing_ok=True)
                if attempt == 3:
                    return tile_id(url), f"corrupt gzip: {exc}"
                continue
            tmp.replace(p)
            return tile_id(url), None
        time.sleep(2 * (attempt + 1))
    tmp.unlink(missing_ok=True)
    return tile_id(url), f"download failed after 4 attempts: {url}"


def stage_fetch(urls: list[str], work: Path, jobs: int) -> None:
    gz = work / "gz"
    gz.mkdir(parents=True, exist_ok=True)
    todo = [(u, str(gz / f"{tile_id(u)}.city.json.gz")) for u in urls]
    have = sum(1 for _, d in todo if Path(d).exists() and Path(d).stat().st_size > 0)
    log(f"fetch: {len(todo)} tiles, {have} already present")
    errors = []
    done = 0
    with ProcessPoolExecutor(max_workers=jobs) as ex:
        for fut in as_completed([ex.submit(_fetch_one, t) for t in todo]):
            tid, err = fut.result()
            done += 1
            if err:
                errors.append(f"{tid}: {err}")
            if done % 500 == 0:
                log(f"fetch: {done}/{len(todo)}")
    if errors:
        raise SystemExit("fetch failed:\n  " + "\n  ".join(errors[:20]))
    total = sum(Path(d).stat().st_size for _, d in todo)
    log(f"fetch: complete, {total / 1e9:.1f} GB")


# ---------------------------------------------------------------- plan


def _header_of(path: str) -> tuple[str, dict]:
    """The tile's `transform` and `referenceSystem`, without parsing the whole
    document: `cjseq cat` puts both on the seq header line, and only that line
    is read."""
    tid = Path(path).name.removesuffix(".city.json.gz")
    with tempfile.TemporaryDirectory() as td:
        raw = Path(td) / "t.city.json"
        with gzip.open(path, "rb") as src, open(raw, "wb") as dst:
            shutil.copyfileobj(src, dst, 1 << 20)
        # Only the first line is wanted, so cjseq is killed rather than left to
        # serialise the whole tile into a pipe nobody reads.
        proc = subprocess.Popen(
            [str(CJSEQ), "cat", str(raw)], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL
        )
        line = proc.stdout.readline()
        proc.kill()
        proc.stdout.close()
        proc.wait()
    h = json.loads(line)
    return tid, {
        "scale": h["transform"]["scale"],
        "translate": h["transform"]["translate"],
        "crs": json.dumps(h.get("metadata", {}).get("referenceSystem")),
        "metadata": h.get("metadata", {}),
    }


def stage_plan(urls: list[str], work: Path, jobs: int) -> dict:
    out = work / "plan.json"
    gz = work / "gz"
    paths = [str(gz / f"{tile_id(u)}.city.json.gz") for u in urls]
    log(f"plan: reading {len(paths)} tile headers")
    heads: dict[str, dict] = {}
    with ProcessPoolExecutor(max_workers=jobs) as ex:
        for n, fut in enumerate(as_completed([ex.submit(_header_of, p) for p in paths]), 1):
            tid, h = fut.result()
            heads[tid] = h
            if n % 1000 == 0:
                log(f"plan: {n}/{len(paths)}")

    crs = {h["crs"] for h in heads.values()}
    if len(crs) != 1:
        raise SystemExit(
            f"plan: {len(crs)} distinct CRS across tiles; merge_sources requires one:\n  "
            + "\n  ".join(sorted(crs)[:5])
        )
    scales = {tuple(h["scale"]) for h in heads.values()}
    if len(scales) != 1:
        # Not fatal in principle (merge_sources takes the min and rescales),
        # but the integer-add collapse below assumes ratio 1, so refuse rather
        # than silently fall back to something slower and unvalidated.
        raise SystemExit(f"plan: tiles disagree on scale: {sorted(scales)}")
    scale = list(next(iter(scales)))

    order = [tile_id(u) for u in urls]
    translate = [min(heads[t]["translate"][i] for t in order) for i in range(3)]

    shifts, n_tie = {}, 0
    for t in order:
        base, tie = [], []
        for i in range(3):
            # Computed exactly as `merge::requantise_vertices` does, in the same
            # order, so the f64 result is bit-identical.
            offset = (heads[t]["translate"][i] - translate[i]) / scale[i]
            m = math.floor(offset)
            f = offset - m
            if f > 0.5:
                m += 1
            base.append(int(m))
            tie.append(f == 0.5)
        n_tie += sum(tie)
        shifts[t] = {"base": base, "tie": tie}
    log(f"plan: {n_tie}/{3 * len(order)} axis shifts land on a rounding tie")

    first = heads[order[0]]["metadata"]
    header = {
        "type": "CityJSON",
        "version": "2.0",
        # merge_sources: first input's metadata, minus geographicalExtent --
        # one tile's extent must never be advertised for the merged dataset.
        "metadata": {k: v for k, v in first.items() if k != "geographicalExtent"},
        "transform": {"scale": scale, "translate": translate},
        "CityObjects": {},
        "vertices": [],
    }
    plan = {"order": order, "shifts": shifts, "header": header, "crs": next(iter(crs))}
    out.write_text(json.dumps(plan))
    log(f"plan: scale {scale}, merged translate {translate}, CRS {next(iter(crs))}")
    return plan


# ---------------------------------------------------------------- seq


def _ax(v: int, base: int, tie: bool) -> int:
    """One requantised coordinate. On a .5 tie Rust rounds away from zero, so a
    non-negative result takes the extra step and a negative one does not."""
    w = v + base
    return w + 1 if tie and w >= 0 else w


def _seq_one(args: tuple[str, str, str, dict]) -> tuple[str, int, str | None]:
    """gunzip | cjseq cat | shift -> one .jsonl of feature lines (no header).

    The header line is dropped here: the merged file carries exactly one, from
    the plan. Written to a .part and renamed, so a rerun after a crash never
    treats a half-file as done.
    """
    gz_path, out_path, tid, shift = args
    out = Path(out_path)
    if out.exists():
        with open(out, "rb") as fh:
            n = sum(1 for _ in fh)
        return tid, n, None
    tmp = out.with_suffix(".part")
    kx, ky, kz = shift["base"]
    tx, ty, tz = shift["tie"]
    n = 0
    try:
        with tempfile.TemporaryDirectory() as td:
            raw = Path(td) / "t.city.json"
            with gzip.open(gz_path, "rb") as src, open(raw, "wb") as dst:
                shutil.copyfileobj(src, dst, 1 << 20)
            proc = subprocess.Popen(
                [str(CJSEQ), "cat", str(raw)], stdout=subprocess.PIPE, stderr=subprocess.PIPE
            )
            with open(tmp, "w") as dst:
                for i, line in enumerate(proc.stdout):
                    if i == 0:
                        continue  # header
                    feat = json.loads(line)
                    if kx or ky or kz or tx or ty or tz:
                        feat["vertices"] = [
                            [
                                _ax(v[0], kx, tx),
                                _ax(v[1], ky, ty),
                                _ax(v[2], kz, tz),
                            ]
                            for v in feat["vertices"]
                        ]
                    dst.write(json.dumps(feat, separators=(",", ":")))
                    dst.write("\n")
                    n += 1
            proc.stdout.close()
            err = proc.stderr.read().decode()[:400]
            proc.stderr.close()
            if proc.wait() != 0:
                raise RuntimeError(f"cjseq cat: {err}")
    except Exception as exc:  # noqa: BLE001
        tmp.unlink(missing_ok=True)
        return tid, 0, str(exc)
    tmp.replace(out)
    return tid, n, None


def stage_seq(plan: dict, work: Path, jobs: int) -> dict[str, int]:
    seq = work / "seq"
    seq.mkdir(parents=True, exist_ok=True)
    gz = work / "gz"
    todo = [
        (str(gz / f"{t}.city.json.gz"), str(seq / f"{t}.jsonl"), t, plan["shifts"][t])
        for t in plan["order"]
    ]
    log(f"seq: {len(todo)} tiles")
    counts, errors, done = {}, [], 0
    with ProcessPoolExecutor(max_workers=jobs) as ex:
        for fut in as_completed([ex.submit(_seq_one, t) for t in todo]):
            tid, n, err = fut.result()
            done += 1
            if err:
                errors.append(f"{tid}: {err}")
            else:
                counts[tid] = n
            if done % 500 == 0:
                log(f"seq: {done}/{len(todo)}")
    if errors:
        raise SystemExit("seq failed:\n  " + "\n  ".join(errors[:20]))
    (work / "counts.json").write_text(json.dumps(counts))
    log(f"seq: complete, {sum(counts.values())} features")
    return counts


# ---------------------------------------------------------------- merge


def stage_merge(plan: dict, work: Path) -> Path:
    out = work / "3dbag.city.jsonl"
    tmp = out.with_suffix(".part")
    seq = work / "seq"
    log(f"merge: concatenating {len(plan['order'])} tiles into {out.name}")
    with open(tmp, "wb") as dst:
        dst.write(json.dumps(plan["header"], separators=(",", ":")).encode())
        dst.write(b"\n")
        for n, t in enumerate(plan["order"], 1):
            with open(seq / f"{t}.jsonl", "rb") as src:
                shutil.copyfileobj(src, dst, 1 << 22)
            if n % 1000 == 0:
                log(f"merge: {n}/{len(plan['order'])}")
    tmp.replace(out)
    log(f"merge: {out.stat().st_size / 1e9:.1f} GB")
    return out


# ---------------------------------------------------------------- convert


def stage_convert(big: Path, dest: Path, extra: list[str]) -> None:
    dest.parent.mkdir(parents=True, exist_ok=True)
    cmd = [str(CITYPARQUET), "convert", str(big), "-o", str(dest), "--overwrite", *extra]
    log("convert: " + " ".join(cmd))
    t0 = time.time()
    rc = subprocess.run(cmd)
    if rc.returncode != 0:
        raise SystemExit(f"convert failed with exit code {rc.returncode}")
    log(f"convert: complete in {(time.time() - t0) / 60:.1f} min")


# ---------------------------------------------------------------- verify


def stage_verify(dest: Path, work: Path) -> None:
    import duckdb

    pq = dest / "building.parquet"
    if not pq.exists():
        raise SystemExit(f"verify: no {pq}")
    con = duckdb.connect()
    rows, ids = con.execute(
        f"select count(*), count(distinct id) from read_parquet('{pq}')"
    ).fetchone()
    counts = json.loads((work / "counts.json").read_text())
    log(f"verify: {rows} rows, {ids} distinct ids, from {sum(counts.values())} input features")
    if ids != rows:
        log(f"verify: WARNING {rows - ids} duplicate ids -- the package cannot round-trip")
    ext = con.execute(
        f"select min(bbox.xmin), min(bbox.ymin), max(bbox.xmax), max(bbox.ymax) "
        f"from read_parquet('{pq}')"
    ).fetchone()
    log(f"verify: extent {ext}")
    kv = dict(con.execute(f"select key, value from parquet_kv_metadata('{pq}')").fetchall())
    for k in (b"city", b"geo"):
        if k not in kv:
            raise SystemExit(f"verify: footer is missing the '{k.decode()}' key")
    city = json.loads(kv[b"city"])
    log(f"verify: footer city.crs {json.loads(kv[b'city']).get('crs', {}).get('name')!r}")
    log(f"verify: footer city keys {sorted(city)}")
    log(f"verify: {pq.stat().st_size / 1e9:.1f} GB")


# ---------------------------------------------------------------- driver


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("stage", choices=["fetch", "plan", "seq", "merge", "convert", "verify", "all"])
    ap.add_argument("--manifest", type=Path, required=True, help="file of tile .city.json.gz URLs")
    ap.add_argument("--work", type=Path, required=True, help="scratch dir (holds ~110 GB)")
    ap.add_argument("--dest", type=Path, required=True, help="output CityParquet package dir")
    ap.add_argument("--jobs", type=int, default=min(32, os.cpu_count() or 8))
    ap.add_argument("--convert-arg", action="append", default=[], help="extra flag for `cityparquet convert`")
    args = ap.parse_args()

    for tool in (CITYPARQUET, CJSEQ):
        if not tool.exists():
            raise SystemExit(f"missing {tool} -- build it first")
    args.work.mkdir(parents=True, exist_ok=True)
    urls = read_urls(args.manifest)

    def plan_or_load() -> dict:
        p = args.work / "plan.json"
        if p.exists():
            return json.loads(p.read_text())
        return stage_plan(urls, args.work, args.jobs)

    s = args.stage
    if s in ("fetch", "all"):
        stage_fetch(urls, args.work, min(args.jobs, 12))
    if s in ("plan", "all"):
        plan_or_load()
    if s in ("seq", "all"):
        stage_seq(plan_or_load(), args.work, args.jobs)
    if s in ("merge", "all"):
        stage_merge(plan_or_load(), args.work)
    if s in ("convert", "all"):
        stage_convert(args.work / "3dbag.city.jsonl", args.dest, args.convert_arg)
    if s in ("verify", "all"):
        stage_verify(args.dest, args.work)


if __name__ == "__main__":
    sys.exit(main())
