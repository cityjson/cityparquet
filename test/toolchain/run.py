"""The tool-chain test: every CityParquet writer and reader against real data.

    uv run --python 3.12 --with duckdb==1.5.5 --with flatcitybuf==0.5.2 \\
        test/toolchain/run.py fetch  [--profile quick]
    uv run ... test/toolchain/run.py run    [--profile quick] [--only ID,...] [--keep]
    uv run ... test/toolchain/run.py report RUN_DIR
    uv run ... test/toolchain/run.py citydb [--only ID,...] [--keep]

`fetch` downloads the corpus (corpus.toml) and checks every byte. `run`
pushes each dataset through every tool chain, compares everything that comes
back with the source, and writes `results.jsonl` and `report.md` into a new
run directory. `report` rewrites the report from a run's results. `citydb`
loads sources into 3DCityDB v5 and reads them back (citydb.py).

The tools come from the environment, each with a default in this tree:

    CITYPARQUET          lib/cityparquet-rs/target/release/cityparquet
    CITYJSON_EXTENSION   lib/duckdb-cityjson/build/release/extension/cityjson/cityjson.duckdb_extension
    THREE_D_EXTENSION    lib/duckdb-3d/build/release/extension/three_d/three_d.duckdb_extension
    CITYGML_TOOLS        benchmark/formats/tools/citygml-tools/citygml-tools
    TOOLCHAIN_WORK       test/runs/toolchain

A tool that is missing skips the steps that need it, and the report says so.
"""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
import platform
import re
import shutil
import subprocess
import sys
import time
import tomllib
import xml.etree.ElementTree as ET
from collections import Counter
from dataclasses import dataclass
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
PROFILES = {"quick": {"quick"}, "full": {"quick", "full"}, "xl": {"quick", "full", "xl"}}
EXT = {"cityjson": ".city.json", "cityjsonseq": ".city.jsonl", "fcb": ".fcb", "citygml2": ".gml", "citygml3": ".gml"}
DUCK_FORMAT = {"cityjson": "cityjson", "cityjsonseq": "cityjsonseq", "fcb": "flatcitybuf"}
STEP_TIMEOUT = int(os.environ.get("TOOLCHAIN_STEP_TIMEOUT", "3600"))


def env_path(name: str, default: str) -> Path:
    return Path(os.environ.get(name) or ROOT / default)


@dataclass
class Tools:
    cityparquet: Path
    cityjson_ext: Path
    three_d_ext: Path
    citygml_tools: Path

    @classmethod
    def from_env(cls) -> "Tools":
        return cls(
            env_path("CITYPARQUET", "lib/cityparquet-rs/target/release/cityparquet"),
            env_path("CITYJSON_EXTENSION", "lib/duckdb-cityjson/build/release/extension/cityjson/cityjson.duckdb_extension"),
            env_path("THREE_D_EXTENSION", "lib/duckdb-3d/build/release/extension/three_d/three_d.duckdb_extension"),
            env_path("CITYGML_TOOLS", "benchmark/formats/tools/citygml-tools/citygml-tools"),
        )

    def missing(self) -> dict[str, str]:
        out = {}
        for k, p in vars(self).items():
            if not p.exists():
                out[k] = str(p)
        return out


def load_corpus() -> list[dict]:
    with open(HERE / "corpus.toml", "rb") as f:
        return tomllib.load(f)["dataset"]


def load_citydb() -> list[dict]:
    with open(HERE / "corpus.toml", "rb") as f:
        return tomllib.load(f).get("citydb", [])


def load_known() -> list[dict]:
    p = HERE / "known.toml"
    if not p.exists():
        return []
    with open(p, "rb") as f:
        return tomllib.load(f).get("issue", [])


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def work_dir() -> Path:
    return env_path("TOOLCHAIN_WORK", "test/runs/toolchain")


# --------------------------------------------------------------------------
# fetch
# --------------------------------------------------------------------------


def _fix_integer_vertices(src: Path, dest: Path) -> None:
    """CityJSON vertices are integers (the `transform` applies to them); a
    fixture that writes 0.0 for 0 gets the same values as integers."""
    doc = json.loads(src.read_text(encoding="utf-8"))
    for v in doc["vertices"]:
        for i, x in enumerate(v):
            if float(x) != int(x):
                raise ValueError(f"vertex {v} is not integral")
            v[i] = int(x)
    dest.write_text(json.dumps(doc, separators=(",", ":")), encoding="utf-8")


def _fix_address_location_lod(src: Path, dest: Path) -> None:
    """An address `location` is a MultiPoint Geometry Object, which needs a
    `lod`; one without gets "1", as the CityJSON specification's example."""
    out = []
    for line in src.read_text(encoding="utf-8").splitlines():
        f = json.loads(line)
        for co in (f.get("CityObjects") or {}).values():
            for a in co.get("address") or []:
                if "location" in a:
                    a["location"].setdefault("lod", "1")
        out.append(json.dumps(f, separators=(",", ":")))
    dest.write_text("\n".join(out) + "\n", encoding="utf-8")


def _fix_python_bytes_literal(src: Path, dest: Path) -> None:
    """A document published as the repr of a Python bytes object
    (`b'<?xml …'`) instead of the bytes themselves: its literal, evaluated."""
    import ast

    dest.write_bytes(ast.literal_eval(src.read_text(encoding="ascii")))


FIXES = {
    "integer-vertices": _fix_integer_vertices,
    "address-location-lod": _fix_address_location_lod,
    "python-bytes-literal": _fix_python_bytes_literal,
}


def fetch(profile: str) -> int:
    corpus = work_dir() / "corpus"
    corpus.mkdir(parents=True, exist_ok=True)
    bad = 0
    for d in load_corpus():
        if d["profile"] not in PROFILES[profile]:
            continue
        dest = corpus / d["file"]
        if dest.exists():
            if dest.stat().st_size == d["bytes"] and sha256(dest) == d["sha256"]:
                print(f"ok      {d['id']}")
                continue
            # Never fetch again: the published bytes changed, or the file is
            # damaged, and either has to be looked at by a person.
            print(f"MISMATCH {d['id']}: {dest} is not the pinned file (delete it to fetch again)")
            bad += 1
            continue
        tmp = dest.with_suffix(dest.suffix + ".part")
        cmd = ["curl", "-sSfL", "--retry", "3", "--retry-all-errors", "-o", str(tmp), d["url"]]
        derive = d.get("derive", "")
        if derive.startswith("fix:"):
            # A published fixture that is not valid CityJSON, repaired by a
            # named, value-preserving step: the original is pinned as well.
            print(f"fetch   {d['id']} ({d['source_bytes']:,} B, then {derive})")
            if subprocess.run(cmd).returncode != 0:
                print(f"FAILED  {d['id']}: {d['url']}")
                bad += 1
                continue
            if tmp.stat().st_size != d["source_bytes"] or sha256(tmp) != d["source_sha256"]:
                print(f"MISMATCH {d['id']}: downloaded bytes are not the pinned original; kept as {tmp}")
                bad += 1
                continue
            FIXES[derive[4:]](tmp, dest)
            tmp.unlink()
            if dest.stat().st_size != d["bytes"] or sha256(dest) != d["sha256"]:
                print(f"MISMATCH {d['id']}: the repaired file is not the pinned one; kept as {dest}")
                bad += 1
            continue
        if derive.startswith("head:"):
            # The first N lines of a CityJSONSeq: fetched as the byte range
            # those lines occupy, so the whole file is never downloaded.
            cmd[1:1] = ["-r", f"0-{d['bytes'] - 1}"]
        print(f"fetch   {d['id']} ({d['bytes']:,} B)")
        if subprocess.run(cmd).returncode != 0:
            print(f"FAILED  {d['id']}: {d['url']}")
            bad += 1
            continue
        if tmp.stat().st_size != d["bytes"] or sha256(tmp) != d["sha256"]:
            print(f"MISMATCH {d['id']}: downloaded bytes are not the pinned ones; kept as {tmp}")
            bad += 1
            continue
        tmp.rename(dest)
    return 1 if bad else 0


# --------------------------------------------------------------------------
# steps
# --------------------------------------------------------------------------


class Run:
    def __init__(self, out: Path, tools: Tools, python: str) -> None:
        self.out = out
        self.tools = tools
        self.python = python
        self.missing = tools.missing()
        self.known = load_known()
        self.results = open(out / "results.jsonl", "a", encoding="utf-8")
        self.env = dict(os.environ, CITYJSON_EXTENSION=str(tools.cityjson_ext), THREE_D_EXTENSION=str(tools.three_d_ext))

    def record(self, ds: dict, step: str, status: str, detail: str = "", failures: list[str] | None = None, **extra) -> dict:
        if status == "fail":
            # Known only when every failure line is explained: one known issue
            # must not hide an unexplained failure in the same step. `failures`
            # is every line when the detail shows only a sample of them.
            lines = failures or [l for l in detail.splitlines() if l.startswith("fail:")] or [detail]
            ids = []
            for line in lines:
                hit = next(
                    (
                        k["id"]
                        for k in self.known
                        if re.search(k.get("dataset", ".*"), ds["id"])
                        and re.search(k.get("step", ".*"), step)
                        and re.search(k["match"], line, re.S)
                    ),
                    None,
                )
                if hit is None:
                    break
                ids.append(hit)
            else:
                status = "known"
                extra["known"] = ",".join(dict.fromkeys(ids))
        rec = {"dataset": ds["id"], "format": ds["format"], "step": step, "status": status, "detail": detail[:4000], **extra}
        self.results.write(json.dumps(rec, default=str) + "\n")
        self.results.flush()
        mark = {"pass": "ok  ", "fail": "FAIL", "known": "KNWN", "skip": "skip", "admitted": "ok~ ", "noted": "ok* "}.get(status, status)
        print(f"  {mark} {step}" + (f": {detail.splitlines()[0][:160]}" if detail and status != "pass" else ""), flush=True)
        return rec

    def sh(self, args: list, timeout: int = STEP_TIMEOUT) -> tuple[int, str, float]:
        t0 = time.time()
        try:
            r = subprocess.run([str(a) for a in args], capture_output=True, text=True, timeout=timeout, env=self.env)
            return r.returncode, (r.stdout + r.stderr).strip(), time.time() - t0
        except subprocess.TimeoutExpired:
            return 124, f"timed out after {timeout} s", time.time() - t0

    def duck(self, *args) -> tuple[dict, float]:
        rc, out, secs = self.sh([self.python, HERE / "duck.py", *args])
        try:
            line = [l for l in out.splitlines() if l.startswith("{")][-1]
            res = json.loads(line)
        except (IndexError, ValueError):
            res = {"ok": False, "error": f"exit {rc}: {out[-2000:]}"}
        return res, secs

    # -- the comparisons ---------------------------------------------------

    def compare(
        self,
        ds: dict,
        step: str,
        source: Path,
        output: Path,
        tol: str | None = None,
        pair: bool = False,
        uv_tol: str | None = None,
        any_face_order: bool = False,
    ) -> dict:
        if not output.exists():
            return self.record(ds, step, "skip", f"{output.name} was not written")
        rep_path = output.with_suffix(output.suffix + ".compare.json")
        args = [self.python, HERE / "cjcompare.py", source, output, "--json", rep_path]
        if tol:
            args += ["--tol", tol]
        if uv_tol:
            args += ["--uv-tol", uv_tol]
        if pair:
            args.append("--pair-unmatched")
        if any_face_order:
            args.append("--any-face-order")
        rc, out, secs = self.sh(args)
        if rc == 2 or not rep_path.exists():
            return self.record(ds, step, "fail", f"comparator error: {out[-1500:]}", seconds=secs)
        rep = json.loads(rep_path.read_text())
        cats = {c: v["status"] for c, v in rep["categories"].items()}
        status = "pass" if rep["ok"] else "fail"
        if status == "pass" and any(s == "admitted" for s in cats.values()):
            status = "admitted"
        fails = {k: v for k, v in rep["examples"].items() if k.startswith("fail:")}
        detail = "\n".join(f"{k}: {e}" for k, ex in fails.items() for e in ex[:3])
        return self.record(
            ds,
            step,
            status,
            detail,
            failures=rep["failures"],
            seconds=secs,
            categories=cats,
            objects=[rep["objects_source"], rep["objects_output"]],
            max_deviation=rep["max_coordinate_deviation"],
            max_uv_deviation=rep["max_uv_deviation"],
            examples=rep["examples"],
        )

    def rs_compare(self, ds: dict, step: str, source: Path, output: Path) -> dict:
        if not output.exists():
            return self.record(ds, step, "skip", f"{output.name} was not written")
        rc, out, secs = self.sh([self.tools.cityparquet, "compare", "--all", source, output])
        if rc == 0:
            return self.record(ds, step, "pass", "", seconds=secs)
        # One `fail:` line per difference, each cut short, so that known.toml
        # can account for every one of them; the detail shows the first 40.
        lines = [f"fail:{l[:300]}" for l in out.splitlines() if l.strip()]
        detail = "\n".join(lines[:40]) + (f"\n({len(lines) - 40} more)" if len(lines) > 40 else "")
        return self.record(ds, step, "fail", detail, failures=lines, seconds=secs)

    def gml_to_cityjson(self, gml: Path, outdir: Path) -> Path | None:
        outdir.mkdir(parents=True, exist_ok=True)
        rc, _, _ = self.sh(
            [
                self.tools.citygml_tools,
                "to-cityjson",
                "--vertex-precision=6",
                "--template-precision=9",
                "--texture-vertex-precision=9",
                "-o",
                outdir,
                gml,
            ]
        )
        produced = sorted(outdir.glob("*.json"))
        return produced[0] if rc == 0 and produced else None


CITY_OBJECT_CLASSES = {
    "Building", "BuildingPart", "BuildingInstallation", "IntBuildingInstallation", "Room",
    "BuildingFurniture", "Bridge", "BridgePart", "BridgeInstallation", "BridgeConstructionElement",
    "Tunnel", "TunnelPart", "TunnelInstallation", "Road", "Railway", "Track", "Square",
    "WaterBody", "PlantCover", "SolitaryVegetationObject", "CityFurniture", "GenericCityObject",
    "CityObjectGroup", "TINRelief", "ReliefFeature", "LandUse",
}
SEMANTIC_SURFACES = {
    "RoofSurface", "WallSurface", "GroundSurface", "ClosureSurface", "FloorSurface",
    "OuterFloorSurface", "InteriorWallSurface", "CeilingSurface", "OuterCeilingSurface",
    "WaterSurface", "WaterGroundSurface", "WaterClosureSurface", "Door", "Window",
}


def gml_census(path: Path) -> Counter:
    """Counts what a CityGML document holds, by element: city objects by
    class, semantic surfaces and openings by type, appearance elements,
    implicit geometries and addresses. Comparing a source's census with a
    written document's sees what citygml-tools would drop from both sides
    alike. Polygons are not counted: a writer may inline a polygon the
    source shared by xlink, which changes the count and not the model."""
    c: Counter = Counter()
    for _, el in ET.iterparse(path, events=("end",)):
        tag = el.tag.rsplit("}", 1)[-1]
        if tag in CITY_OBJECT_CLASSES:
            c[f"object:{tag}"] += 1
        elif tag in SEMANTIC_SURFACES:
            c[f"surface:{tag}"] += 1
        elif tag in ("X3DMaterial", "ParameterizedTexture", "GeoreferencedTexture"):
            c[f"appearance:{tag}"] += 1
        elif tag in ("ImplicitGeometry", "Address"):
            c[tag] += 1
        el.clear()
    return c


def census_diff(a: Counter, b: Counter) -> list[str]:
    return [f"{k}: {a.get(k, 0)} -> {b.get(k, 0)}" for k in sorted(set(a) | set(b)) if a.get(k, 0) != b.get(k, 0)]


def run_dataset(r: Run, ds: dict, corpus: Path, keep: bool) -> None:
    src = corpus / ds["file"]
    fmt = ds["format"]
    d = r.out / ds["id"]
    d.mkdir(parents=True, exist_ok=True)
    print(f"== {ds['id']} ({fmt}, {ds['file']})", flush=True)
    cp = r.tools.cityparquet
    have_rs = "cityparquet" not in r.missing
    have_dk = "cityjson_ext" not in r.missing
    have_3d = have_dk and "three_d_ext" not in r.missing
    have_gt = "citygml_tools" not in r.missing
    is_gml = fmt in ("citygml2", "citygml3")
    out_ext = ".city.jsonl" if fmt == "fcb" else EXT[fmt]

    if not src.exists():
        r.record(ds, "corpus", "fail", f"{src} missing: run `run.py fetch` first")
        return

    # For CityGML, both the source and every written GML document are read
    # by citygml-tools, so that the comparator sees two CityJSON documents
    # produced the same way.
    reference = src
    if is_gml:
        if not have_gt:
            r.record(ds, "citygml-tools", "skip", "citygml-tools not found: CityGML comparisons skipped")
            reference = None
        else:
            reference = r.gml_to_cityjson(src, d / "cgt_source")
            if reference is None:
                r.record(ds, "citygml-tools source", "fail", "citygml-tools could not read the source")
    gml_tol = "0.001" if is_gml else None

    pkg_rs = d / "pkg_rs"
    rs_ok = False
    if have_rs:
        rc, out, secs = r.sh([cp, "convert", src, "-o", pkg_rs, "--overwrite", *ds.get("rs_convert_args", [])])
        rs_ok = rc == 0
        r.record(ds, "rs convert", "pass" if rs_ok else "fail", out[-1500:] if not rs_ok else out.splitlines()[-1] if out else "", seconds=secs)
    else:
        r.record(ds, "rs convert", "skip", "cityparquet binary not found")

    if rs_ok:
        rc, out, secs = r.sh([cp, "validate", pkg_rs])
        r.record(ds, "rs validate (rs package)", "pass" if rc == 0 else "fail", out[-1500:], seconds=secs)
        rs_out = d / f"rs_out{out_ext}"
        rc, out, secs = r.sh([cp, "export", pkg_rs, rs_out, *(["--citygml-version", "3.0"] if fmt == "citygml3" else [])])
        r.record(ds, "rs export (rs package)", "pass" if rc == 0 else "fail", out[-1500:], seconds=secs)
        if rc == 0:
            if is_gml:
                census = census_diff(gml_census(src), gml_census(rs_out))
                r.record(ds, "gml census source = rs export", "pass" if not census else "fail", "\n".join(census))
                if reference is not None:
                    back = r.gml_to_cityjson(rs_out, d / "cgt_rs_out")
                    if back is None:
                        r.record(ds, "round trip rs (gml)", "fail", "citygml-tools could not read the written CityGML")
                    else:
                        r.compare(ds, "round trip rs", reference, back, gml_tol, pair=True)
            else:
                r.compare(ds, "round trip rs", src, rs_out)
                r.rs_compare(ds, "rs compare (second oracle)", src, rs_out)
        if have_dk:
            res, secs = r.duck("validate", pkg_rs)
            r.record(ds, "dk validate (rs package)", "pass" if res.get("ok") and not res.get("errors") else "fail", json.dumps(res)[:1500], seconds=secs)
    elif have_rs:
        for s in ("rs validate (rs package)", "rs export (rs package)", "round trip rs"):
            r.record(ds, s, "skip", "rs convert failed")

    pkg_dk = d / "pkg_dk"
    dk_ok = False
    if have_dk and not is_gml:
        res, secs = r.duck("write", src, fmt, pkg_dk)
        dk_ok = bool(res.get("ok"))
        r.record(ds, "dk write", "pass" if dk_ok else "fail", res.get("error", ""), seconds=secs)
    elif is_gml:
        r.record(ds, "dk write", "skip", "duckdb-cityjson reads no CityGML")
    else:
        r.record(ds, "dk write", "skip", "cityjson extension not found")

    if dk_ok:
        res, secs = r.duck("validate", pkg_dk)
        r.record(ds, "dk validate (dk package)", "pass" if res.get("ok") and not res.get("errors") else "fail", json.dumps(res)[:1500], seconds=secs)
        if have_rs:
            rc, out, secs = r.sh([cp, "validate", pkg_dk])
            r.record(ds, "rs validate (dk package)", "pass" if rc == 0 else "fail", out[-1500:], seconds=secs)
        dk_out = d / f"dk_out{EXT[fmt]}"
        res, secs = r.duck("export", pkg_dk, dk_out, DUCK_FORMAT[fmt])
        r.record(ds, "dk export (dk package)", "pass" if res.get("ok") else "fail", res.get("error", ""), seconds=secs)
        r.compare(ds, "round trip dk", src, dk_out)
        if have_rs:
            rs_of_dk = d / f"rs_of_dk{out_ext}"
            rc, out, secs = r.sh([cp, "export", pkg_dk, rs_of_dk])
            r.record(ds, "rs export (dk package)", "pass" if rc == 0 else "fail", out[-1500:], seconds=secs)
            r.compare(ds, "cross dk package -> rs export", src, rs_of_dk)
            if rs_ok:
                # The same exporter over the two writers' packages: any
                # difference is between the packages, not the exporters.
                r.compare(ds, "packages agree (rs export of each)", d / f"rs_out{out_ext}", rs_of_dk)
                structure(r, ds, pkg_rs, pkg_dk)

    if rs_ok and have_dk:
        cross_fmt = "cityjsonseq" if is_gml else fmt
        dk_of_rs = d / f"dk_of_rs{EXT[cross_fmt]}"
        res, secs = r.duck("export", pkg_rs, dk_of_rs, DUCK_FORMAT[cross_fmt])
        r.record(ds, "dk export (rs package)", "pass" if res.get("ok") else "fail", res.get("error", ""), seconds=secs)
        if is_gml:
            rs_seq = d / "rs_out_seq.city.jsonl"
            rc, out, _ = r.sh([cp, "export", pkg_rs, rs_seq])
            if rc == 0:
                r.compare(ds, "exporters agree on the rs package (rs vs dk)", rs_seq, dk_of_rs)
        else:
            r.compare(ds, "cross rs package -> dk export", src, dk_of_rs)
        # DuckDB loads the rs package and writes it again: the rewritten
        # package must still export to the source.
        pkg_rw = d / "pkg_rs_rewritten"
        res, secs = r.duck("rewrite", pkg_rs, pkg_rw)
        r.record(ds, "dk rewrite (rs package)", "pass" if res.get("ok") else "fail", res.get("error", ""), seconds=secs)
        if res.get("ok") and have_rs:
            rw_out = d / f"rewritten{'.city.jsonl' if is_gml else out_ext}"
            rc, out, secs = r.sh([cp, "export", pkg_rw, rw_out])
            if rc != 0:
                r.record(ds, "rs export (rewritten package)", "fail", out[-1500:], seconds=secs)
            elif is_gml:
                r.compare(ds, "rewritten package = rs package", d / "rs_out_seq.city.jsonl", rw_out)
            else:
                r.compare(ds, "rewritten package round trip", src, rw_out)

    if have_3d:
        measures = {}
        for label, path, kind, made in (("rs package", pkg_rs, "package", rs_ok), ("dk package", pkg_dk, "package", dk_ok), ("source", src, fmt, True)):
            if not made:
                # The writer failed, and that step already says so.
                continue
            if kind in ("citygml2", "citygml3"):
                continue
            res, secs = r.duck("measure", path, kind, d / f"measure_{label.replace(' ', '_')}.parquet")
            if res.get("ok"):
                measures[label] = d / f"measure_{label.replace(' ', '_')}.parquet"
                r.record(ds, f"three_d reads {label}", "pass", json.dumps(res), seconds=secs, measure=res)
            else:
                r.record(ds, f"three_d reads {label}", "fail", res.get("error", ""), seconds=secs)
        base = measures.get("rs package")
        for other in ("dk package", "source"):
            if base and other in measures:
                agree(r, ds, f"three_d agrees: rs package vs {other}", base, measures[other])

    if not keep:
        failed = any(
            json.loads(l)["status"] == "fail"
            for l in (r.out / "results.jsonl").read_text().splitlines()
            if json.loads(l)["dataset"] == ds["id"]
        )
        if not failed:
            shutil.rmtree(d, ignore_errors=True)


def structure(r: Run, ds: dict, a: Path, b: Path) -> None:
    """The two writers' packages side by side: files, row counts, ids per
    table and column types. Ids and counts must agree; a column type that
    differs is reported, since the specification lets a writer pick a type
    for a mixed attribute."""
    import duckdb

    con = duckdb.connect()

    def info(p: Path) -> dict:
        out = {}
        for f in sorted(p.glob("*.parquet")):
            q = f"read_parquet('{f}')"
            cols = {n: t for n, t, *_ in con.execute(f"DESCRIBE SELECT * FROM {q}").fetchall()}
            n = con.execute(f"SELECT count(*) FROM {q}").fetchone()[0]
            ids = None
            if "id" in cols and "feature_id" in cols:
                ids = {x for (x,) in con.execute(f"SELECT id FROM {q}").fetchall()}
            out[f.name] = {"cols": cols, "rows": n, "ids": ids}
        return out

    ia, ib = info(a), info(b)
    fails, notes = [], []
    # Only a difference in the objects themselves fails here; the round trips
    # judge content. Column names, types and sidecar contents are each writer's
    # choice within the specification, and are listed for the report.
    for f in sorted(set(ia) ^ set(ib)):
        owner = "rs" if f in ia else "dk"
        (fails if (ia.get(f) or ib.get(f))["ids"] is not None else notes).append(f"{f} only in the {owner} package")
    for f in sorted(set(ia) & set(ib)):
        x, y = ia[f], ib[f]
        if x["ids"] is not None or y["ids"] is not None:
            if x["rows"] != y["rows"]:
                fails.append(f"{f}: {x['rows']} rows vs {y['rows']}")
            if x["ids"] != y["ids"]:
                fails.append(f"{f}: ids differ ({len((x['ids'] or set()) ^ (y['ids'] or set()))} not in both)")
        elif x["rows"] != y["rows"]:
            notes.append(f"{f}: {x['rows']} rows vs {y['rows']} (sidecar)")
        only_a = sorted(set(x["cols"]) - set(y["cols"]))
        only_b = sorted(set(y["cols"]) - set(x["cols"]))
        if only_a or only_b:
            notes.append(f"{f}: columns only in rs {only_a[:8]}, only in dk {only_b[:8]}")
        typed = [f"{c} {x['cols'][c]} vs {y['cols'][c]}" for c in sorted(set(x["cols"]) & set(y["cols"])) if x["cols"][c] != y["cols"][c]]
        if typed:
            notes.append(f"{f}: column types differ: {'; '.join(typed[:6])}")
    status = "fail" if fails else ("noted" if notes else "pass")
    r.record(ds, "packages agree (structure)", status, "\n".join(fails + notes))


def agree(r: Run, ds: dict, step: str, a: Path, b: Path) -> None:
    """duckdb-3d's view of two inputs, row by row: the same objects parse as
    solids, with the same validity, faces and shells, and the same volume,
    surface area and footprint to a relative 1e-6."""
    import duckdb

    con = duckdb.connect()
    q = f"""
    WITH a AS (SELECT * FROM read_parquet('{a}')), b AS (SELECT * FROM read_parquet('{b}'))
    SELECT coalesce(a.id, b.id) AS id, coalesce(a.col, b.col) AS col,
           a.id IS NULL AS only_b, b.id IS NULL AS only_a,
           a.parsed IS DISTINCT FROM b.parsed OR a.valid IS DISTINCT FROM b.valid
             OR a.faces IS DISTINCT FROM b.faces OR a.shells IS DISTINCT FROM b.shells AS topo,
           NOT (coalesce(abs(a.volume - b.volume) <= 1e-6 * greatest(abs(a.volume), 1), a.volume IS NULL AND b.volume IS NULL)
                AND coalesce(abs(a.area - b.area) <= 1e-6 * greatest(abs(a.area), 1), a.area IS NULL AND b.area IS NULL)
                AND coalesce(abs(a.footprint - b.footprint) <= 1e-6 * greatest(abs(a.footprint), 1), a.footprint IS NULL AND b.footprint IS NULL)) AS measure,
           a.parsed AS pa, b.parsed AS pb, a.valid AS va, b.valid AS vb, a.faces AS fa, b.faces AS fb, a.volume AS vola, b.volume AS volb
    FROM a FULL OUTER JOIN b ON a.id = b.id AND a.col = b.col
    """
    rows = con.execute(f"SELECT * FROM ({q}) WHERE only_a OR only_b OR topo OR measure").fetchall()
    total = con.execute(f"SELECT count(*), count(*) FILTER (WHERE pa) FROM ({q})").fetchone()
    if not rows:
        r.record(ds, step, "pass", f"{total[0]} geometries, {total[1]} parsed as solids", rows=total[0])
        return
    kinds = Counter("missing" if x[2] or x[3] else ("topology" if x[4] else "measure") for x in rows)
    ex = [f"{x[0]} {x[1]}: parsed {x[6]}/{x[7]} valid {x[8]}/{x[9]} faces {x[10]}/{x[11]} volume {x[12]}/{x[13]}" for x in rows[:6]]
    r.record(ds, step, "fail", f"{len(rows)} of {total[0]} geometries differ {dict(kinds)}\n" + "\n".join(ex), rows=total[0])


# --------------------------------------------------------------------------
# environment and report
# --------------------------------------------------------------------------


def environment(tools: Tools, python: str) -> dict:
    def git(*args, cwd=ROOT) -> str:
        try:
            return subprocess.run(["git", *args], cwd=cwd, capture_output=True, text=True).stdout.strip()
        except OSError:
            return ""

    env = {
        "date": dt.datetime.now(dt.timezone.utc).isoformat(timespec="seconds"),
        "cityparquet_commit": git("rev-parse", "--short", "HEAD"),
        "cityparquet_branch": git("rev-parse", "--abbrev-ref", "HEAD"),
        "cityparquet_dirty": bool(git("status", "--porcelain", "--untracked-files=no")),
        "machine": f"{platform.system()} {platform.machine()}, {os.cpu_count()} cores",
        "tools": {k: str(v) for k, v in vars(tools).items()},
        "missing": tools.missing(),
    }
    if tools.cityparquet.exists():
        # The CLI prints no version, so the binary is identified by its hash
        # and build time: the commit above is the tree's, not necessarily the
        # binary's.
        built = dt.datetime.fromtimestamp(tools.cityparquet.stat().st_mtime, dt.timezone.utc)
        env["cityparquet_version"] = f"sha256 {sha256(tools.cityparquet)[:12]}, built {built:%Y-%m-%d %H:%M} UTC"
    probe = (
        "import duckdb,os,json;c=duckdb.connect(config={'allow_unsigned_extensions':'true'});"
        "[c.execute(f\"LOAD '{p}'\") for p in (os.environ['CITYJSON_EXTENSION'],os.environ['THREE_D_EXTENSION']) if os.path.exists(p)];"
        "print(json.dumps({'duckdb':duckdb.__version__,'extensions':c.execute(\"select extension_name, extension_version from duckdb_extensions() where loaded and extension_name in ('cityjson','three_d')\").fetchall()}))"
    )
    r = subprocess.run(
        [python, "-c", probe],
        capture_output=True,
        text=True,
        env=dict(os.environ, CITYJSON_EXTENSION=str(tools.cityjson_ext), THREE_D_EXTENSION=str(tools.three_d_ext)),
    )
    try:
        env.update(json.loads(r.stdout))
    except ValueError:
        env["duckdb_probe_error"] = r.stderr[-500:]
    if tools.citygml_tools.exists():
        env["citygml_tools"] = tools.citygml_tools.resolve().parent.name
    return env


def run(profile: str, only: set[str] | None, keep: bool) -> int:
    tools = Tools.from_env()
    stamp = dt.datetime.now().strftime("%Y%m%d-%H%M%S")
    out = work_dir() / f"run-{stamp}"
    out.mkdir(parents=True)
    python = sys.executable
    env = environment(tools, python)
    env["profile"] = profile
    (out / "environment.json").write_text(json.dumps(env, indent=1))
    print(f"run directory {out}")
    r = Run(out, tools, python)
    corpus = work_dir() / "corpus"
    selected = []
    for ds in load_corpus():
        if ds["profile"] not in PROFILES[profile] or (only and ds["id"] not in only):
            continue
        selected.append(ds)
        if ds.get("status", "active") != "active":
            r.record(ds, "pending", "skip", ds.get("reason", ""))
            continue
        run_dataset(r, ds, corpus, keep)
    (out / "corpus.json").write_text(json.dumps(selected, indent=1))
    r.results.close()
    write_report(out)
    rows = [json.loads(l) for l in (out / "results.jsonl").read_text().splitlines()]
    return 1 if any(x["status"] == "fail" for x in rows) else 0


def run_citydb(only: set[str] | None, keep: bool) -> int:
    import citydb

    tools = Tools.from_env()
    stamp = dt.datetime.now().strftime("%Y%m%d-%H%M%S")
    out = work_dir() / f"citydb-{stamp}"
    out.mkdir(parents=True)
    python = sys.executable
    env = environment(tools, python)
    env["profile"] = "citydb"
    env["citydb"] = {"pg_image": citydb.PG_IMAGE, "tool_image": citydb.TOOL_IMAGE}
    (out / "environment.json").write_text(json.dumps(env, indent=1))
    print(f"run directory {out}")
    r = Run(out, tools, python)
    corpus = {d["id"]: d for d in load_corpus()}
    corpus_dir = work_dir() / "corpus"
    entries = [e for e in load_citydb() if not only or e.get("id", e.get("dataset")) in only]
    listed = []
    for e in entries:
        ds, src, _, _ = citydb.source_of(e, corpus, corpus_dir, ROOT)
        size = src.stat().st_size if src is not None and src.exists() else ds.get("bytes", 0)
        listed.append(
            {"id": f"citydb:{ds['id']}", "format": ds["format"], "kind": ds.get("kind", "real"), "bytes": size,
             "exercises": f"EPSG:{e['srid']}; {e['exercises']}"}
        )
    why = None if "cityparquet" not in r.missing else "cityparquet binary not found"
    why = why or citydb.available()
    if why:
        for e, d in zip(entries, listed):
            r.record(d, "3DCityDB", "skip", why)
    else:
        with citydb.Server() as server:
            for e in entries:
                citydb.run_dataset(r, e, corpus, corpus_dir, ROOT, server, keep)
    (out / "corpus.json").write_text(json.dumps(listed, indent=1))
    r.results.close()
    write_report(out)
    rows = [json.loads(l) for l in (out / "results.jsonl").read_text().splitlines()]
    return 1 if any(x["status"] == "fail" for x in rows) else 0


def write_report(out: Path) -> None:
    import report

    (out / "report.md").write_text(report.render(out), encoding="utf-8")
    print(f"report {out / 'report.md'}")


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = p.add_subparsers(dest="cmd", required=True)
    f = sub.add_parser("fetch")
    f.add_argument("--profile", choices=PROFILES, default="quick")
    rp = sub.add_parser("run")
    rp.add_argument("--profile", choices=PROFILES, default="quick")
    rp.add_argument("--only", help="comma-separated dataset ids")
    rp.add_argument("--keep", action="store_true", help="keep every derived file, not only those of a dataset that failed")
    cd = sub.add_parser("citydb", help="load sources into 3DCityDB v5 and read them back")
    cd.add_argument("--only", help="comma-separated entry ids")
    cd.add_argument("--keep", action="store_true", help="keep every derived file")
    rr = sub.add_parser("report")
    rr.add_argument("run_dir", type=Path)
    a = p.parse_args()
    if a.cmd == "fetch":
        return fetch(a.profile)
    if a.cmd == "run":
        return run(a.profile, set(a.only.split(",")) if a.only else None, a.keep)
    if a.cmd == "citydb":
        return run_citydb(set(a.only.split(",")) if a.only else None, a.keep)
    write_report(a.run_dir)
    return 0


if __name__ == "__main__":
    sys.path.insert(0, str(HERE))
    sys.exit(main())
