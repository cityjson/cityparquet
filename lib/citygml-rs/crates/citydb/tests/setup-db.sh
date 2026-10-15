#!/usr/bin/env bash
# Start a 3DCityDB v5 server and load the datasets the citygml-citydb tests
# read, one database per dataset, each imported by citydb-tool from a real
# file committed beside the other CityGML crates (or shipped in the
# citydb-tool image). Prints the CITYDB_TEST_URL the tests take.
#
#   tests/setup-db.sh            # start, load, print the URL
#   tests/setup-db.sh stop       # remove the server
#
# CONTAINER (docker or podman), CITYDB_PORT and the image tags can be
# overridden from the environment.
set -euo pipefail

CONTAINER=${CONTAINER:-docker}
PORT=${CITYDB_PORT:-55400}
NAME=${CITYDB_NAME:-citygml-citydb-test}
NETWORK=${CITYDB_NETWORK:-citygml-citydb-test}
PG_IMAGE=${CITYDB_PG_IMAGE:-ghcr.io/3dcitydb/3dcitydb-pg:5.1.2}
TOOL_IMAGE=${CITYDB_TOOL_IMAGE:-ghcr.io/3dcitydb/citydb-tool:1.3.2}
PASSWORD=changeMe

here=$(cd "$(dirname "$0")" && pwd)
crates=$(cd "$here/../.." && pwd)

if [[ ${1:-} == stop ]]; then
  "$CONTAINER" rm -f "$NAME" >/dev/null 2>&1 || true
  "$CONTAINER" network rm "$NETWORK" >/dev/null 2>&1 || true
  exit 0
fi

"$CONTAINER" network create "$NETWORK" >/dev/null 2>&1 || true
"$CONTAINER" rm -f "$NAME" >/dev/null 2>&1 || true
# No SRID: the server starts without a 3DCityDB instance, and each dataset
# gets a database of its own in its own reference system below.
"$CONTAINER" run -d --name "$NAME" --network "$NETWORK" -p "$PORT:5432" \
  -e POSTGRES_PASSWORD=$PASSWORD "$PG_IMAGE" >/dev/null

for _ in $(seq 60); do
  if "$CONTAINER" exec "$NAME" pg_isready -U postgres -h localhost >/dev/null 2>&1 \
    && "$CONTAINER" exec "$NAME" psql -U postgres -tAc "SELECT 1 FROM pg_database WHERE datname = 'template_postgis'" 2>/dev/null | grep -q 1; then
    break
  fi
  sleep 1
done
sleep 2

# database SRID SRS_NAME
create_db() {
  "$CONTAINER" exec "$NAME" createdb -U postgres -T template_postgis "$1"
  "$CONTAINER" exec -w /3dcitydb "$NAME" psql -U postgres -d "$1" -q \
    -v srid="$2" -v srs_name="$3" -v changelog=no -f create-db.sql >/dev/null
}

# database format file... (paths in the container)
import() {
  local db=$1 format=$2
  shift 2
  "$CONTAINER" run --rm --network "$NETWORK" -v "$crates:/crates:ro" "$TOOL_IMAGE" \
    import "$format" "$@" -H "$NAME" -d "$db" -u postgres -p $PASSWORD >/dev/null 2>&1
}

c2=/crates/citygml2/tests/data
c3=/crates/citygml3/tests/data

# CityGML 3.0 files the citygml3 reader's tests read: the database holds
# the model each spells, so the citygml3 reader of the same file is the
# oracle.
create_db plateau3_bldg 6697 "urn:ogc:def:crs:EPSG::6697"
import plateau3_bldg citygml $c3/plateau/53394519_bldg_6697_op.gml
create_db plateau3_tran 6697 "urn:ogc:def:crs:EPSG::6697"
import plateau3_tran citygml $c3/plateau/53394529_tran_6697_op.gml
create_db plateau3_veg 6697 "urn:ogc:def:crs:EPSG::6697"
import plateau3_veg citygml $c3/plateau/53394600_veg_6697_op.gml
create_db plateau3_brid 6697 "urn:ogc:def:crs:EPSG::6697"
import plateau3_brid citygml $c3/plateau/53394610_brid_6697_op.gml
create_db lod4_building3 25832 "urn:ogc:def:crs,crs:EPSG::25832,crs:EPSG::5783"
import lod4_building3 citygml $c3/lod4_building_v2.gml
create_db railway_trees3 25833 "urn:ogc:def:crs:EPSG::25833"
import railway_trees3 citygml $c3/railway_trees_fragment.gml
create_db b1_lod2 25832 "urn:ogc:def:crs:EPSG::25832"
import b1_lod2 citygml $c3/b1_lod2_cs_w_sem.gml
create_db storeys 25832 "urn:ogc:def:crs:EPSG::25832"
import storeys citygml $c3/ogc/BuildingUnits_Storeys_xlink.gml
create_db carriageway 32755 "urn:ogc:def:crs:EPSG::32755"
import carriageway citygml $c3/ogc/citygml3_individual_section_1carriageway.gml

# The same data as CityGML 2.0 and CityJSON, restructured into the CityGML
# 3.0 model by citydb-tool's importers.
create_db delft 7415 "urn:ogc:def:crs,crs:EPSG::28992,crs:EPSG::5109"
import delft cityjson $c2/3dbag/delft_100.city.jsonl

for type in bldg brid frn tran veg wtr; do
  create_db "plateau_$type" 6697 "urn:ogc:def:crs:EPSG::6697"
  import "plateau_$type" citygml $c2/plateau/udx/$type/*.gml
done

create_db lod4_building 25832 "urn:ogc:def:crs,crs:EPSG::25832,crs:EPSG::5783"
import lod4_building citygml $c2/lod4_building_v2.gml

create_db railway_trees 25833 "urn:ogc:def:crs:EPSG::25833"
import railway_trees citygml $c2/railway_trees_fragment.gml

# The KIT LoD 3 railway scene citydb-tool ships, with its texture images.
create_db railway_scene 25833 "urn:ogc:def:crs:EPSG::25833"
import railway_scene citygml /opt/citydb-tool/samples/datasets/railway_scene_lod3_v2.zip

echo "CITYDB_TEST_URL=postgresql://postgres:$PASSWORD@localhost:$PORT"
