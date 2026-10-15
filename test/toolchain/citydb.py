"""The 3DCityDB family: a dataset loaded into 3DCityDB v5 and read back.

    uv run ... test/toolchain/run.py citydb [--only ID,...] [--keep]

Each `[[citydb]]` entry of corpus.toml names a source (a corpus dataset, a
file in this repository, or one in the citydb-tool image) and the reference
system its database is created in. The source goes through:

    source --citydb-tool import--> DB1 --cityparquet convert--> package
    package --cityparquet export--> CityJSONSeq --citydb-tool import--> DB2
    DB2 --cityparquet convert--> package 2

and these checks, each a step in the report:

- `C2 reader = citydb-tool (DB1)`: `citydb-tool export cityjson` of DB1
  against `cityparquet export` of the package. citydb-tool's exporter is a
  mapping from the database to CityJSON that shares nothing with ours.
- `C3 database round trip`: citydb-tool's export of DB1 against its export of
  DB2. Both sides go through the same exporter, so a difference is a loss on
  our path, or a difference between citydb-tool's CityGML and CityJSON
  importers (DB1 may come from CityGML, DB2 always comes from CityJSON).
- `C4 fixed point`: package 2 exports to what the package exports to
  (`cityparquet compare`).
- `C5`: the package is valid to `cityparquet validate` and to the DuckDB
  extension.
- `C6 timing`: the direct conversion against `citydb-tool export` followed by
  `cityparquet convert` of the export, wall time each, and the peak resident
  memory of cityparquet (citydb-tool's runs in its container and is not
  measured).

The server is a `3dcitydb-pg` container this family starts and removes;
citydb-tool runs as its image. Both are pinned like the database benchmark's.
`CONTAINER` (docker or podman), `CITYDB_PG_IMAGE`, `CITYDB_TOOL_IMAGE` and
`TOOLCHAIN_CITYDB_PORT` override the defaults. Without a container runtime the
family is skipped and the report says so.
"""

from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import time
from pathlib import Path

CONTAINER = os.environ.get("CONTAINER", "docker")
PG_IMAGE = os.environ.get("CITYDB_PG_IMAGE", "ghcr.io/3dcitydb/3dcitydb-pg:5.1.2")
TOOL_IMAGE = os.environ.get("CITYDB_TOOL_IMAGE", "ghcr.io/3dcitydb/citydb-tool:1.3.2")
PORT = int(os.environ.get("TOOLCHAIN_CITYDB_PORT", "55410"))
NAME = "toolchain-citydb"
NETWORK = "toolchain-citydb"
PASSWORD = "changeMe"
IMPORT_FORMAT = {"cityjson": "cityjson", "cityjsonseq": "cityjson", "citygml2": "citygml", "citygml3": "citygml"}


def available() -> str | None:
    """Why the family cannot run, or None when it can."""
    if shutil.which(CONTAINER) is None:
        return f"{CONTAINER} not found"
    for image in (PG_IMAGE, TOOL_IMAGE):
        r = subprocess.run([CONTAINER, "image", "inspect", image], capture_output=True)
        if r.returncode != 0:
            r = subprocess.run([CONTAINER, "pull", image], capture_output=True, text=True)
            if r.returncode != 0:
                return f"cannot pull {image}: {r.stderr.strip()[-300:]}"
    return None


class Server:
    """A 3DCityDB server with one database per dataset."""

    def __enter__(self) -> "Server":
        subprocess.run([CONTAINER, "network", "create", NETWORK], capture_output=True)
        subprocess.run([CONTAINER, "rm", "-f", NAME], capture_output=True)
        subprocess.run(
            [CONTAINER, "run", "-d", "--name", NAME, "--network", NETWORK, "-p", f"{PORT}:5432",
             "-e", f"POSTGRES_PASSWORD={PASSWORD}", PG_IMAGE],
            check=True, capture_output=True,
        )
        for _ in range(120):
            ready = self.psql("postgres", "SELECT 1 FROM pg_database WHERE datname = 'template_postgis'")
            if ready.returncode == 0 and ready.stdout.strip() == "1":
                break
            time.sleep(1)
        else:
            raise RuntimeError("the 3DCityDB server did not start")
        time.sleep(2)
        return self

    def __exit__(self, *_) -> None:
        subprocess.run([CONTAINER, "rm", "-f", NAME], capture_output=True)
        subprocess.run([CONTAINER, "network", "rm", NETWORK], capture_output=True)

    def psql(self, db: str, sql: str) -> subprocess.CompletedProcess:
        return subprocess.run(
            [CONTAINER, "exec", NAME, "psql", "-U", "postgres", "-d", db, "-tAc", sql], capture_output=True, text=True
        )

    def create(self, db: str, srid: int, srs_name: str) -> tuple[int, str]:
        subprocess.run([CONTAINER, "exec", NAME, "dropdb", "-U", "postgres", "--if-exists", db], capture_output=True)
        r = subprocess.run(
            [CONTAINER, "exec", NAME, "createdb", "-U", "postgres", "-T", "template_postgis", db], capture_output=True, text=True
        )
        if r.returncode != 0:
            return r.returncode, r.stderr
        r = subprocess.run(
            [CONTAINER, "exec", "-w", "/3dcitydb", NAME, "psql", "-U", "postgres", "-d", db, "-q",
             "-v", f"srid={srid}", "-v", f"srs_name={srs_name}", "-v", "changelog=no", "-f", "create-db.sql"],
            capture_output=True, text=True,
        )
        return r.returncode, r.stderr

    def drop(self, db: str) -> None:
        subprocess.run([CONTAINER, "exec", NAME, "dropdb", "-U", "postgres", "--if-exists", db], capture_output=True)

    def tool(self, mounts: dict[Path, str], *args: str) -> tuple[int, str, float]:
        """citydb-tool, with `mounts` (a host directory to a container path)."""
        cmd = [CONTAINER, "run", "--rm", "--network", NETWORK, "--user", f"{os.getuid()}:{os.getgid()}"]
        for host, inside in mounts.items():
            cmd += ["-v", f"{host}:{inside}"]
        cmd += [TOOL_IMAGE, *args, "-H", NAME, "-u", "postgres", "-p", PASSWORD]
        t0 = time.time()
        r = subprocess.run(cmd, capture_output=True, text=True)
        return r.returncode, (r.stdout + r.stderr).strip(), time.time() - t0

    def url(self, db: str) -> str:
        # The password goes in PGPASSWORD, not the URL (cityparquet warns
        # about a password on its command line).
        return f"postgresql://postgres@localhost:{PORT}/{db}"


def export_args(entry: dict) -> list[str]:
    """citydb-tool's CityJSON export at the precision of the package: a
    geographic reference system needs nine decimals of a degree."""
    digits = str(entry.get("vertex_precision", 3))
    # No material defaults: a property the source leaves out stays out, as
    # in the package.
    return [
        "--vertex-precision", digits, "--template-precision", "9", "--texture-vertex-precision", "9",
        "--no-material-defaults",
    ]


# What citydb-tool writes into every database and CityJSON file of its own
# accord, removed from both sides of a comparison so that what is left is
# the model. Each is citydb-tool's bookkeeping, not content of the source:
#
# - `creationDate` on city objects and semantic surfaces: its importer stamps
#   every feature with the time of the import (`--creation-date`, "default:
#   attribute value or now"), its CityJSON exporter writes no feature dates,
#   and its CityJSON importer ignores a semantic surface's.
# - ids it generated (`ID_<uuid>`) for semantic surfaces and materials that
#   had none, which its next import generates afresh; and the names both
#   exporters give a material without one (`ID_<uuid>`, `material-<n>`),
#   all spelled `material` (CityJSON requires a name).
# - a texture's image file name and `type`: its exporter renames every image
#   (`tex_<n>.jpg`, numbered as it writes them) and infers the type from the
#   new name; the database stores no MIME type. Whether there is an image is
#   kept.
# 3DCityDB stores texture coordinates as single-precision floats, and
# citydb-tool's exporter writes seven significant digits of them: the direct
# reader hands over the stored float, the exporter its seven-digit rounding,
# which is within 5e-5 of it below 1000 (a repeated texture's coordinates in
# the railway scene reach 714).
UV_TOL = "5e-5"
GENERATED = re.compile(r"^ID_[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$")
UNNAMED = re.compile(r"^(ID_[0-9a-f-]{36}|material-\d+)$")


def without_bookkeeping(src: Path, dest: Path) -> Path:
    def surfaces(geometry: dict) -> None:
        for s in (geometry.get("semantics") or {}).get("surfaces") or []:
            s.pop("creationDate", None)
            if GENERATED.match(str(s.get("id", ""))):
                del s["id"]

    def appearance(app: dict) -> None:
        for m in app.get("materials") or []:
            if UNNAMED.match(str(m.get("name", ""))):
                m["name"] = "material"
        for t in app.get("textures") or []:
            t.pop("type", None)
            if "image" in t:
                t["image"] = "None" if t["image"] in (None, "None") else "image"

    out = []
    for line in src.read_text(encoding="utf-8").splitlines():
        doc = json.loads(line)
        for o in (doc.get("CityObjects") or {}).values():
            (o.get("attributes") or {}).pop("creationDate", None)
            if o.get("attributes") == {}:
                del o["attributes"]
            for g in o.get("geometry") or []:
                surfaces(g)
        for t in doc.get("geometry-templates", {}).get("templates", []):
            surfaces(t)
        appearance(doc.get("appearance") or {})
        out.append(json.dumps(doc, separators=(",", ":")))
    dest.write_text("\n".join(out) + "\n", encoding="utf-8")
    return dest


def source_of(entry: dict, corpus: dict[str, dict], corpus_dir: Path, root: Path) -> tuple[dict, Path | None, str, dict[Path, str]]:
    """The entry as a dataset record, the source on this machine (None for
    one inside the citydb-tool image), its path inside the citydb-tool
    container, and the directory to mount for it."""
    if "dataset" in entry:
        ds = dict(corpus[entry["dataset"]])
        src = corpus_dir / ds["file"]
        return ds, src, f"/src/{ds['file']}", {corpus_dir.resolve(): "/src:ro"}
    ds = {"id": entry["id"], "format": entry["format"], "file": entry.get("path") or entry["image_path"]}
    if "path" in entry:
        src = root / entry["path"]
        return ds, src, f"/src/{src.name}", {src.parent.resolve(): "/src:ro"}
    return ds, None, entry["image_path"], {}


def run_dataset(r, entry: dict, corpus: dict[str, dict], corpus_dir: Path, root: Path, server: Server, keep: bool) -> None:
    ds, src, inside, mounts = source_of(entry, corpus, corpus_dir, root)
    d = r.out / f"citydb-{ds['id']}"
    d.mkdir(parents=True, exist_ok=True)
    rec = dict(ds, id=f"citydb:{ds['id']}")
    print(f"== {rec['id']} ({ds['format']}, {ds['file']}, EPSG:{entry['srid']})", flush=True)
    if src is not None and not src.exists():
        r.record(rec, "corpus", "fail", f"{src} missing: run `run.py fetch` first")
        return
    mounts[d.resolve()] = "/work"
    cp = r.tools.cityparquet
    env = dict(r.env, PGPASSWORD=PASSWORD)
    db1, db2 = f"{ds['id']}_1".replace("-", "_"), f"{ds['id']}_2".replace("-", "_")

    def sh(args: list) -> tuple[int, str, float, int | None]:
        """A command, its output, wall time and peak resident memory (KiB)."""
        probe = (
            "import resource,subprocess,sys;"
            "r=subprocess.run(sys.argv[1:]);"
            "print('maxrss',resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss,file=sys.stderr);"
            "sys.exit(r.returncode)"
        )
        t0 = time.time()
        p = subprocess.run([r.python, "-c", probe, *map(str, args)], capture_output=True, text=True, env=env)
        secs = time.time() - t0
        out = (p.stdout + p.stderr).strip()
        rss = None
        lines = []
        for line in out.splitlines():
            if line.startswith("maxrss "):
                rss = int(line.split()[1])
            else:
                lines.append(line)
        return p.returncode, "\n".join(lines), secs, rss

    def load(db: str, path_inside: str, fmt: str, step: str) -> bool:
        rc, err = server.create(db, entry["srid"], entry["srs_name"])
        if rc != 0:
            r.record(rec, step, "fail", f"create-db: {err[-1500:]}")
            return False
        args = ["import", IMPORT_FORMAT[fmt], path_inside, "-d", db, *entry.get("import_args", [])]
        rc, out, secs = server.tool(mounts, *args)
        counted = server.psql(db, "SELECT count(*) FROM citydb.feature")
        n = counted.stdout.strip()
        ok = rc == 0 and n not in ("", "0")
        r.record(rec, step, "pass" if ok else "fail", f"{n} features" if ok else out[-1500:], seconds=secs)
        return ok

    def convert(db: str, pkg: Path, step: str) -> tuple[bool, float, int | None]:
        rc, out, secs, rss = sh([cp, "convert", server.url(db), "-o", pkg, "--overwrite"])
        last = out.splitlines()[-1] if out else ""
        r.record(rec, step, "pass" if rc == 0 else "fail", out[-1500:] if rc else last, seconds=secs, max_rss_kib=rss)
        return rc == 0, secs, rss

    def plain(path: Path) -> Path:
        return without_bookkeeping(path, path.with_name(path.name.replace(".city.jsonl", ".plain.city.jsonl")))

    def tool_export(db: str, name: str, step: str) -> tuple[Path | None, float]:
        rc, out, secs = server.tool(mounts, "export", "cityjson", "-o", f"/work/{name}", "-d", db, *export_args(entry))
        r.record(rec, step, "pass" if rc == 0 else "fail", "" if rc == 0 else out[-1500:], seconds=secs)
        return (d / name if rc == 0 else None), secs

    try:
        if not load(db1, inside, ds["format"], "citydb-tool import (DB1)"):
            return
        pkg = d / "pkg"
        ok, direct_secs, direct_rss = convert(db1, pkg, "rs convert (DB1)")
        if not ok:
            return

        rc, out, _ = r.sh([cp, "validate", pkg])
        r.record(rec, "C5 rs validate", "pass" if rc == 0 else "fail", out[-1500:])
        if "cityjson_ext" not in r.missing:
            res, secs = r.duck("validate", pkg)
            r.record(rec, "C5 dk validate", "pass" if res.get("ok") and not res.get("errors") else "fail", json.dumps(res)[:1500], seconds=secs)

        ours = d / "rs_export.city.jsonl"
        rc, out, secs = r.sh([cp, "export", pkg, ours])
        r.record(rec, "rs export (package)", "pass" if rc == 0 else "fail", out[-1500:], seconds=secs)
        theirs, export_secs = tool_export(db1, "db1.city.jsonl", "citydb-tool export (DB1)")
        if rc == 0 and theirs is not None:
            r.compare(rec, "C2 reader = citydb-tool (DB1)", plain(theirs), plain(ours), uv_tol=UV_TOL)

        if theirs is not None:
            # C6: the same package by way of citydb-tool's CityJSON export.
            via = d / "pkg_via_export"
            rc6, out6, secs6, rss6 = sh([cp, "convert", theirs, "-o", via, "--overwrite"])
            detail = (
                f"direct {direct_secs:.1f} s, {direct_rss or 0} KiB; "
                f"citydb-tool export {export_secs:.1f} s + cityparquet convert {secs6:.1f} s, {rss6 or 0} KiB"
            )
            # A citydb-tool export cityparquet refuses is C2's finding, not
            # a timing: the indirect path is then not measured.
            r.record(rec, "C6 timing", "pass" if rc6 == 0 else "skip",
                     detail if rc6 == 0 else f"cityparquet refuses citydb-tool's export: {out6[-1200:]}",
                     direct_seconds=direct_secs, direct_rss_kib=direct_rss, export_seconds=export_secs,
                     via_convert_seconds=secs6, via_rss_kib=rss6)

        if rc != 0 or not load(db2, "/work/rs_export.city.jsonl", "cityjsonseq", "citydb-tool import (DB2)"):
            return
        again, _ = tool_export(db2, "db2.city.jsonl", "citydb-tool export (DB2)")
        if theirs is not None and again is not None:
            # DB2 holds the package's coordinates, which are on the
            # package's grid: within a step of DB1's is the same position.
            step = json.loads(ours.open(encoding="utf-8").readline())["transform"]["scale"]
            tol = ",".join(repr(x) for x in step)
            r.compare(rec, "C3 database round trip (citydb-tool exports)", plain(theirs), plain(again), tol, uv_tol=UV_TOL)

        pkg2 = d / "pkg2"
        ok, _, _ = convert(db2, pkg2, "rs convert (DB2)")
        if ok:
            ours2 = d / "rs_export2.city.jsonl"
            rc, out, secs = r.sh([cp, "export", pkg2, ours2])
            if rc != 0:
                r.record(rec, "rs export (package 2)", "fail", out[-1500:], seconds=secs)
            else:
                r.rs_compare(rec, "C4 fixed point (package 2 = package)", plain(ours), plain(ours2))
    finally:
        server.drop(db1)
        server.drop(db2)
        if not keep:
            failed = any(
                json.loads(line)["status"] == "fail"
                for line in (r.out / "results.jsonl").read_text().splitlines()
                if json.loads(line)["dataset"] == rec["id"]
            )
            if not failed:
                shutil.rmtree(d, ignore_errors=True)
