# CityParquet stack — manual test guide

A step-by-step walkthrough for verifying the three implementations together: the
Rust reference library (`lib/cityparquet-rs`, in-tree) and the two DuckDB
extensions (`lib/duckdb-cityjson`, `lib/duckdb-3d`, both submodules). The
automated part of this — every suite in the tree, one line per stage — is
`./test/run-all.sh`; this document is the part a script cannot hold: the
expected outputs, the cross-module chains and the known issues.

Every command in Parts 0–4 was **executed on 2026-10-08** (macOS arm64, Rust
1.93.1; both extension shells are DuckDB **v1.5.4**, the `duckdb` on `PATH` is
v1.5.5) against these commits, and every expected output is the **real**
observed value, not an illustration:

| Repository                                               | Commit    |
| -------------------------------------------------------- | --------- |
| monorepo (`lib/cityparquet-rs`, `documents/`, `test/`)   | `e35e9a3` |
| `lib/duckdb-cityjson` (submodule, `develop`)             | `939e1f1` |
| `lib/duckdb-3d` (submodule, `develop`)                   | `94268e7` |
| `lib/cityparquet-rs/vendor/city3d-stac-tool` (submodule) | `c8ab8d9` |

The two DuckDB submodules are checked out at their `develop` heads, ahead of the
pointers the monorepo commit records; the run used the checkouts. The
walkthrough started on monorepo `8d12563`; every `cityparquet` output pasted
below was re-run on `e35e9a3` (a texture-UV tolerance fix) and is unchanged
apart from the one extra warning line it adds in 4.5.

Part 5 (benchmarks) is **procedure only**: the recipes are inspected and the
corpus state is checked, but the multi-hour runs are not part of this pass.

Run everything from the repository root unless stated otherwise:

```sh
cd cityparquet          # wherever you cloned github.com/cityjson/cityparquet
```

---

## Part 0 — Prerequisites

```sh
for c in duckdb fcb uv just cargo cmake ninja python3 jq; do
  printf "%-8s " "$c"; command -v $c || echo MISSING
done
lib/duckdb-cityjson/build/release/duckdb --version   # v1.5.4, once built (0.4)
```

All must resolve. `fcb` (the FlatCityBuf CLI, 0.7.8 here) is needed by the read
benchmark and its tests, `uv` by the benchmark charts and the catalogue
driver's suite, `jq` by the benchmark shell suites. The `duckdb` on `PATH`
serves the plain-Parquet checks; the extensions load only into the DuckDB they
were built against, which is each submodule's own `build/release/duckdb`.

### 0.1 Check out cityparquet-rs's vendored submodule

`lib/cityparquet-rs` vendors `city3d-stac-tool` under `vendor/`, and `just check`
gates on it (`vendor-check`):

```sh
(cd lib/cityparquet-rs && git submodule update --init)
```

### 0.2 Fetch the real-data fixtures

`just check` reads real fixtures, never inline CityJSON, and fails with
`fixture must exist; run 'just fixtures'` without them:

```sh
(cd lib/cityparquet-rs && just fixtures)
ls lib/cityparquet-rs/tests/fixtures/
# b1_lod2_cs_w_sem.gml  b1_lod2_s.gml  berlin_citygml1.gml  delft.city.jsonl  delft.fcb
# empty.city.jsonl  freiburg_no_preamble_srs.gml  lod3_railway.city.json  lod4_building_v2.gml
```

- `delft.city.jsonl` — a 3DBAG tile of Delft, 2231 objects (the workhorse).
- `lod3_railway.city.json` — the CityGML LoD3 railway sample: nine CityGML
  modules, materials, textures and implicit geometries.
- `delft.fcb` — the published FlatCityBuf encoding of Delft (1.12).
- `lod4_building_v2.gml` — the SIG3D LoD4 building, as citygml4j ships it (1.13).
- `freiburg_no_preamble_srs.gml` is a **400 kB HTTP range request** against a
  1.5 GiB source; a full download is neither needed nor attempted.

### 0.3 vcpkg must contain the pinned baseline

Both DuckDB extensions pin vcpkg `builtin-baseline`
`84bab45d415d22042bd0b9081aea57f362da3f35` (2025-12-13); duckdb-cityjson also
resolves `flatcitybuf` from a git registry (`HideBa/vcpkg`, its own baseline).
The vcpkg **working tree** must contain the builtin baseline: vcpkg reads
`baseline.json` from that commit but resolves version files from the working
tree. Check it:

```sh
V=${VCPKG_ROOT:-$HOME/vcpkg}
git -C $V merge-base --is-ancestor 84bab45d415d22042bd0b9081aea57f362da3f35 HEAD \
  && echo ancestor || echo "not ancestor"
# ancestor
```

An older checkout fails in two distinct ways, one after the other.
duckdb-cityjson dies at configure when the commit is absent:

```
fatal: path 'versions/baseline.json' exists on disk, but not in '84bab45d…'
while loading baseline version for openssl
-- Running vcpkg install - failed
```

and duckdb-3d, when the commit is present but the working tree is older, finds
no version database entry for the pinned `proj`:

```
error: no version database entry for proj at 9.7.1.
```

Put the tree on (or past) the baseline. A typical vcpkg checkout is shallow, so
fetch the commit explicitly and detach onto it:

```sh
git -C $V fetch --depth 1 origin 84bab45d415d22042bd0b9081aea57f362da3f35
git -C $V checkout --detach 84bab45d415d22042bd0b9081aea57f362da3f35
```

The first duckdb-3d configure after that is slow — vcpkg builds `proj` and its
dependencies from source.

### 0.4 Build all three components

```sh
# 1. cityparquet-rs (Rust); `collection` and FlatCityBuf input are default features
cargo build --release --manifest-path lib/cityparquet-rs/Cargo.toml -p cityparquet-cli

# 2. duckdb-cityjson — `just rebuild` is the inner-loop command
(cd lib/duckdb-cityjson && just rebuild)

# 3. duckdb-3d
(cd lib/duckdb-3d && just build)
```

Artefacts referenced later:

| Component                          | Path                                                                             |
| ---------------------------------- | -------------------------------------------------------------------------------- |
| `cityparquet` CLI                  | `lib/cityparquet-rs/target/release/cityparquet`                                  |
| cityjson extension                 | `lib/duckdb-cityjson/build/release/extension/cityjson/cityjson.duckdb_extension` |
| three_d extension                  | `lib/duckdb-3d/build/release/extension/three_d/three_d.duckdb_extension`         |
| DuckDB shell w/ cityjson preloaded | `lib/duckdb-cityjson/build/release/duckdb`                                       |
| DuckDB shell w/ three_d preloaded  | `lib/duckdb-3d/build/release/duckdb`                                             |

Each submodule's `build/release/duckdb` has _its own_ extension statically
loaded. To use both together, load the other one's `.duckdb_extension` file
explicitly (Part 4). `INSTALL cityjson FROM community` is not needed and not
supported: the published community build predates the per-LoD column shape.

---

## Part 1 — cityparquet-rs: write, read, check CityParquet

```sh
export CP=lib/cityparquet-rs/target/release/cityparquet
export OUT=/tmp/cp_test && mkdir -p $OUT
```

### 1.1 The library's gate

```sh
(cd lib/cityparquet-rs && just check)
```

`check: lint test isolation vendor-check`, then `cargo fmt --all --check` and a
Prettier pass over the Markdown. `test` is `cargo nextest run --workspace
--all-features`; the read-benchmark harness is its own workspace
(`benchmark/readbench`) and is not part of this gate. `vendor-check` runs
`fmt`/`clippy`/`test` over `vendor/city3d-stac-tool` as well, because that
submodule is deliberately outside the Cargo workspace.

Expected `test` summary (debug profile; the `test` recipe alone takes ~93 s on
a warm build):

```
     Summary [  92.978s] 836 tests run: 836 passed, 6 skipped
```

`just check` exits non-zero on the first failing recipe, so a red `test` hides
whether the later gates would pass. They run on their own too:

```sh
(cd lib/cityparquet-rs && just lint && just isolation && just vendor-check && cargo fmt --all --check)
```

### 1.2 Convert a single-module dataset (Delft, 2231 objects)

```sh
$CP convert lib/cityparquet-rs/tests/fixtures/delft.city.jsonl -o $OUT/delft --overwrite
ls $OUT/delft
```

The report has ten space-separated fields — `object_count files_count
skipped_same_lod_geometries attribute_coercion_nulls degenerate_rings_dropped
degenerate_surfaces_dropped materials_written textures_written
implicit_geometries_written invalid_appearance_refs_dropped`:

```
2231 2 0 0 0 0 0 0 0 0
```

and the package is one object table per CityGML module, plus the STAC Item:

```
building.parquet
metadata.json
```

### 1.3 The package is plain-Parquet readable

```sh
duckdb -c "SELECT count(*) FROM '$OUT/delft/building.parquet';"          # 2231
duckdb -c "DESCRIBE SELECT * FROM read_parquet('$OUT/delft/building.parquet');"
```

The column order is the specification's:

- `id`, `feature_id`, `object_type`, `parents`, `children`, `children_roles`
- `address` — a list of `STRUCT(street, house_number, po_box, zip_code, city, state, country, free_text, location)`
- `bbox` — `STRUCT(xmin, ymin, zmin, xmax, ymax, zmax)`
- one **quad per LoD**, LoD0 included and suffixed like every other:
  `geometry_lod0_0`, `geometry_properties_lod0_0`, `material_lod0_0`,
  `texture_lod0_0` — and the same for `lod1_2`, `lod1_3`, `lod2_2`
- `implicit_geometry` — `STRUCT(id BIGINT, point BLOB, transformationMatrix DOUBLE[])`
- `other` (`JSON`), then the flattened typed attribute columns (`b3_*`)

Delft's LoD0 comes from the source: 3DBAG carries a footprint on every
`Building`. Three things to check in the Parquet logical types:

```sh
duckdb -list -c "SELECT name, logical_type FROM parquet_schema('$OUT/delft/building.parquet')
                 WHERE name IN ('geometry_lod2_2','surfaces','other');"
```

```
name|logical_type
surfaces|JsonType()
surfaces|JsonType()
surfaces|JsonType()
geometry_lod2_2|NULL
surfaces|JsonType()
other|JsonType()
```

(`surfaces` once per LoD, the `geometry_properties_lod*` field.)

- **`geometry_lod0_0` is the only GeoParquet-legal column**, and DuckDB reports
  it as `GEOMETRY`; every higher LoD is a plain `BLOB`. The Parquet `GEOMETRY`
  logical type does the work, not the `geo` footer: the writer annotates a
  column exactly when it declares it in `geo`, so the two cannot disagree about
  which columns a GeoParquet reader may touch. The annotation carries the CRS
  as the full PROJJSON (`GeometryType(crs={"$schema":…,"type":"CompoundCRS",…})`).

  A `PolyhedralSurface Z` is deliberately left unannotated. DuckDB promotes any
  annotated column and converts it eagerly, and its geometry model has no
  PolyhedralSurface, so annotating a solid column would make even
  `SELECT count(*)` over it fail before any `ST_3D*` function sees a value.

- **`other` and `geometry_properties_lod*.surfaces` carry the Parquet `JSON`
  logical type**, so any Parquet reader knows they hold JSON text.

- **`geometry_properties_lod*` is a STRUCT**:
  `STRUCT("type" VARCHAR, surfaces JSON, face_semantics INTEGER[], shells INTEGER[][])`.

Footer metadata — three keys, and the two objects name different primary
columns by design (`geo` must name a legal column, `city` names the richest):

```sh
duckdb -list -c "SELECT key::VARCHAR AS k, octet_length(value) AS len
                 FROM parquet_kv_metadata('$OUT/delft/building.parquet');"
```

```
ARROW:schema|23464
city|4078
geo|2226
```

```sh
duckdb -list -c "SELECT json_extract_string(decode(value),'\$.primary_column') AS primary_col,
                        json_keys(json_extract(decode(value),'\$.columns'))    AS geo_cols
                 FROM parquet_kv_metadata('$OUT/delft/building.parquet') WHERE key::VARCHAR='geo';"
```

Expected: `geometry_lod0_0|[geometry_lod0_0]`. The `city` object's own
`primary_column` is `geometry_lod2_2`, and its `columns` is a list with one
entry per geometry column, each `"encoding": "WKB"` — the only encoding
CityParquet defines — plus `geometry_types` and `orientation_3d`.

### 1.4 The STAC `metadata.json`

```sh
python3 -c "
import json; d=json.load(open('$OUT/delft/metadata.json'))
print('type:', d['type'], '| stac_version:', d['stac_version'])
print('extensions:', d['stac_extensions'])
p=d['properties']
for k in ['city3d:city_objects','city3d:lods','city3d:co_types','city3d:version',
          'cityparquet:version','proj:code','city3d:semantic_surfaces']:
    print(' ', k, '=', p[k])
for k,v in d['assets'].items(): print(' asset', k, v.get('roles'))
"
```

```
type: Feature | stac_version: 1.1.0
extensions: ['https://cityjson.github.io/stac-city3d/v0.2.0/schema.json', 'https://stac-extensions.github.io/projection/v2.0.0/schema.json', 'https://stac-extensions.github.io/file/v2.1.0/schema.json']
  city3d:city_objects = 2231
  city3d:lods = ['0.0', '1.2', '1.3', '2.2']
  city3d:co_types = ['Building', 'BuildingPart']
  city3d:version = 2.0
  cityparquet:version = 0.1.0-draft
  proj:code = EPSG:7415
  city3d:semantic_surfaces = True
 asset building.parquet ['data', 'cityparquet-objects']
```

Object tables are discovered by the `cityparquet-objects` asset role; sidecars
carry `cityparquet-sidecar`. There is no top-level `tables` key. `datetime` is
the source's `referenceDate`, else the conversion time; `--datetime` pins it for
a byte-reproducible package.

**The Item follows the CRS state.** Compare Delft with the CRS-less railway
package from 1.5:

```sh
python3 -c "
import json
for p in ['delft','railway']:
    d=json.load(open('$OUT/%s/metadata.json'%p))
    pr=d['properties']
    print(p, '| proj keys:', [k for k in pr if k.startswith('proj:')],
          '| geometry:', 'null' if d.get('geometry') is None else 'present',
          '| bbox:', 'present' if d.get('bbox') else 'absent')
"
```

```
delft | proj keys: ['proj:code'] | geometry: present | bbox: present
railway | proj keys: [] | geometry: null | bbox: absent
```

With no resolvable CRS there is nothing to reproject to WGS84, so the Item
declares no footprint rather than a fabricated one, and the `projection`
extension drops out of `stac_extensions`.

### 1.5 Convert a multi-module dataset (railway) — and the tri-state CRS

The railway fixture declares no `referenceSystem`. That is declared, not fatal:

```sh
$CP convert lib/cityparquet-rs/tests/fixtures/lod3_railway.city.json -o $OUT/railway --overwrite
ls $OUT/railway
```

A `warning:` on stderr, then a normal conversion — 85 materials, 34 textures
and 3 implicit geometries written:

```
warning: source carries a CRS-bearing coordinate (geometry, or a GeometryInstance template placement) but declares no CRS; `city.crs` is written as an explicit null (CRS unknown) and the coordinates carry no georeference — supply the CRS explicitly to georeference them
121 13 0 0 0 0 85 34 3 0
```

`--crs` georeferences such a source. It applies only when the source declares
none, and the output records that it was supplied:

```sh
$CP convert lib/cityparquet-rs/tests/fixtures/lod3_railway.city.json \
    -o $OUT/railway_crs --crs EPSG:25832 --overwrite
```

No warning, the same `121 13 0 0 0 0 85 34 3 0` report, and
`city.other.crs_source = "operator-supplied"` in the footer.

**Verify all three CRS states** — `null` and absent mean different things, and
neither may be confused with a known CRS:

```sh
for t in delft/building railway/building railway/materials; do
  printf "%-20s " "$t"
  duckdb -noheader -list -c "SELECT decode(value) FROM parquet_kv_metadata('$OUT/$t.parquet') WHERE key::VARCHAR='city';" \
  | python3 -c "import sys,json; d=json.load(sys.stdin); print('crs key present:', 'crs' in d, '| value:', type(d['crs']).__name__ if d.get('crs') is not None else d.get('crs', 'ABSENT'))"
done
```

```
delft/building       crs key present: True | value: dict
railway/building     crs key present: True | value: None
railway/materials    crs key present: False | value: ABSENT
```

Known (a PROJJSON object), unknown (an explicit `null`), and unspecified (a
geometry-free sidecar, no key at all).

Per GeoParquet an **absent** `crs` asserts OGC:CRS84, so writing absent over
projected coordinates would silently move the city; that is why `null` exists
and why only a geometry-free sidecar may omit the key.

The package holds **9 module tables** + 3 sidecars + `metadata.json` = 13
files. The module split, and the CityGML CM class names inside each:

```sh
for f in bridge building city_furniture generics relief transportation tunnel vegetation water_body; do
  echo -n "$f: "
  duckdb -noheader -list -c \
    "SELECT count(*)||' rows | '||string_agg(DISTINCT object_type, ',' ORDER BY object_type) FROM '$OUT/railway/$f.parquet';"
done
```

```
bridge: 9 rows | Bridge,BridgeConstructiveElement,BridgeInstallation
building: 59 rows | Building,BuildingInstallation
city_furniture: 11 rows | CityFurniture
generics: 3 rows | CityObjectGroup,GenericOccupiedSpace
relief: 1 rows | TINRelief
transportation: 10 rows | Railway
tunnel: 12 rows | Tunnel,TunnelInstallation
vegetation: 15 rows | SolitaryVegetationObject
water_body: 1 rows | WaterBody
```

`Bridge`, `BridgeInstallation` and `BridgeConstructiveElement` share one file
because they share a CityGML module; `object_type`, not the filename, carries
the class.

```sh
duckdb -list -c "SELECT
  (SELECT count(*) FROM read_parquet('$OUT/railway/materials.parquet'))           AS materials,
  (SELECT count(*) FROM read_parquet('$OUT/railway/textures.parquet'))            AS textures,
  (SELECT count(*) FROM read_parquet('$OUT/railway/implicit_geometries.parquet')) AS implicit_geometries;"
```

Expected: `85|34|3`. Sidecars are content-gated: written whenever the source
has that content, with no flag to opt in.

### 1.6 Round trip: convert → export → compare

The core semantic-losslessness claim:

```sh
$CP convert lib/cityparquet-rs/tests/fixtures/delft.city.jsonl -o $OUT/delft_rt --overwrite
$CP export $OUT/delft_rt $OUT/delft_rt.city.jsonl
$CP compare lib/cityparquet-rs/tests/fixtures/delft.city.jsonl $OUT/delft_rt.city.jsonl
echo "exit=$?"
```

`export` prints `feature_count object_count instance_geometries_dropped
appearance_refs_dropped`:

```
2231 2 0 0 0 0 0 0 0 0
1115 2231 0 0
equal (excluded: 20)
exit=0
```

The multi-module railway, sidecars and all, round-trips the same way:

```sh
$CP convert lib/cityparquet-rs/tests/fixtures/lod3_railway.city.json -o $OUT/railway_rt --overwrite
$CP export $OUT/railway_rt $OUT/railway_rt.city.json
$CP compare lib/cityparquet-rs/tests/fixtures/lod3_railway.city.json $OUT/railway_rt.city.json
echo "exit=$?"
```

```
121 13 0 0 0 0 85 34 3 0
38 121 0 0
equal (excluded: 26)
exit=0
```

> **LoD0 synthesis is opt-in, and it changes the content.** `--lod0`
> synthesises a footprint into `geometry_lod0_0` for every object that lacks a
> source LoD0, so the GeoParquet-legal column is populated. Round-tripping such
> a package adds geometry the source never had, and `compare` reports it (exit
> 2, one line per object):
>
> ```sh
> $CP convert lib/cityparquet-rs/tests/fixtures/lod3_railway.city.json -o $OUT/railway_lod0 --lod0 --overwrite
> $CP export $OUT/railway_lod0 $OUT/railway_lod0.city.json
> $CP compare lib/cityparquet-rs/tests/fixtures/lod3_railway.city.json $OUT/railway_lod0.city.json
> ```
>
> ```
> object GMLID_0373494_301709_129: geometry at lod Some("0.0") present in B, missing in A
> object GMLID_0528986_170479_94: geometry at lod Some("0.0") present in B, missing in A
> …
> ... (31 more)
> ```
>
> Without `--lod0` the package holds only what the source carries, which is why
> every round trip in this guide converts without it.

### 1.7 The bundled interop script

```sh
(cd lib/cityparquet-rs && just interop)
```

Expected last lines: `interop bloom cross-writer ok` and `interop ok`. It
converts both fixtures and has DuckDB assert the results natively — 2231 Delft
buildings, a `bbox.xmin` query, the 85/34/3 sidecars, 121 rows unioned across
railway's nine module tables (`union_by_name = true`, since two modules need not
share a column list) — then, when a duckdb-cityjson build is present, probes
every DuckDB-written id and feature id through the Rust bloom-filter prune
(`2231 DuckDB-written ids probed, none lost`). The railway conversion prints the
CRS warning from 1.5 on stderr; that is expected, not a failure.

### 1.8 Validate a package against the specification

`cityparquet validate` checks a package against the specification's MUST
(error) and SHOULD (warning) statements at the Parquet logical-type level —
column types and nullability, the `city`/`geo` footers, sidecars and the STAC
Item — and assumes nothing about how the writer laid the files out. Exit 2 on
any error.

```sh
for p in delft railway; do echo "== $p"; $CP validate $OUT/$p; echo "exit=$?"; done
```

```
== delft
0 error(s), 0 warning(s)
exit=0
== railway
0 error(s), 0 warning(s)
exit=0
```

The same check over duckdb-cityjson's packages is in 2.4 and 2.7, and both
writers are checked against each other in 4.4.

### 1.9 Partitioned output

```sh
$CP convert lib/cityparquet-rs/tests/fixtures/delft.city.jsonl \
    -o $OUT/delft_parts --partition features --feature-num 500 --overwrite
ls $OUT/delft_parts
duckdb -c "SELECT count(*) FROM read_parquet('$OUT/delft_parts/*/building.parquet');"
```

`convert` reports the partition count, a duplicate-id check and the dropped
appearance references, then one line per partition:

```
partitions=3 duplicate_ids=0 invalid_appearance_refs_dropped=0
features-00000 1001 0
features-00001 1000 0
features-00002 230 0
```

`ls` shows the three package directories `features-00000/1/2`, and the glob
query returns `2231` — every object, exactly once. `--partition count --number
N` and `--partition box --cell-size METRES` are the other two methods.

### 1.10 The STAC catalogue driver (`scripts/catalog2cityparquet`)

A Python driver that walks the published City3D STAC catalogue, converts each
item, and ledgers _why_ each one did or did not convert. Its own suite fakes
every origin and subprocess — no network, no binaries:

```sh
just catalog-test
```

Expected: **301 passed, 11 skipped** (run by `test/run-all.sh`, Part 6).

Everything else in this tool is network-dependent and out of scope for a
routine test pass — `just catalog-convert` is hours long against the live
catalogue. One small collection is the unit for a real-data check:

```sh
just catalog-convert-collection rotterdam-3d out/e2e
just catalog-histogram out/e2e
```

Roll the ledger up with `catalog-histogram`, never by counting lines in
`_reports/*.jsonl`: the files are append-only and a resumed run re-attempts a
previously failed item, so line-counting over-counts failures.

### 1.11 A STAC Collection over several packages

`cityparquet collection` aggregates packages' `metadata.json` Items into one
STAC Collection — `city3d:*` summaries, the union of their extents, one `item`
link per package:

```sh
$CP collection $OUT/delft $OUT/railway -o $OUT/collection.json \
    --id cp-test --title "CityParquet walkthrough" --license CC-BY-4.0
python3 -c "
import json; c=json.load(open('$OUT/collection.json'))
print('type:', c['type'], '| id:', c['id'], '| stac_version:', c['stac_version'])
print('extent.spatial.bbox:', c['extent']['spatial']['bbox'])
s=c['summaries']
for k in sorted(s): print(' ', k, '=', s[k])
print('item links:', [l['href'] for l in c['links'] if l['rel']=='item'])
print('item_assets:', list(c.get('item_assets',{}).keys()))
"
```

```
2 items aggregated into /tmp/cp_test/collection.json
type: Collection | id: cp-test | stac_version: 1.1.0
extent.spatial.bbox: [[4.36071356020891, 51.99711429598357, -3.7475221157073975, 4.378046226704813, 52.00785440056329, 95.04304504394533]]
  city3d:city_objects = {'min': 121, 'max': 2231, 'total': 2352}
  city3d:co_types = ['Bridge', 'BridgeConstructiveElement', 'BridgeInstallation', 'Building', 'BuildingInstallation', 'BuildingPart', 'CityFurniture', 'CityObjectGroup', 'GenericCityObject', 'Railway', 'SolitaryVegetationObject', 'TINRelief', 'Tunnel', 'TunnelInstallation', 'WaterBody']
  city3d:lods = ['0.0', '1.2', '1.3', '2.2', '3.0']
  city3d:materials = [False, True]
  city3d:semantic_surfaces = [True]
  city3d:textures = [False, True]
  city3d:version = ['2.0']
  proj:code = ['EPSG:7415']
item links: ['./delft/metadata.json', './railway/metadata.json']
item_assets: ['data']
```

Three things to read off it:

- The spatial extent is Delft's alone: the CRS-less railway Item declares no
  WGS84 bbox (1.4), so it contributes nothing to the union rather than a
  fabricated box.
- `city3d:co_types` uses the **source** vocabulary (`GenericCityObject`), not
  `object_type`'s CityGML class (`GenericOccupiedSpace`), as
  `documents/docs/03-specification/05-metadata.mdx` specifies.
- `item_assets` lists `data`, which no CityParquet Item carries — it comes from
  the upstream `city3d-stac-gen` builder (Known issues #18).

### 1.12 FlatCityBuf input

`convert` reads FlatCityBuf (`.fcb`, the `fcb` cargo feature, on in the CLI)
front to back, its header standing in for the CityJSON one:

```sh
$CP convert lib/cityparquet-rs/tests/fixtures/delft.fcb -o $OUT/delft_fcb --overwrite
$CP validate $OUT/delft_fcb
duckdb -noheader -list -c "SELECT json_extract_string(decode(value),'\$.source_format')
  FROM parquet_kv_metadata('$OUT/delft_fcb/building.parquet') WHERE key::VARCHAR='city';"
```

```
2231 2 0 0 0 0 0 0 0 0
0 error(s), 0 warning(s)
FlatCityBuf
```

The published `delft.fcb` is not byte-for-byte the `delft.city.jsonl` fixture —
it carries an LoD0 footprint on every `BuildingPart` too — so the round trip
compares against the FlatCityBuf file's own CityJSON, decoded by the `fcb` CLI:

```sh
fcb deser lib/cityparquet-rs/tests/fixtures/delft.fcb $OUT/delft_from_fcb.city.jsonl
$CP export $OUT/delft_fcb $OUT/delft_fcb.city.jsonl
$CP compare $OUT/delft_from_fcb.city.jsonl $OUT/delft_fcb.city.jsonl
echo "exit=$?"
```

```
Successfully decoded to CityJSON
1115 2231 0 0
equal (excluded: 20)
exit=0
```

### 1.13 CityGML LoD4

CityGML 2.0 LoD4 is kept as LoD4: a `lod4Solid` becomes `geometry_lod4_0`.

```sh
$CP convert lib/cityparquet-rs/tests/fixtures/lod4_building_v2.gml -o $OUT/lod4 --overwrite
duckdb -list -c "SELECT id, object_type, geometry_properties_lod4_0.type AS t
                 FROM '$OUT/lod4/building.parquet' WHERE geometry_lod4_0 IS NOT NULL;"
```

```
1 2 0 0 0 0 0 0 0 0
id|object_type|t
GML_7b1a5a6f-ddad-4c3d-a507-3eb9ee0a8e68|Building|Solid
```

The Item's `city3d:lods` is `['4.0']`. CityJSON 2.0 defines LoDs 0 to 3 only,
so a CityJSON export writes `"4.0"` and says so; a CityGML export writes the
`lod4Solid` back:

```sh
$CP export $OUT/lod4 $OUT/lod4.city.json
$CP export $OUT/lod4 $OUT/lod4_rt.gml
```

```
1 1 0 0
warning: 1 geometries are LoD 4, written as "4.0"; CityJSON 2.0 defines LoDs 0 to 3 only
1 buildings written; 0 non-building skipped, 0 without geometry, 0 composite solids written, 0 multi-solids skipped, 0 lod columns skipped, 5 attributes written, 0 attributes skipped
```

What the CityGML reader leaves out (the building's interior `Room`, for one) is
listed in `lib/cityparquet-rs/docs/citygml-reader-writer-limitations.md`.

---

## Part 2 — duckdb-cityjson extension

```sh
export DK=lib/duckdb-cityjson/build/release/duckdb
```

### 2.1 SQL test suite

```sh
(cd lib/duckdb-cityjson && ./build/release/test/unittest "test/sql/*")
```

Expected: `All tests passed (5 skipped tests, 2243 assertions in 80 test cases)`.
The five skips are `require-env` gates on network access:

```
require-env CITYJSON_NOTEBOOK_TEST: 2    cityjson_notebook_e2e, cityjson_notebook_geoparquet
require-env CITYJSON_REMOTE_TEST: 2      cityjson_remote, cityjson_corpus_parity
require-env FCB_REMOTE_TEST_URL: 1       cityjson_fcb_remote (2.9)
```

### 2.2 Read CityJSONSeq

```sh
$DK -c "SELECT count(*) AS objects
        FROM read_cityjsonseq('lib/cityparquet-rs/tests/fixtures/delft.city.jsonl');"
```

Expected: `2231` — the object count cityparquet-rs reports. Two implementations
agreeing on it is the point of the check.

### 2.3 Per-LoD WKB mode

Without `lod =>` you get `geom_lod*` STRUCTs. With it you get the grammar the
Parquet file uses: `geometry_lodX_Y` (BLOB/WKB) + `geometry_properties_lodX_Y`
(STRUCT) + `material_lodX_Y` / `texture_lodX_Y`. There is no bare `geometry`
column — that keeps the LoD recoverable from the column name, and lets
`COPY TO cityjson` re-emit it.

```sh
$DK -list -c "
SELECT id, geometry_properties_lod2_2
FROM read_cityjsonseq('lib/cityparquet-rs/tests/fixtures/delft.city.jsonl', lod => '2.2')
WHERE geometry_lod2_2 IS NOT NULL LIMIT 2;"
```

```
id|geometry_properties_lod2_2
NL.IMBAG.Pand.0503100000012869-0|{'type': Solid, 'surfaces': '[{"type":"GroundSurface"},{"type":"RoofSurface"},{"on_footprint_edge":true,"type":"WallSurface"},{"on_footprint_edge":false,"type":"WallSurface"}]', 'face_semantics': [0, 2, 2, 2, 2, 1], 'shells': [[6]]}
NL.IMBAG.Pand.0503100000016459-0|{'type': Solid, 'surfaces': '[{"type":"GroundSurface"},{"type":"RoofSurface"},{"on_footprint_edge":true,"type":"WallSurface"},{"on_footprint_edge":false,"type":"WallSurface"}]', 'face_semantics': [0, 2, 2, 2, 2, 1], 'shells': [[6]]}
```

`shells` is `INTEGER[][]`, nested unconditionally, so a plain `Solid` is `[[6]]`.
`type` is the CityGML CM geometry name (`Solid`, `MultiSurface`, `MultiCurve`,
…). In the scan, `surfaces` is `VARCHAR` — the `json` extension is not a
dependency of this one — and `cityparquet_write` annotates it as Parquet `JSON`.

### 2.4 Write a package from CityJSON — and its row order

A CityParquet package is a **DuckDB schema** whose tables are named by the
specification's file basenames, plus a `__cityparquet` bookkeeping table. A
package starts as an empty schema: initialise it, and the first insert creates
every module table it needs and adopts the source's CRS.

> **Two traps.**
>
> 1. **Pragma expansion happens before execution, for the whole submitted
>    script.** A `CREATE SCHEMA` and a `PRAGMA` that depends on it cannot share
>    one script — the pragma is expanded against the pre-batch catalog and fails
>    with `Schema with name pkg does not exist`. Submit one statement per
>    invocation (or per `-c`), against a persistent database file, as below.
> 2. **Never name the database file after the schema.** `duckdb pkg.db` plus a
>    schema `pkg` gives `Ambiguous reference to catalog or schema "pkg"`.
>
> PRAGMA named parameters use `=`, not `:=`.

```sh
cd lib/duckdb-cityjson
D=$OUT/mut.db
F=../cityparquet-rs/tests/fixtures/delft.city.jsonl
./build/release/duckdb $D -c "CREATE SCHEMA pkg;"
./build/release/duckdb $D -c "PRAGMA cityparquet_init('pkg');"
./build/release/duckdb -noheader -list $D -c "PRAGMA insert_cityjsonseq('pkg', '$F');"
./build/release/duckdb -list $D -c "SELECT table_name, role FROM pkg.__cityparquet ORDER BY 1;"
```

```
table_name|role
building|object
```

(The insert's expanded script includes guard queries — unresolved parents, CRS
agreement, extension-prefix collisions — that raise on a violation and return
no rows otherwise; with `-noheader -list` a passing insert prints nothing.)

Consistency checks — an empty result _is_ the pass:

```sh
./build/release/duckdb -list $D -c "PRAGMA cityparquet_validate('pkg');" \
                              -c "SELECT count(*) AS findings FROM cityparquet_validation;"
```

```
check_name|severity|table_name|object_id|message
findings
0
```

Write the schema out as a package. `cityparquet_write` is a **table function**,
not a pragma (it must decide whether to emit a `geo` key at all, which SQL
cannot branch on). No `crs =>` is needed: the package adopted EPSG:7415 from
the source.

```sh
./build/release/duckdb -list $D -c "SELECT * FROM cityparquet_write('pkg', '$OUT/pkg_out');"
```

```
file|action|rows|bytes
building.parquet|written|2231|3741462
metadata.json|written|0|6762
```

The reference implementation accepts it:

```sh
../cityparquet-rs/target/release/cityparquet validate $OUT/pkg_out
```

```
warning[column.nullability] building.parquet: `id` is declared nullable; the spec makes it non-null (spec 02-object-table-schema)
…
0 error(s), 6 warning(s)
```

Zero errors. The warnings are the known DuckDB limitation: its Parquet writer
declares every column `OPTIONAL` and cannot declare `REQUIRED`, so the
validator reports a nullable _declaration_ as a warning while a null _value_ in
a non-null column would be an error.

**Rows are in Hilbert order by default**, keyed the way cityparquet-rs keys
them, so both writers order the same input the same way. Compare the two Delft
packages row by row:

```sh
duckdb -list -c "
SELECT count(*) FILTER (WHERE a.id = b.id) AS same_position, count(*) AS total
FROM (SELECT id, row_number() OVER () r FROM '$OUT/delft/building.parquet')   a
JOIN (SELECT id, row_number() OVER () r FROM '$OUT/pkg_out/building.parquet') b USING (r);"
```

```
same_position|total
2231|2231
```

`ordering => 'source'` keeps each table in its own order instead — the first
row is then the source's first feature, `NL.IMBAG.Pand.0503100000012869`,
rather than the Hilbert order's `NL.IMBAG.Pand.0503100000000010`:

```sh
./build/release/duckdb -list $D -c "SELECT * FROM cityparquet_write('pkg', '$OUT/pkg_src', ordering => 'source');"
duckdb -noheader -list -c "SELECT id FROM '$OUT/pkg_src/building.parquet' LIMIT 1;"
# NL.IMBAG.Pand.0503100000012869
cd ../..
```

### 2.5 GeoParquet `geo` footer generation

```sh
$DK -noheader -list -c "
SELECT geo IS NOT NULL FROM cityjson_geoparquet_geo('lib/cityparquet-rs/tests/fixtures/delft.city.jsonl');"
```

Expected `true`: a `geo` object for the GeoParquet-legal columns. With `COPY` it
writes a GeoParquet file straight from a scan:

```sh
$DK -list -c "
SET VARIABLE geo = (SELECT geo FROM cityjson_geoparquet_geo('lib/cityparquet-rs/tests/fixtures/delft.city.jsonl'));
COPY (SELECT * FROM read_cityjsonseq('lib/cityparquet-rs/tests/fixtures/delft.city.jsonl'))
  TO '$OUT/delft_duckdb.parquet'
  (FORMAT PARQUET, KV_METADATA {geo: getvariable('geo')});
SELECT count(*) AS n FROM read_parquet('$OUT/delft_duckdb.parquet');"
```

Expected: `2231`.

### 2.6 Appearance sidecar readers

Three table functions produce the sidecar tables directly, with ids interned
across the whole file:

```sh
F=lib/cityparquet-rs/tests/fixtures/lod3_railway.city.json
$DK -list -c "
SELECT (SELECT count(*) FROM cityjson_materials('$F'))           AS materials,
       (SELECT count(*) FROM cityjson_textures('$F'))            AS textures,
       (SELECT count(*) FROM cityjson_implicit_geometries('$F')) AS implicit_geometries;"
```

Expected: `85|34|3` — the counts cityparquet-rs writes in 1.5. Two independent
implementations interning the same file's appearance to the same cardinality is
the strongest cheap check in this document.

`read_cityjson[seq](..., appearance := 'sidecar')` is the scan-side
counterpart: it emits global sidecar ids instead of file-local ones.

```sh
$DK -list -c "
SELECT count(*) AS rows, count(material_lod3_0) AS with_material
FROM read_cityjson('$F', lod => '3', appearance := 'sidecar');"
# 121|24
```

### 2.7 CityParquet package mutation in SQL

The mutating entry points are `PRAGMA`s that _return SQL text_, which DuckDB
runs inside the caller's transaction. Continue in `lib/duckdb-cityjson` with
the database from 2.4.

**One CRS per package.** `pkg` adopted EPSG:7415 from Delft, so inserting the
CRS-less railway sample is refused rather than mixed in:

```sh
cd lib/duckdb-cityjson
./build/release/duckdb -noheader -list $D -c "PRAGMA insert_cityjsonseq('pkg', 'test/data/railway_appearance.city.jsonl');"
```

```
Invalid Input Error: insert_cityjson: the destination is {"$schema":…,"name":"Amersfoort / RD New + NAP height",…} but the source declares no CRS this writer can resolve, so its CRS is unknown -- a package states one CRS for every row it holds, and an unknown cannot be shown to be that one; give the source a metadata.referenceSystem, or insert into a package whose CRS is unknown too
```

**A multi-module package with sidecars.** One call adds a whole file, routed
**by CityGML module**, creating every table and sidecar the source needs:

```sh
R=../cityparquet-rs/tests/fixtures/lod3_railway.city.json
./build/release/duckdb $D -c "CREATE SCHEMA rail;"
./build/release/duckdb $D -c "PRAGMA cityparquet_init('rail');"
./build/release/duckdb -noheader -list $D -c "PRAGMA insert_cityjson('rail', '$R');"
./build/release/duckdb -list $D -c "SELECT table_name, role FROM rail.__cityparquet ORDER BY 1;"
```

```
table_name|role
bridge|object
building|object
city_furniture|object
generics|object
implicit_geometries|sidecar
materials|sidecar
relief|object
textures|sidecar
transportation|object
tunnel|object
vegetation|object
water_body|object
```

```sh
./build/release/duckdb -list $D -c "SELECT * FROM cityparquet_write('rail', '$OUT/rail_out');"
```

```
WARNING:
cityparquet_write: no CRS for schema 'rail' -- the package's footer carries none and none was given (crs => 'EPSG:7415'), so every file's `crs` is written as an explicit null (CRS unknown) and metadata.json declares no projection

file|action|rows|bytes
bridge.parquet|written|9|134625
building.parquet|written|59|67344
city_furniture.parquet|written|11|165051
generics.parquet|written|3|456040
relief.parquet|written|1|527636
transportation.parquet|written|10|1188117
tunnel.parquet|written|12|173393
vegetation.parquet|written|15|5536
water_body.parquet|written|1|51289
implicit_geometries.parquet|written|3|23242
materials.parquet|written|85|5564
textures.parquet|written|34|2078
metadata.json|written|0|3593
```

The same 9 module tables, 121 rows and 85/34/3 sidecars as cityparquet-rs
writes in 1.5. The warning is the tri-state rule from 1.5 on this side: the
source declares no CRS, so the package says so (`crs: null`); `crs =>`
georeferences it. The objects' `implicit_geometry` column is populated —
all 15 vegetation objects reference a template:

```sh
duckdb -list -c "SELECT count(*) FILTER (WHERE implicit_geometry IS NOT NULL) AS ig_nonnull, count(*) AS n
                 FROM '$OUT/rail_out/vegetation.parquet';"
# 15|15
../cityparquet-rs/target/release/cityparquet validate $OUT/rail_out | tail -1
# 0 error(s), 66 warning(s)        (all column.nullability, as in 2.4)
```

**Addresses.** `address_location.city.jsonl` is one real Helsinki building
whose `address` member was filled in by hand with the recognised vocabulary
(see `test/data/README.md`):

```sh
A=$OUT/addr.db
./build/release/duckdb $A -c "CREATE SCHEMA hel;"
./build/release/duckdb $A -c "PRAGMA cityparquet_init('hel');"
./build/release/duckdb -noheader -list $A -c "PRAGMA insert_cityjsonseq('hel', 'test/data/address_location.city.jsonl');"
./build/release/duckdb -list $A -c "SELECT * FROM cityparquet_write('hel', '$OUT/dk_addr');"
duckdb -list -c "SELECT id, len(address) AS n_addr, address[1].street AS street,
                        address[1].city AS city, address[1].location IS NOT NULL AS has_loc
                 FROM '$OUT/dk_addr/building.parquet' WHERE address IS NOT NULL;"
```

```
file|action|rows|bytes
building.parquet|written|1|29253
metadata.json|written|0|5923
id|n_addr|street|city|has_loc
BID_f40f0a11-71b2-4de3-9f38-e23558b2f95b|2|Mannerheimintie|Helsinki|true
```

The address survives, location included. Exporting this package with
cityparquet-rs does **not** compare equal to its source, for a reason unrelated
to the address — see Known issue #16; the reverse chain on the same fixture
(rs → duckdb-cityjson → rs) hits #17.

**Loading a cityparquet-rs package.** `cityparquet_read` loads a package
directory into a schema, footers included:

```sh
./build/release/duckdb -list -c "CREATE SCHEMA delft;" \
                       -c "PRAGMA cityparquet_read('$OUT/delft', 'delft');" \
                       -c "SELECT count(*) FROM delft.building;"
# 2231
```

**The CRS survives `cityparquet_read` → `cityparquet_write` without `crs =>`.**
A package that was read keeps its footer, and the footer's CRS is written back,
full PROJJSON included:

```sh
./build/release/duckdb -list $OUT/crscheck.db -c "
CREATE SCHEMA delft2;
PRAGMA cityparquet_read('$OUT/delft', 'delft2');
SELECT * FROM cityparquet_write('delft2', '$OUT/delft_nocrs_arg');"
```

```
WARNING:
GEOMETRY columns with coordinate reference system identifiers are not supported in storage versions prior to v1.5.0 (database "crscheck" is using storage version v1.0.0+). CRS will not be persisted.

file|action|rows|bytes
building.parquet|written|2231|3741956
metadata.json|written|0|6794
```

(The warning concerns the DuckDB database file's own storage of the
`GEOMETRY` column, not the package.) Confirm the PROJJSON in the written footer:

```sh
duckdb -noheader -list -c "SELECT decode(value) FROM parquet_kv_metadata('$OUT/delft_nocrs_arg/building.parquet') WHERE key::VARCHAR='city';" \
  | python3 -c "
import sys,json
d=json.load(sys.stdin); crs=d.get('crs')
print('crs key present:', 'crs' in d, '| type:', type(crs).__name__)
print('crs.type:', crs.get('type'), '| crs.name:', crs.get('name'), '| id:', crs.get('id'))
"
```

```
crs key present: True | type: dict
crs.type: CompoundCRS | crs.name: Amersfoort / RD New + NAP height | id: {'authority': 'EPSG', 'code': 7415}
```

`crs =>` is for georeferencing a package that has no CRS of its own; it is not
needed to carry forward a CRS the package already has. A plain
`read_parquet` + `cityparquet_init` load, by contrast, discards the footer and
with it the CRS (`docs/FUNCTIONS.md`, "CityParquet packages").

The rest of the family — `cityparquet_reconcile`, `cityparquet_delete`
(cascade), `cityparquet_merge`, `cityparquet_orphans` / `cityparquet_vacuum`,
and appending into an rs-written package (`cityparquet_append.test`, which keeps
the destination's bboxes) — is covered by `test/sql/cityparquet_*.test` in 2.1.
Each mutating pragma has a scalar `*_sql()` twin that returns the generated text
without running it, which is the fastest way to see what a call would do.

```sh
cd ../..
```

### 2.8 Metadata functions

```sh
$DK -list -c "
SELECT version, title, city_objects_count, reference_system
FROM cityjsonseq_metadata('lib/cityparquet-rs/tests/fixtures/delft.city.jsonl');"
```

```
version|title|city_objects_count|reference_system
2.0|3DBAG|2231|{'base_url': 'https://www.opengis.net/def/crs/', 'authority': EPSG, 'version': 0, 'code': 7415}
```

### 2.9 FlatCityBuf

The FCB dependency is a released vcpkg port resolved from a git registry
(`HideBa/vcpkg`, pinned baseline). `read_flatcitybuf` decodes only what the
query projects, and pushes down bbox (`min_x`/`min_y`/`max_x`/`max_y`) and
attribute `WHERE` predicates.

```sh
(cd lib/duckdb-cityjson && ./build/release/duckdb -list -c "
SELECT count(*) FROM read_flatcitybuf('test/data/fcb_bbox_attr.fcb');
SELECT count(*) FROM read_flatcitybuf('test/data/fcb_bbox_attr.fcb') WHERE height >= 10;")
```

Expected: `3` and `3`. On the real Delft encoding, the same object count both
readers report:

```sh
$DK -list -c "SELECT count(*) AS objects, count(geometry_lod2_2) AS lod22
              FROM read_flatcitybuf('lib/cityparquet-rs/tests/fixtures/delft.fcb', lod => '2.2');"
# objects|lod22
# 2231|1116
```

Network-gated remote range reads are opt-in and skipped by 2.1:

```sh
(cd lib/duckdb-cityjson && just test-fcb-remote)   # HTTP range requests, ~2.3 GB hosted 3DBAG
```

The bbox in that test is a 500 m square hard-wired to the default hosted subset
(RD New / EPSG:7415), so a different `url=` needs a matching bbox.

### 2.10 Opt-in harnesses — wasm, and the C++ kernels

None of these runs under `make test`.

- **Wasm** (~2 GB toolchain, ~10 min bootstrap, ~4 min build) — not exercised in
  this pass:

  ```sh
  (cd lib/duckdb-cityjson && just wasm-setup && just wasm && just test-wasm)
  ```

  The Node smoke harness asserts `pragma_platform() = wasm_mvp`, that the
  extension loads, and that `read_cityjson` / `read_flatcitybuf` return the
  native build's oracle values. Remote reads are XFAIL under Node (its runtime
  implements only the `NODE_FS` protocol), not a defect in the artefact.

- **C++ kernel harnesses**, from `lib/duckdb-cityjson`:

  ```sh
  P="$(pwd)/build/release/vcpkg_installed/arm64-osx"   # or a `just vendor-fcb` .vendor/prefix
  FCB_PREFIX="$P" test/cpp/run_fcb_selective_tests.sh
  test/cpp/run_face_triangulation_tests.sh
  test/cpp/run_obj_parser_tests.sh
  ```

  Expected: `All fcb selective-deserialisation assertions passed.`,
  `face_triangulation: all checks passed` and `obj_parser: all checks passed`.
  The FCB harness links `-lduckdb` against the build's `src/libduckdb.dylib`.

---

## Part 3 — duckdb-3d extension

### 3.1 SQL + C++ suites

**`make test_full` is the target to verify against.** It configures, builds,
and runs everything in one command, and **stages the `cityjson` and `spatial`
extensions itself** into a scratch `HOME`, so the `require cityjson` /
`require spatial` interop tests run instead of skipping. It needs the sibling
`lib/duckdb-cityjson` release build and network access (spatial/httpfs on the
first run, a remote Delft tile on every run):

```sh
(cd lib/duckdb-3d && make test_full)
```

Expected:

- SQL: `All tests passed (748 assertions in 37 test cases)` — **zero skips**
- C++: `All tests passed (682 assertions in 204 test cases)`

**Under `test_full`, a skip is a failure.** The recipe greps its own log for
`skipped test|were skipped` and exits non-zero if it finds one, because a skip
there means staging failed silently.

A bare `make test` (release SQL suite, no staging, no network) is the faster
loop, and the gated tests skip: `All tests passed (7 skipped tests, 550
assertions in 30 test cases)`, the skips being `require cityjson: 6` and
`require spatial: 1` — Part 4 covers that interop by hand. `make test_cpp`
runs the C++ kernel suite alone.

Rebuild before you trust a failure: `make test` runs whatever is in
`build/release`, and a stale extension fails tests that are ahead of it.

### 3.2 Hollow solid / inner shell (`shells`, `03-geometry-semantics`)

```sh
(cd lib/duckdb-3d && THREE_D_TEST_FIXTURES=1 ./build/release/test/unittest "test/sql/st_3d_hollow_solid.test")
```

Expected: `All tests passed (13 assertions in 1 test case)`.

**`THREE_D_TEST_FIXTURES=1` is required.** The `ST_AsWKB*` fixture constructors
are registered only when it is set, so without it the file reports
`Skipped tests for the following reasons: require-env THREE_D_TEST_FIXTURES: 1`.
`make test` exports it; a bare `unittest` invocation does not.

This file proves an interior shell's volume _subtracts_ (outer cube 64 − cavity
8 = 56) and that a wrongly-wound cavity is **rejected** rather than silently
added.

### 3.3 The `ST_3D*` namespace

Every name that would otherwise collide with `spatial` lives under `ST_3D*`
(`ST_3DTransform`, not `ST_Transform`). Confirm what the build registers:

```sh
./lib/duckdb-3d/build/release/duckdb -noheader -list -c "
SELECT DISTINCT function_name FROM duckdb_functions()
WHERE function_name LIKE 'st\_3d%'     ESCAPE '\'
   OR function_name LIKE 'st\_geom3d%' ESCAPE '\'
   OR function_name IN ('st_crs','st_setcrs','st_makesolid','st_force3d','st_isplanar')
ORDER BY 1;"
```

The 52 names include `st_3dtransform`, `st_3dplaceimplicit`, the CRS accessors
`st_crs` / `st_setcrs`, and `st_geom3dfromwkb` — the class-generic
constructors live under `ST_Geom3D*`, so a bare `st_3d%` filter misses them.

### 3.4 Typed constructors and the fixture gate

Constructors return the real `SOLID_3D` type, and the `ST_AsWKB*` fixture
builders exist only under `THREE_D_TEST_FIXTURES`:

```sh
THREE_D_TEST_FIXTURES=1 ./lib/duckdb-3d/build/release/duckdb -list -unsigned -c "
SELECT typeof(ST_3DFromWKB(ST_AsWKBPolyhedralTetra()))       AS typed,
       ST_3DVolume(ST_3DFromWKB(ST_AsWKBHollowCube()))       AS hollow_vol;"
```

```
typed|hollow_vol
SOLID_3D|56.0
```

Count the gated functions to see the gate itself:

```sh
./lib/duckdb-3d/build/release/duckdb -noheader -list -c \
  "SELECT count(*) FROM duckdb_functions() WHERE function_name LIKE 'st\_aswkb%' ESCAPE '\';"
# 1   -- only the real ST_AsWKB exporter
THREE_D_TEST_FIXTURES=1 ./lib/duckdb-3d/build/release/duckdb -noheader -list -c \
  "SELECT count(*) FROM duckdb_functions() WHERE function_name LIKE 'st\_aswkb%' ESCAPE '\';"
# 13  -- plus the 12 fixture builders
```

Without the variable, `ST_AsWKBPolyhedralTetra()` is a `Catalog Error`, which
is the intended behaviour and not a broken build.

Apart from those builders there is no standalone geometry smoke test —
`three_d` has no WKT constructor of its own (`ST_GeomFromText` belongs to
`spatial`), so real geometry arrives via `cityjson` or Parquet. Use Part 4.

---

## Part 4 — Cross-component integration

### 4.1 cityjson + three_d in one session

Load the cityjson extension into the three_d shell (absolute path). The STRUCT
`geometry_properties_lod*` goes straight into `ST_3DFromWKB`:

```sh
./lib/duckdb-3d/build/release/duckdb -list -unsigned -c "
LOAD '$(pwd)/lib/duckdb-cityjson/build/release/extension/cityjson/cityjson.duckdb_extension';
SELECT id,
       ST_3DNumFaces(solid)              AS faces,
       ST_3DIsClosed(solid)              AS closed,
       ROUND(ST_3DSurfaceArea(solid), 6) AS area,
       ROUND(ST_3DVolume(solid), 6)      AS vol
FROM (SELECT id, ST_3DFromWKB(geometry_lod2_2, geometry_properties_lod2_2) AS solid
      FROM read_cityjson('lib/duckdb-3d/test/data/unit_cube.city.json', lod => '2.2')
      WHERE geometry_lod2_2 IS NOT NULL);"
```

```
id|faces|closed|area|vol
cube|6|true|6.0|1.0
```

### 4.2 The `shells` contract — a cavity must subtract

```sh
./lib/duckdb-3d/build/release/duckdb -list -unsigned -c "
LOAD '$(pwd)/lib/duckdb-cityjson/build/release/extension/cityjson/cityjson.duckdb_extension';
SELECT geometry_properties_lod2_0
FROM read_cityjson('lib/duckdb-3d/test/data/hollow_solid.city.json', lod => '2');
SELECT ST_3DNumShells(s) AS shells, ST_3DIsClosed(s) AS closed, ROUND(ST_3DVolume(s),6) AS volume
FROM (SELECT ST_3DFromWKB(geometry_lod2_0, geometry_properties_lod2_0) AS s
      FROM read_cityjson('lib/duckdb-3d/test/data/hollow_solid.city.json', lod => '2')
      WHERE geometry_lod2_0 IS NOT NULL);"
```

`shells: [[6, 6]]` in the properties, and the cavity subtracted (64 − 8 = 56):

```
geometry_properties_lod2_0
{'type': Solid, 'surfaces': NULL, 'face_semantics': NULL, 'shells': [[6, 6]]}
shells|closed|volume
2|true|56.0
```

The LoD column is `*_lod2_0`: the requested `lod => '2'` normalises to the
two-part suffix. `shells` must be nested, one inner list per solid; the strict
constructor refuses the flat form (`ST_3DTryFromWKB` returns `NULL`):

```sh
THREE_D_TEST_FIXTURES=1 ./lib/duckdb-3d/build/release/duckdb -unsigned -c \
  "SELECT ST_3DFromWKB(ST_AsWKBHollowCube(), '{\"type\": \"Solid\", \"shells\": [6, 6]}');"
# Invalid Error: geometry_properties JSON: shells must be an array of per-solid arrays of shell face counts, e.g. [[12]]
```

### 4.3 cityparquet-rs package → duckdb-3d

3D analysis straight off a package written by the Rust reference
implementation, with no CityJSON in the loop:

```sh
./lib/duckdb-3d/build/release/duckdb -list -unsigned -c "
WITH solids AS (
  SELECT id, ST_3DTryFromWKB(geometry_lod2_2, geometry_properties_lod2_2) AS s
  FROM read_parquet('$OUT/delft/building.parquet')
  WHERE geometry_lod2_2 IS NOT NULL
), v AS (
  SELECT id, s, ST_3DValidationReport(s) AS r FROM solids WHERE s IS NOT NULL
)
SELECT count(*) AS parsed,
       SUM(CASE WHEN r.is_valid THEN 1 ELSE 0 END) AS valid,
       ROUND(SUM(CASE WHEN r.is_valid THEN ST_3DVolume(s) END), 1) AS vol_valid_m3
FROM v;"
```

```
parsed|valid|vol_valid_m3
1116|1098|1915862.7
```

The `geometry_properties_lod*` STRUCT is consumed directly; 1116 of 2231 rows
carry LoD2.2 (the geometry lives on the BuildingParts, not their Building
parents); and 18 real-world solids are invalid. Volumes come from an ear-clipping
triangulation of each face, so they can differ from another implementation's in
the second decimal place.

> **Required idiom.** `ST_3DTryFromWKB` + `ST_3DValidationReport`, never bare
> `ST_3DFromWKB` + `ST_3DVolume` — on real data the strict path aborts the whole
> query with `Invalid Error: ST_3DVolume: solid is not closed`.
>
> **The LoD0 footprint is a `GEOMETRY` column.** It carries the Parquet
> `GEOMETRY` logical type, and DuckDB's promotion follows the logical type
> rather than the `geo` footer, so it reads as `GEOMETRY` whether
> `enable_geoparquet_conversion` is on or off, and `geometry_lod0_0::BLOB`
> raises. Projecting `geometry_lod2_2` alone is fine, and so is `SELECT *`. To
> hand the footprint to duckdb-3d, pass it straight in — `ST_Geom3DFromWKB`
> takes `GEOMETRY` as well as `BLOB`:
>
> ```sh
> ./lib/duckdb-3d/build/release/duckdb -list -c "
> SELECT ROUND(SUM(ST_3DFootprintArea(ST_Geom3DFromWKB(geometry_lod0_0))),1) AS footprint_m2
> FROM read_parquet('$OUT/delft/building.parquet') WHERE geometry_lod0_0 IS NOT NULL;"
> # 323154.4
> ```

### 4.4 Cross-writer conformance — `just conformance`

`test/conformance.sh` checks the two writers against each other and against
the specification on both real fixtures. For each source:

```
rs     source ──convert──▶ P_rs ──validate (rs)
duckdb source ──insert_cityjson + cityparquet_write──▶ P_dk ──validate (rs)
duckdb P_rs ──cityparquet_read + cityparquet_validate + cityparquet_write──▶ P_rs_dk ──validate (rs)
```

and every package is exported by rs and `compare`d with the source, so a
package that conforms yet carries the wrong content still fails. It rebuilds
the rs CLI itself and needs a duckdb-cityjson build.

```sh
just conformance
```

```
== delft
ok   rs writes, rs validates
ok     validate P_rs
ok     P_rs round trip
ok   duckdb writes
ok     validate P_dk
ok     P_dk round trip (rs export)
ok   duckdb reads P_rs, writes again
ok     validate P_rs_dk
ok     P_rs_dk round trip (rs export)
== lod3_railway
ok   rs writes, rs validates
ok     validate P_rs
ok     P_rs round trip
ok   duckdb writes
ok     validate P_dk
ok     P_dk round trip (rs export)
ok   duckdb reads P_rs, writes again
ok     validate P_rs_dk
ok     P_rs_dk round trip (rs export)

conformance ok
```

18 checks, ~26 s. `test/run-all.sh` runs it as a stage.

### 4.5 The six-hop duckdb-cityjson ↔ cityparquet-rs round trip

CityJSON → cityparquet-rs → CityParquet → **duckdb-cityjson** → CityParquet →
cityparquet-rs → CityJSON, step by step:

```sh
mkdir -p $OUT/n
$CP convert lib/cityparquet-rs/tests/fixtures/delft.city.jsonl -o $OUT/n/rs --overwrite
$DK -list -c "
CREATE SCHEMA d;
PRAGMA cityparquet_read('$OUT/n/rs','d');
SELECT * FROM cityparquet_write('d','$OUT/n/rt');"
$CP export $OUT/n/rt $OUT/n/rt.city.jsonl
$CP compare lib/cityparquet-rs/tests/fixtures/delft.city.jsonl $OUT/n/rt.city.jsonl
echo "exit=$?"
```

```
2231 2 0 0 0 0 0 0 0 0
file|action|rows|bytes
building.parquet|written|2231|3741956
metadata.json|written|0|6789
1115 2231 0 0
equal (excluded: 20)
exit=0
```

`cityparquet_read` tolerates the `CREATE SCHEMA` in the same script (unlike
`cityparquet_init`, 2.4). No `crs =>` and no `SET
enable_geoparquet_conversion=false` are needed: the CRS rides the footer
(2.7), and `cityparquet_write` keeps a `GEOMETRY` column `GEOMETRY`-typed.

The **sidecar-bearing** railway takes the same chain — materials, textures,
implicit geometries and nine module tables through both writers:

```sh
$CP convert lib/cityparquet-rs/tests/fixtures/lod3_railway.city.json -o $OUT/n/rs_rail --overwrite
$DK -list -c "
CREATE SCHEMA d;
PRAGMA cityparquet_read('$OUT/n/rs_rail','d');
SELECT * FROM cityparquet_write('d','$OUT/n/rt_rail');"
$CP export $OUT/n/rt_rail $OUT/n/rt_rail.city.json
$CP compare lib/cityparquet-rs/tests/fixtures/lod3_railway.city.json $OUT/n/rt_rail.city.json
echo "exit=$?"
```

```
121 13 0 0 0 0 85 34 3 0
WARNING:
cityparquet_write: no CRS for schema 'd' -- the package's footer carries none … (CRS unknown) and metadata.json declares no projection
file|action|rows|bytes
bridge.parquet|written|9|135024
…
implicit_geometries.parquet|written|3|23246
materials.parquet|written|85|5613
textures.parquet|written|34|2079
metadata.json|written|0|3619
38 121 0 0
equal (excluded: 26)
exit=0
```

The warning restates the source's own state: its `crs` is `null` going in and
`null` coming out.

**Degenerate rings.** A ring with fewer than three vertices cannot form a
closed WKB ring. Both writers drop it on write, and duckdb-cityjson says so.
`degenerate_rings_appearance.city.jsonl` is `railway_appearance.city.jsonl`
with three real rings cut to two vertices by hand (`test/data/README.md`):

```sh
cd lib/duckdb-cityjson
G=$OUT/degenerate.db
./build/release/duckdb $G -c "CREATE SCHEMA rings;"
./build/release/duckdb $G -c "PRAGMA cityparquet_init('rings');"
./build/release/duckdb -noheader -list $G -c "PRAGMA insert_cityjsonseq('rings', 'test/data/degenerate_rings_appearance.city.jsonl');"
```

```
WARNING:
cityjson: object 'GMLID_BUI100628_817_8083' (MultiSurface, lod 3.0): dropped 1 ring(s) with fewer than three vertices, which cannot form a closed WKB ring, and 1 surface(s) whose exterior ring was one of them

WARNING:
cityjson: object 'GMLID_855011_330784_753' (MultiSurface, lod 3.0): dropped 2 ring(s) with fewer than three vertices, which cannot form a closed WKB ring, and 1 surface(s) whose exterior ring was one of them
```

The package it writes reads back through cityparquet-rs, and agrees with what
cityparquet-rs makes of the same source (which drops the same 3 rings and 2
surfaces — the fifth and sixth report fields — and, under
`--tolerate-invalid-appearance`, the one dangling material reference the source
also carries):

```sh
./build/release/duckdb -list $G -c "SELECT * FROM cityparquet_write('rings', '$OUT/dk_rings');"
CPR=../cityparquet-rs/target/release/cityparquet
$CPR export $OUT/dk_rings $OUT/dk_rings.city.jsonl; echo "exit=$?"
$CPR convert test/data/degenerate_rings_appearance.city.jsonl -o $OUT/rs_rings \
    --tolerate-invalid-appearance --overwrite
$CPR export $OUT/rs_rings $OUT/rs_rings.city.jsonl
$CPR compare $OUT/rs_rings.city.jsonl $OUT/dk_rings.city.jsonl; echo "exit=$?"
cd ../..
```

```
WARNING:
cityparquet_write: no CRS for schema 'rings' -- … (CRS unknown) and metadata.json declares no projection

file|action|rows|bytes
bridge.parquet|written|1|9728
city_furniture.parquet|written|1|10392
implicit_geometries.parquet|written|3|23136
materials.parquet|written|4|1774
textures.parquet|written|4|1337
metadata.json|written|0|1759
2 2 0 0
exit=0
warning: source carries a CRS-bearing coordinate … (as in 1.5)
warning: 1 invalid material/texture reference(s) dropped (--tolerate-invalid-appearance): a dangling index, or a texture ring with fewer UVs than vertices, left untextured
2 6 0 0 3 2 3 4 3 1
2 2 0 0
equal (excluded: 4)
exit=0
```

### 4.6 Implicit geometries — `ST_3DPlaceImplicit` over the sidecar join

An object's `implicit_geometry` struct carries the template `id`, the reference
`point` (WKB `Point Z`) and the row-major 4×4 `transformationMatrix`; the
template geometry is a row of `implicit_geometries.parquet`. `ST_3DPlaceImplicit`
materialises one instance at `M · v + p`. On cityparquet-rs's railway package
from 1.5:

```sh
./lib/duckdb-3d/build/release/duckdb -list -unsigned -c "
SELECT o.id, t.id AS template,
       ST_3DAsText(ST_3DCentroid(ST_3DPlaceImplicit(
         ST_Geom3DFromWKB(t.geometry_lod3_0),
         ST_Geom3DFromWKB(o.implicit_geometry.point),
         o.implicit_geometry.transformationMatrix))) AS placed_centroid
FROM '$OUT/railway/vegetation.parquet' o
JOIN '$OUT/railway/implicit_geometries.parquet' t ON t.id = o.implicit_geometry.id
ORDER BY o.id LIMIT 3;"
```

```
id|template|placed_centroid
GMLID_SO0107241_3793_12555|1|POINT Z (1.15844898 6.6314892 9.0743353)
GMLID_SO0124800_3522_13577|0|POINT Z (0.828664787 7.54983942 9.32059222)
GMLID_SO015374_872_14131|0|POINT Z (0.803664787 6.93783942 9.16959222)
```

All 15 vegetation objects place, over the 3 templates. The same query over
duckdb-cityjson's package of the same source (`$OUT/rail_out`, 2.7) returns
the same three centroids, digit for digit. The railway's matrices are all the
identity; a `NULL` matrix also means identity, and a singular matrix raises for
a `SOLID_3D` (`lib/duckdb-3d/docs/FUNCTIONS.md`).

---

## Part 5 — Benchmarks (procedure only)

The benchmark recipes are inspected and the corpus state is checked here; the
multi-hour runs are not part of this walkthrough.

### 5.0 State of the corpus and the results tree

**Check it, do not read it from here.** Two commands answer the question at the
moment you ask it:

```sh
ls benchmark/runs/data benchmark/runs/formats/results 2>&1
git log --oneline -3 -- benchmark/runs/formats/results
```

No results are committed until the full run on the benchmark host, so on a
fresh clone the results directory is absent. Two methodology documents state
what a run means: `benchmark/formats/READ_BENCHMARK.md` (the cross-format read
benchmark and its fairness caveats) and `benchmark/formats/README.md` (file
sizes and the configuration benchmark).

Two things worth knowing before a run, because neither is visible from a
directory listing:

- `benchmark/runs/data/readbench/` prepared artefacts carry the version of the
  conversion chain that built them; a stale stamp makes `readbench_prepare.sh`
  refuse the dataset and print the exact `rm -rf` that clears it. Delete the
  tree if in doubt: `just bench-prep` downloads the prepared corpus again from
  the hosted `v<chain>/` folder and verifies every file against its manifest
  (`benchmark/README.md`, "The hosted corpus"), so nothing is converted. The
  six city datasets are hosted; the 3DBAG slice is added once by
  `just bench-prep --rebuild-sources --datasets 3dbag_n1000000` on a machine
  with the R2 token and enough memory. `--no-cache` rebuilds every artefact
  but the CityJSON and the CityGML with the current code and uploads only what
  differs; `--local` builds everything here without bucket access.
- Packages of different writer generations must not be mixed: regenerate
  rather than reuse a package written before a format change (the tri-state
  `city.crs`, the JSON logical type).

### 5.1 Corpus eligibility — read this before running anything

The corpus is the six city datasets — `rotterdam_delfshaven`,
`vienna_102081`, `nyc_da13_buildings`, `zurich_building_lod2`, `tokyo`,
`montreal` — and the 3DBAG slice `3dbag_n1000000`. `benchmark/manifest.toml`
lists them, and `benchmark/formats/README.md` (§ The corpus) gives each one's
source and query predicates. `just bench-prep` fetches the city datasets into
`benchmark/runs/data/benchmark/` and cuts the slice into
`benchmark/runs/data/3dbag/`.

Every one converts. A dataset that declares no `referenceSystem` still
converts: it gets `city.crs: null` and a stderr warning, and its package is not
georeferenced (no `proj:*` in its STAC Item, no WGS84 extent). Pass `--crs` per
dataset if a run needs georeferenced output; no bench script passes it for you.

The read-bench runner rejects a multi-module package (Known issues #8). Every
corpus dataset is single-module, so this blocks only an input you add yourself
— `lod3_railway`, for instance.

Verify a source's declaration before blaming the writer:

```sh
python3 - <<'EOF'
import json, glob, os
for f in sorted(glob.glob('benchmark/runs/data/benchmark/*.json') +
                glob.glob('benchmark/runs/data/3dbag/*.jsonl')):
    with open(f) as fh:
        d = json.loads(fh.readline()) if f.endswith('.jsonl') else json.load(fh)
    print('%-28s %s' % (os.path.basename(f),
          d.get('metadata', {}).get('referenceSystem') or 'ABSENT'))
EOF
```

### 5.2 Regenerate all CityParquet packages

```sh
rm -rf out/cityparquet
just convert-all benchmark/runs/data/benchmark out/cityparquet
```

One package directory per input, each in Hilbert order, the writer's default.
Run it over `benchmark/runs/data/3dbag` as well for the 1M-object slice. Hilbert
order holds a CityGML or FlatCityBuf input parsed in memory (a CityJSON or
CityJSONSeq input is read back in that order instead); `--ordering source`
streams any input one feature at a time.

### 5.3 Read benchmark

```sh
rm -rf benchmark/runs/data/readbench          # only if you want a clean prepare
just bench-prep --families formats
just bench-run --families formats
```

`readbench_prepare.sh` **skips any package directory that already exists**, so
delete first whenever the format has moved.

`--transport local|http` (default `local`) plus `--base-url` makes every format
read over HTTP instead of from a local file, and populates the trailing
`bytes_read` / `http_requests` CSV columns (empty for every local row). Upload
steps and methodology are in `benchmark/formats/READ_BENCHMARK.md`.

The **network family** runs the same read benchmark over HTTP against a local
server that simulates a network profile (`fast` 1,000 Mbps / 5 ms, `typical`
100 Mbps / 20 ms, `slow` 20 Mbps / 50 ms):

```sh
just bench-run --families network --datasets rotterdam --network-profile typical \
  --network-repeat 2 --numa-node off --max-load off   # macOS: no NUMA, no load gate
```

Check the log for `cross-format consistency OK` per dataset, and that each
`runs/network/full/<profile>/<dataset>.csv.params.json` carries server and
client totals that agree (`network.server` against `network.clients`).

The `CityParquetRunner` supports only single-table packages, so a multi-module
input such as `lod3_railway` produces no read numbers (Known issues #8).

### 5.4 Aggregate results into one page

```sh
just bench-summary
```

The renderer (`benchmark/plot/benchviz`) reads what the runs above left under
`benchmark/runs/` and writes `benchmark/runs/summary/<profile>/` —
`bench_data.json`, `completeness.json`, a self-contained `index.html`, and the
static figures. It measures nothing, so re-running it is free. Read the
messages it prints: the figures step can refuse (existing print-sheet captions
are tied to the corpus they were drawn from) while still writing the HTML page.

The **paper** repository runs the same recipe against this repository as a
submodule, with `--figures` pointing the figure output at
`paper/assets/bench/`.

---

## Part 6 — Every automated suite: `test/run-all.sh`

```sh
./test/run-all.sh           # or: just test-all
./test/run-all.sh --list    # what would run, and why anything is skipped
```

One stage per suite, in dependency order. A stage whose toolchain is absent is
**skipped with its reason**, not failed; only a suite that ran and failed makes
the script exit non-zero. Result of this pass:

```
PASS cityparquet-rs: just check                          836 passed, 6 skipped (nextest); isolation ok
PASS cityparquet-rs: just interop                        interop bloom cross-writer ok; interop ok
PASS benchmark: just plot-test                           75 passed
PASS benchmark: just scripts-test                        67 ok
PASS benchmark/databases: pytest (unit)                  511 passed, 1 skipped, 43 deselected
PASS benchmark/readbench: cargo test                     34 targets: 240 passed, 0 failed, 1 ignored
PASS scripts/catalog2cityparquet: just catalog-test      301 passed, 11 skipped
PASS duckdb-cityjson: unittest                           2243 assertions in 80 test cases (5 skipped)
PASS cross-writer conformance (rs <-> duckdb-cityjson)   conformance ok
PASS duckdb-3d: make test                                550 assertions in 30 test cases (7 skipped)
PASS citylake: cargo build
================================================================
passed  11
failed  0
skipped 0

Everything that could run, ran.
```

(Each `PASS` line is the script's; the counts beside it are taken from that
stage's own log.) 9 min 08 s wall-clock on this machine. The one `ignored`
readbench test is Known issue #8; the skips inside a passing stage are
network- or extension-gated tests (2.1, 3.1) and the catalogue driver's
live-network cases.

---

## Known issues

Issue numbers are stable across passes; a closed issue keeps its number.

### Open

| #   | Issue                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      | Status                                                                                                                                                                                                                                                                                                                                                                           |
| --- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 8   | **The read-bench runner rejects multi-table packages.** `benchmark/readbench/src/formats/cityparquet.rs` queries a single Parquet file, so a multi-module package (`lod3_railway`) has no read numbers                                                                                                                                                                                                                                                                                     | One test `#[ignore]`d (`benchmark/readbench/tests/attr_consistency.rs`). Every corpus dataset is single-module (5.1)                                                                                                                                                                                                                                                             |
| 15  | **duckdb-cityjson's pre-commit format gate skips.** Local `clang-format` is 21.1.6 and CI pins 11.0.1; a different version reformats conforming code, so the hook skips the format phase by design. `clang-tidy` (22.1.8 here) is on `PATH`, so the tidy phase runs                                                                                                                                                                                                                        | Local tooling, not something a commit can fix: install `clang_format==11.0.1` to format-check locally; CI is the format gate otherwise                                                                                                                                                                                                                                           |
| 16  | **duckdb-cityjson writes an object-valued attribute as plain `UTF8`, not Parquet `JSON`.** `insert_cityjson[seq]` reads an object attribute (`Integrate_LoD[1]` in the Helsinki building) as `VARCHAR`, and `cityparquet_write` emits it without the `JSON` annotation, so cityparquet-rs restores a string where the source had an object. `02-object-table-schema` maps "object or heterogeneous array" to `JSON`; cityparquet-rs's own package carries `JsonType()` for the same column | Library defect, reported. Repro in 2.7: write `dk_addr`, then `cityparquet export $OUT/dk_addr …` and `compare` with `test/data/address_location.city.jsonl` → exit 2, `"Integrate_LoD[1]":{…}` vs `"Integrate_LoD[1]":"{…}"`; `parquet_schema` shows `UTF8` (duckdb) vs `JsonType()` (rs)                                                                                       |
| 17  | **An attribute named `ID` next to the reserved `id` diverges between the writers.** duckdb-cityjson treats the case-insensitive collision as a collision and puts `ID` in `other`; cityparquet-rs writes a separate `ID` column. DuckDB identifiers are case-insensitive, so `cityparquet_read` of the rs package renames it `ID_1`, and the six-hop chain on the Helsinki building returns `"ID_1":767157.0` for `"ID":767157.0` (`compare` exit 2)                                       | Specification decision: `02-object-table-schema`'s reserved-name collision rule does not say whether it is case-sensitive. Either rs diverts case-insensitive collisions too, or a reader must restore the name. Repro: `cityparquet convert test/data/address_location.city.jsonl -o rs_addr`, `cityparquet_read` + `cityparquet_write` it, `export`, `compare` with the source |
| 18  | **`cityparquet collection` declares `item_assets.data`**, an asset no CityParquet Item carries                                                                                                                                                                                                                                                                                                                                                                                             | Upstream: `city3d-stac-gen`'s Collection builder adds it. Seen in 1.11                                                                                                                                                                                                                                                                                                           |

### Closed

| #   | Issue                                                                                                   | Evidence in this pass                                                                                                                                                  |
| --- | ------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 2   | `implicit_geometries.id` was `BIGINT` in duckdb-cityjson, `VARCHAR` in cityparquet-rs                   | Both write `BIGINT` (1.5, 4.6); the sidecar join and the six-hop railway chain work (4.5, 4.6)                                                                         |
| 3   | `make release` did not link flatcitybuf into `src/libduckdb.dylib`                                      | The FCB C++ harness links `-lduckdb` and passes (2.10)                                                                                                                 |
| 4   | The `test/cpp` harnesses could not run                                                                  | All three pass (2.10)                                                                                                                                                  |
| 5   | `just interop` passed the removed `--profile` flag                                                      | `interop ok` (1.7)                                                                                                                                                     |
| 6   | `vendor-check` and the CityGML fixtures were undocumented prerequisites                                 | 0.1 / 0.2; `test/run-all.sh` names both in its skip reasons                                                                                                            |
| 7   | `Railway.city.jsonl` fails conversion on a dangling material reference                                  | `--tolerate-invalid-appearance` drops and counts it (4.5: tenth report field `1`); strict stays the default                                                            |
| 9   | LoD0 synthesis broke a naive round-trip `compare`                                                       | Synthesis is opt-in (`--lod0`); the default convert round-trips (1.6)                                                                                                  |
| 10  | `cityparquet-rs/CLAUDE.md` documented `convert INPUT OUTPUT_DIR` positionally                           | Documents `-o/--output` and the current flags                                                                                                                          |
| 11  | The bench justfile and scripts read the removed `manifest['tables']` key                                | Table lookups go through `benchmark/scripts/package_tables.py`                                                                                                         |
| 12  | Prepared read-bench artefacts carried the pre-by-type manifest                                          | Artefacts carry a chain-version stamp, and a stale one is refused (5.0)                                                                                                |
| 13  | A duckdb-cityjson package with a degenerate (<3-vertex) ring failed `cityparquet export`                | duckdb-cityjson drops such rings with a warning; its package exports (`2 2 0 0`, exit 0) and compares equal with cityparquet-rs's own package of the same source (4.5) |
| 14  | `just check` could not pass: readbench's `attr_consistency` called `fcb -i/-o`, which `fcb` 0.7.8 lacks | readbench is outside the library's gate (1.1) and calls `fcb ser` positionally; its suite passes (Part 6)                                                              |

---

## Summary

Every runnable step of Parts 0–4 runs on real data at the commits above, and
`test/run-all.sh` passes all 11 stages with none skipped.

- **cityparquet-rs**: 836 tests pass; Delft (2231 objects), the nine-module
  railway with its 85/34/3 sidecars, FlatCityBuf Delft and the SIG3D LoD4
  building convert, `validate` with 0 errors and 0 warnings, and round-trip to
  `equal` (`excluded: 20` for Delft, `26` for railway).
- **duckdb-cityjson**: 2243 assertions in 80 cases; packages built from an
  empty schema pass rs `validate` with 0 errors (nullability declarations are
  warnings), share rs's Hilbert row order row for row, and carry addresses and
  implicit geometries.
- **duckdb-3d**: `make test_full` runs 748 SQL + 682 C++ assertions with zero
  skips; 1098 of Delft's 1116 LoD2.2 solids are valid (1 915 862.7 m³), and
  `ST_3DPlaceImplicit` places the railway's 15 implicit geometries identically
  from either writer's package.
- **Across modules**: `just conformance` is green (18 checks), and the six-hop
  rs ↔ duckdb-cityjson chain is `equal` on Delft and on the sidecar-bearing
  railway. Both writers drop a degenerate ring alike (Known issue #13).

Open: an object-valued attribute loses its `JSON` annotation in
duckdb-cityjson (#16), a case-insensitive `ID`/`id` collision the
specification does not settle (#17), the read-bench multi-table limit (#8),
an upstream `item_assets` artefact (#18), and the local clang-format version
(#15).
