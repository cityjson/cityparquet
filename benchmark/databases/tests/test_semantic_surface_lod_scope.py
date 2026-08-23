"""Proves the `semantic-surface` any-LoD fix with real data, not just text.

Background (see the Task 12 report's "fix report" section): `sql_duckdb.py`'s
`semantic-surface` scenario originally checked ONLY `geometry_properties_lod2_2`.
On delft this coincidentally agreed with cjdb's and 3DCityDB's own (correctly
any-LoD) queries, because every BuildingPart that has a RoofSurface at LoD2.2
also happens to have one at every other LoD. That coincidence meant the
harness's own cross-system count-check could not have caught a
single-LoD-vs-any-LoD drift on delft — exactly the failure mode a smoke
target exists to guard against, silently defeated by a lucky fixture.

`tests/fixtures/lod_divergence.city.jsonl` breaks the coincidence
deliberately: one BuildingPart with a RoofSurface classified ONLY at LoD1.2
(LoD1.3 and LoD2.2 both carry the identical geometry with NO semantics at
all). A LoD2.2-only query must NOT match it; the any-LoD query both
`sql_duckdb.py` and `sql_cjdb.py` now express must.

`sql_cjdb.py`'s `SCHEMA` is a hardcoded module constant ("cjdb"), not
parameterised by whatever schema a `CjdbSystem` instance was configured
with — a pre-existing (Task 8) design fact, out of scope to change here
since the harness only ever runs one cjdb schema in practice. So this file
imports the fixture INTO that same shared "cjdb" schema (additively,
alongside whatever delft data is already there — `cjdb import --overwrite`
is scoped to the source FILENAME, not the whole schema, so this never
touches delft's own rows) and checks for this fixture's own object_id in
the matched set, rather than comparing raw aggregate counts, which would
be contaminated by delft's own 1116.
"""

from __future__ import annotations

import subprocess
from pathlib import Path

import duckdb
import pytest

from citybench.config import BBox, Dataset, Params
from citybench.scenarios import sql_cjdb, sql_duckdb
from citybench.systems.cjdb import CjdbSystem

pytestmark = pytest.mark.integration

FIXTURE = Path(__file__).parent / "fixtures" / "lod_divergence.city.jsonl"
OBJECT_ID = "roof-at-lod1-only"
CITYPARQUET_BIN = (
    Path(__file__).resolve().parents[2]
    / "cityparquet-rs" / "target" / "release" / "cityparquet"
)

# semantic-surface's own SQL branches take none of Params' other fields, so
# this is a placeholder satisfying the dataclass, not a meaningful value.
_PARAMS = Params(
    bbox_full=BBox(0.0, 0.0, 0.0, 1.0, 1.0, 1.0), attr_column="object_type",
    attr_eq="x", numeric_column="h_dak_max", target_id="x", parent_id=None,
    total_city_objects=1,
)


@pytest.fixture(scope="module")
def cityparquet_package(tmp_path_factory) -> Path:
    if not CITYPARQUET_BIN.exists():
        pytest.skip(f"{CITYPARQUET_BIN} not built")
    out = tmp_path_factory.mktemp("lod_divergence_cp")
    subprocess.run(
        [str(CITYPARQUET_BIN), "convert", str(FIXTURE), "--output", str(out),
         "--overwrite"],
        check=True, capture_output=True, text=True,
    )
    return out


@pytest.fixture(scope="module")
def cjdb_system():
    # Default schema ("cjdb") deliberately, matching sql_cjdb.py's own
    # hardcoded SCHEMA constant — see this module's docstring.
    system = CjdbSystem()
    system.prepare()
    system.ingest(
        Dataset(name="lod_divergence", source=FIXTURE,
                cityparquet_dir=Path("unused"), hilbert_dir=Path("unused"))
    )
    yield system
    system.teardown()


def _matching_object_ids(conn, count_sql: str, args: tuple) -> set[str]:
    """Reshape a `SELECT count(*) FROM t WHERE <predicate>` into row ids.

    Reuses the exact predicate `sql_cjdb.sql_for` builds (the thing under
    test) rather than a hand-rolled duplicate, just projecting object_id
    instead of aggregating — so the shared "cjdb" schema's other rows
    (delft's own 1116) don't have to be excluded by count arithmetic.
    """
    assert count_sql.startswith("SELECT count(*)"), count_sql
    row_sql = count_sql.replace("SELECT count(*)", "SELECT object_id", 1)
    with conn.cursor() as cur:
        cur.execute(row_sql, args)
        return {row[0] for row in cur.fetchall()}


def test_duckdb_any_lod_query_finds_the_lod1_only_roofsurface(cityparquet_package):
    # The fixture's package holds exactly one object, so a plain count is
    # unambiguous here (unlike the cjdb side, which shares a schema — see
    # this module's docstring).
    table = f"read_parquet('{cityparquet_package}/building.parquet')"
    sql, args = sql_duckdb.sql_for("semantic-surface", _PARAMS, table)
    con = duckdb.connect()
    assert con.execute(sql, list(args)).fetchone()[0] == 1


def test_a_lod2_only_query_would_have_missed_it(cityparquet_package):
    # Reconstructs the OLD, buggy shape directly (not via sql_duckdb.py,
    # which no longer expresses it) to prove the fixture actually
    # discriminates: if this assertion ever failed, the fixture would no
    # longer be exercising the regression the two tests either side of it
    # exist to guard.
    table = f"read_parquet('{cityparquet_package}/building.parquet')"
    con = duckdb.connect()
    lod2_only_sql = (
        f"SELECT count(*) FROM {table} WHERE list_contains("
        "json_extract_string(geometry_properties_lod2_2.surfaces, "
        "'$[*].type'), 'RoofSurface')"
    )
    assert con.execute(lod2_only_sql).fetchone()[0] == 0


def test_cjdb_any_lod_query_finds_the_lod1_only_roofsurface(cjdb_system):
    sql, args = sql_cjdb.sql_for("semantic-surface", _PARAMS)
    ids = _matching_object_ids(cjdb_system._conn, sql, args)
    assert OBJECT_ID in ids


def test_duckdb_and_cjdb_agree_on_the_divergence_fixture(cityparquet_package, cjdb_system):
    # The actual cross-system proof this file exists for: both queries,
    # run through the real adapters' own sql_for builders, must agree that
    # THIS object matches, on a dataset engineered to expose an LoD-scope
    # drift that a raw aggregate count could not have caught on delft.
    table = f"read_parquet('{cityparquet_package}/building.parquet')"
    duck_sql, duck_args = sql_duckdb.sql_for("semantic-surface", _PARAMS, table)
    duck_matches = duckdb.connect().execute(duck_sql, list(duck_args)).fetchone()[0] == 1

    cjdb_sql, cjdb_args = sql_cjdb.sql_for("semantic-surface", _PARAMS)
    cjdb_matches = OBJECT_ID in _matching_object_ids(cjdb_system._conn, cjdb_sql, cjdb_args)

    assert duck_matches is True
    assert cjdb_matches is True
