"""The DuckDB side of the tool-chain test, one operation per process.

    python duck.py write    SOURCE FORMAT PACKAGE    # cityjson|cityjsonseq|fcb -> package
    python duck.py export   PACKAGE OUTPUT FORMAT    # package -> cityjson|cityjsonseq|flatcitybuf
    python duck.py rewrite  PACKAGE OUTPUT           # cityparquet_read, then cityparquet_write
    python duck.py validate PACKAGE                  # PRAGMA cityparquet_validate
    python duck.py measure  INPUT KIND OUT.parquet   # duckdb-3d metrics; KIND package|cityjson|cityjsonseq|fcb

run.py starts each one as its own process, so an extension that crashes
takes one step with it, not the whole run. Every operation prints one JSON
object on stdout. The extensions are loaded from CITYJSON_EXTENSION and
THREE_D_EXTENSION; DuckDB is the version run.py pins.
"""

from __future__ import annotations

import json
import os
import sys

import duckdb


def connect(three_d: bool = False) -> duckdb.DuckDBPyConnection:
    c = duckdb.connect(config={"allow_unsigned_extensions": "true"})
    c.execute(f"LOAD '{os.environ['CITYJSON_EXTENSION']}'")
    if three_d:
        c.execute(f"LOAD '{os.environ['THREE_D_EXTENSION']}'")
    return c


def lit(s: str) -> str:
    return "'" + s.replace("'", "''") + "'"


INSERT = {"cityjson": "insert_cityjson", "cityjsonseq": "insert_cityjsonseq", "fcb": "insert_flatcitybuf"}
READ = {"cityjson": "read_cityjson", "cityjsonseq": "read_cityjsonseq", "fcb": "read_flatcitybuf"}


def write(source: str, fmt: str, package: str) -> dict:
    c = connect()
    c.execute("CREATE SCHEMA p")
    c.execute("PRAGMA cityparquet_init('p')")
    c.execute(f"PRAGMA {INSERT[fmt]}('p', {lit(source)})")
    rows = c.execute(f"SELECT * FROM cityparquet_write('p', {lit(package)})").fetchall()
    return {"files": [[str(x) for x in r] for r in rows]}


def _tables(c, schema: str) -> dict[str, list[str]]:
    rows = c.execute(f"SELECT table_name, role FROM {schema}.__cityparquet ORDER BY 1").fetchall()
    out: dict[str, list[str]] = {}
    for name, role in rows:
        out.setdefault(role, []).append(name)
    return out


def _union(c, objects: list[str]) -> str:
    """Every object table as one relation. A geometry column is GEOMETRY in a
    table whose geometry there is GeoParquet-legal and BLOB in one whose is
    not (spec 05-metadata), and DuckDB cannot unify the two, so a column whose
    types differ across tables is read as WKB BLOB in each."""
    types: dict[str, set[str]] = {}
    cols: dict[str, dict[str, str]] = {}
    for t in objects:
        cols[t] = {n: ty for n, ty, *_ in c.execute(f"DESCRIBE p.{t}").fetchall()}
        for n, ty in cols[t].items():
            types.setdefault(n, set()).add(ty)
    parts = []
    for t in objects:
        mixed = [n for n, ty in cols[t].items() if len(types[n]) > 1 and ty.startswith("GEOMETRY")]
        repl = f" REPLACE ({', '.join(f'ST_AsWKB({n}) AS {n}' for n in mixed)})" if mixed else ""
        parts.append(f"SELECT *{repl} FROM p.{t}")
    return " UNION ALL BY NAME ".join(parts)


def export(package: str, output: str, fmt: str) -> dict:
    c = connect()
    c.execute(f"PRAGMA cityparquet_read({lit(package)}, 'p')")
    roles = _tables(c, "p")
    objects = roles.get("object", [])
    if not objects:
        raise RuntimeError("the package has no object table")
    relation = _union(c, objects)
    # The package's own metadata: CRS and Extension declarations from the
    # object tables' `city` footers. The footer's CRS is PROJJSON and CityJSON
    # states an OGC URL, so the URL is built from the PROJJSON `id`, as the
    # cityparquet-write skill says to state it. A package that states no CRS,
    # or one with no `id`, gets none.
    crs = c.execute(
        "SELECT max(cityparquet_city_field(city, 'crs')) FROM p.__cityparquet WHERE role = 'object'"
    ).fetchone()[0]
    url = None
    if crs is not None:
        ident = (json.loads(crs) if isinstance(crs, str) else crs).get("id") or {}
        if ident.get("authority") and ident.get("code") is not None:
            url = f"https://www.opengis.net/def/crs/{ident['authority']}/0/{ident['code']}"
    meta = (
        (f"SELECT ''{url}'' AS crs, " if url else "SELECT ")
        + "max(cityparquet_city_field(city, ''extensions'')) AS extensions "
        "FROM p.__cityparquet WHERE role = ''object''"
    )
    options = [f"FORMAT {fmt}", f"metadata_query '{meta}'"]
    sidecars = set(roles.get("sidecar", [])) | {
        t for r, ts in roles.items() if r not in ("object",) for t in ts
    }
    if "materials" in sidecars:
        options.append("materials_query 'SELECT * FROM p.materials'")
    if "textures" in sidecars:
        options.append("textures_query 'SELECT * FROM p.textures'")
    if "implicit_geometries" in sidecars and fmt != "flatcitybuf":
        options.append("implicit_geometries_query 'SELECT * FROM p.implicit_geometries'")
    sql = f"COPY ({relation}) TO {lit(output)} ({', '.join(options)})"
    c.execute(sql)
    return {"tables": roles, "sql": sql}


def rewrite(package: str, output: str) -> dict:
    c = connect()
    c.execute(f"PRAGMA cityparquet_read({lit(package)}, 'p')")
    rows = c.execute(f"SELECT * FROM cityparquet_write('p', {lit(output)})").fetchall()
    return {"files": [[str(x) for x in r] for r in rows]}


def validate(package: str) -> dict:
    c = connect()
    c.execute(f"PRAGMA cityparquet_read({lit(package)}, 'p')")
    c.execute("PRAGMA cityparquet_validate('p')")
    rows = c.execute("SELECT * FROM cityparquet_validation").fetchall()
    cols = [d[0] for d in c.description]
    findings = [dict(zip(cols, [str(x) for x in r])) for r in rows]
    errors = [f for f in findings if f.get("severity") == "error"]
    return {"findings": findings[:50], "errors": len(errors), "total": len(findings)}


def measure(src: str, kind: str, out: str) -> dict:
    """duckdb-3d over every geometry column: parsed as a solid, valid, volume,
    surface area, faces, shells and footprint area, one row per (object,
    column). The same SQL runs over a package and over the extension's own
    read of the source, so the three can be joined row for row."""
    c = connect(three_d=True)
    if kind == "package":
        c.execute(f"PRAGMA cityparquet_read({lit(src)}, 'p')")
        tables = [f"p.{t}" for t in _tables(c, "p").get("object", [])]
    else:
        c.execute(f"CREATE TABLE src AS SELECT * FROM {READ[kind]}({lit(src)})")
        tables = ["src"]
    parts = []
    for t in tables:
        cols = [r[0] for r in c.execute(f"DESCRIBE {t}").fetchall()]
        for col in cols:
            if not col.startswith("geometry_lod"):
                continue
            props = col.replace("geometry_", "geometry_properties_", 1)
            arg = f"{col}, {props}" if props in cols else col
            parts.append(
                f"SELECT id, '{col}' AS col, ST_3DTryFromWKB({arg}) AS s FROM {t} WHERE {col} IS NOT NULL"
            )
    if not parts:
        c.execute("CREATE TABLE m (id VARCHAR, col VARCHAR)")
    else:
        c.execute(
            "CREATE TABLE m AS WITH g AS (" + " UNION ALL ".join(parts) + "), "
            "r AS (SELECT id, col, s, CASE WHEN s IS NOT NULL THEN ST_3DValidationReport(s) END AS v FROM g) "
            "SELECT id, col, s IS NOT NULL AS parsed, v.is_valid AS valid, v.face_count AS faces, "
            "v.shell_count AS shells, v.solid_count AS solids, v.degenerate_face_count AS degenerate, "
            "CASE WHEN v.is_valid THEN ST_3DVolume(s) END AS volume, "
            "CASE WHEN v.degenerate_face_count = 0 THEN ST_3DSurfaceArea(s) END AS area, "
            "CASE WHEN s IS NOT NULL THEN ST_3DFootprintArea(s) END AS footprint FROM r"
        )
    c.execute(f"COPY m TO {lit(out)} (FORMAT parquet)")
    n, parsed, valid = c.execute("SELECT count(*), count(*) FILTER (WHERE parsed), count(*) FILTER (WHERE valid) FROM m").fetchone()
    return {"rows": n, "parsed": parsed, "valid": valid}


def main() -> int:
    op, *args = sys.argv[1:]
    try:
        res = {"write": write, "export": export, "rewrite": rewrite, "validate": validate, "measure": measure}[op](*args)
        print(json.dumps({"ok": True, **res}, default=str))
        return 0
    except Exception as e:  # noqa: BLE001 -- the error is the result
        print(json.dumps({"ok": False, "error": f"{type(e).__name__}: {e}"}))
        return 1


if __name__ == "__main__":
    sys.exit(main())
