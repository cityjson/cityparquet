"""lod-query on all three systems: ids plus the LoD 2.2 geometry in each
system's native binary form, and not applicable — never zero — on a dataset
without LoD 2.2."""
import pytest

from citybench import params as params_mod
from citybench.scenarios import sql_citydb, sql_cjdb, sql_duckdb
from citybench.scenarios.registry import LOD_QUERY_TARGET, ScenarioUnavailable
from conftest import make_params

IDS = (5, 7)
TABLE = "read_parquet('/x/building.parquet')"


def test_target_is_lod_2_2():
    assert LOD_QUERY_TARGET == "2.2"


def test_lods_from_columns_reads_the_geometry_column_names():
    cols = ["id", "geometry_lod0_0", "geometry_lod2_2", "geometry_properties_lod2_2",
            "geometry_lod1_3"]
    assert params_mod.lods_from_columns(cols) == ("0", "1.3", "2.2")


def test_lods_round_trip_through_json():
    p = make_params(lods=("0", "2.2"))
    assert params_mod.from_json(params_mod.to_json(p)).lods == ("0", "2.2")


@pytest.mark.parametrize("call", [
    lambda p: sql_duckdb.sql_for("lod-query", p, TABLE),
    lambda p: sql_cjdb.sql_for("lod-query", p),
    lambda p: sql_citydb.sql_for("lod-query", p, cityobject_class_ids=IDS),
])
def test_not_applicable_without_lod_2_2(call):
    with pytest.raises(ScenarioUnavailable):
        call(make_params(lods=("0", "2")))


def test_duckdb_returns_id_and_the_lod_2_2_wkb_only():
    sql, _ = sql_duckdb.sql_for("lod-query", make_params(lods=("2.2",)), TABLE)
    assert sql.startswith("SELECT id, geometry_lod2_2 FROM")
    assert "geometry_lod2_2 IS NOT NULL" in sql and "*" not in sql


def test_cjdb_returns_object_id_and_the_lod_2_2_element_only():
    sql, _ = sql_cjdb.sql_for("lod-query", make_params(lods=("2.2",)))
    assert sql.startswith("SELECT object_id, jsonb_path_query_first(geometry,")
    assert '@.lod == "2.2"' in sql and "@?" in sql and "SELECT *" not in sql


def test_citydb_returns_objectid_and_the_tier_2_geometry_only():
    sql, args = sql_citydb.sql_for("lod-query", make_params(lods=("2.2",)),
                                   cityobject_class_ids=IDS)
    assert "f.objectid, gd.geometry FROM" in sql and "f.*" not in sql
    assert args == (sql_citydb.CITYDB_LOD_TIER,) and sql_citydb.CITYDB_LOD_TIER == "2"
