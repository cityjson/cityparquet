"""Requires `just up`, the delft fixture and its converted package, and the
local DuckDB CityJSON extension build. Run with `-m integration`.

The three systems return the same OBJECTS, not just as many: identifier
sets and non-null geometry counts (`citybench.identity`), and `id-lookup`
returns every field of the object.
"""
from __future__ import annotations

import json
from pathlib import Path

import pytest

from citybench.config import Dataset
from citybench.identity import IDENTITY_SCENARIOS, compare, summarise
from citybench.params import derive
from citybench.runner import variants
from citybench.scenarios.registry import ScenarioUnavailable
from citybench.systems.citydb import CityDbSystem
from citybench.systems.cjdb import CjdbSystem
from citybench.systems.duckdb_cp import DuckDBCityParquet

pytestmark = pytest.mark.integration

FIXTURE = Path(__file__).parent.parent / "data" / "delft.city.jsonl"
PACKAGE = Path(__file__).parent.parent / "data" / "cityparquet" / "delft"


@pytest.fixture(scope="module")
def params():
    return derive(FIXTURE, PACKAGE)


@pytest.fixture(scope="module")
def systems():
    dataset = Dataset(name="delft", source=FIXTURE, cityparquet_dir=PACKAGE)
    started = [DuckDBCityParquet(), CjdbSystem(), CityDbSystem()]
    for system in started:
        system.prepare()
        system.ingest(dataset)
    yield started
    for system in started:
        system.teardown()


def _source_object(object_id: str) -> dict:
    for line in FIXTURE.read_text().splitlines():
        feature = json.loads(line)
        if object_id in feature.get("CityObjects", {}):
            return feature["CityObjects"][object_id]
    raise LookupError(object_id)


@pytest.mark.parametrize("scenario", sorted(IDENTITY_SCENARIOS))
def test_every_system_returns_the_same_objects_and_geometries(systems, params, scenario):
    for window, probe in variants(scenario, params):
        summaries = {}
        for system in systems:
            try:
                columns, rows = system.verify_rows(scenario, params, window=window, probe=probe)
            except ScenarioUnavailable:
                continue
            summaries[system.tag] = summarise(scenario, columns, rows)
        assert compare(summaries) is None, (scenario, window, probe)


def _present_probes(params):
    return [probe for _, probe in variants("id-lookup", params) if probe and probe.present]


def test_cjdb_id_lookup_returns_every_attribute_and_every_geometry(systems, params):
    cjdb = next(s for s in systems if s.tag == "cjdb")
    for probe in _present_probes(params):
        source = _source_object(probe.id)
        columns, rows = cjdb.verify_rows("id-lookup", params, probe=probe)
        row = dict(zip(columns, rows[0]))
        attributes, geometry = row["attributes"], row["geometry"]
        attributes = json.loads(attributes) if isinstance(attributes, str) else attributes
        geometry = json.loads(geometry) if isinstance(geometry, str) else geometry
        assert set(attributes or {}) == set(source.get("attributes", {})), probe.id
        assert len(geometry or []) == len(source.get("geometry", [])), probe.id


def test_duckdb_id_lookup_returns_every_attribute_and_every_geometry(systems, params):
    duck = next(s for s in systems if s.tag == "duckdb-cityparquet")
    for probe in _present_probes(params):
        source = _source_object(probe.id)
        columns, rows = duck.verify_rows("id-lookup", params, probe=probe)
        row = dict(zip(columns, rows[0]))
        for key, value in source.get("attributes", {}).items():
            assert key in row, (probe.id, key)
            if value is not None:
                assert row[key] is not None, (probe.id, key)
        geometries = [c for c in columns
                      if c.startswith("geometry_lod") and row[c] is not None]
        # One `geometry_lod*` column per LoD, so one per source geometry.
        assert len(geometries) == len(source.get("geometry", [])), probe.id
