# CityParquet stack — manual test guide

A step-by-step walkthrough for verifying the three implementations: the Rust
reference library (`lib/cityparquet-rs`, in-tree) and the two DuckDB extensions
(`lib/duckdb-cityjson`, `lib/duckdb-3d`, both submodules).

Every command below was **executed on 2026-08-20** (macOS arm64, DuckDB v1.5.4,
Rust 1.93.1) against these exact commits, and the expected outputs are the
**real** observed values, not illustrations:

| Submodule | Commit |
|---|---|
| `cityparquet-rs` | `571b24f` |
| `duckdb-cityjson` | `3c84395` |
| `duckdb-3d` | `5c25f21` |

All three are `origin/develop` as recorded by the parent repo's `develop`, and
every fix this guide once carried as an uncommitted working-tree patch is now
committed and pushed. A cross-repo campaign closed the duckdb-cityjson →
cityparquet-rs interop gap that §4.5 used to mark **BROKEN** — that direction is
now this document's strongest check. `documents/` (the specification, in the
parent repo) also moved, to `d7b373c`, gaining four sections that this pass's
findings settle against.

Where a step currently fails, it is marked **BROKEN** with the
cause and a working substitute. Part 5 (benchmarks) is the one exception: it is
**procedure-only** — the recipes were inspected and the corpus state checked,
but the multi-hour runs were not re-executed today.

Run everything from the repo root unless stated otherwise:

```sh
cd cityparquet          # wherever you cloned github.com/cityjson/cityparquet
```

> **Since the monorepo migration (2026-08-23) the layout this pass was written
> against has moved.** `cityparquet-rs`, `citylake` and the two DuckDB
> extensions are now under `lib/`; the benchmark corpora, results and plotting
> project are under `benchmark/`; and every recipe that reaches both halves of
> the benchmark harness — `bench`, `convert-all`, `write-bench`,
> `compression-bench`, the fetchers, the renderers, `plot-test`,
> `scripts-test` — is in the **root** `justfile` rather than
> `lib/cityparquet-rs/justfile`. The commands below are updated to match. The
> findings, commit references and dates are the record of the pass and are not.

> ### What changed since the 2026-07-23 pass
>
> This guide was rewritten from scratch. The previous version described the
> stack as of 2026-07-23 and is stale in almost every part. The breaking
> changes, in the order you will hit them:
>
> | Change | Where | Effect on the old guide |
> |---|---|---|
> | **Object tables are split per CityGML module**, not per 1st-level family | cityparquet-rs `25d471b` (2026-07-23) | `railway` now writes `transportation.parquet`, `city_furniture.parquet`, `relief.parquet`, … — not `railway.parquet`, `cityfurniture.parquet`, `tinrelief.parquet`. `object_type` carries CityGML CM class names (`GenericOccupiedSpace`, `BridgeConstructiveElement`) |
> | **`--profile` is gone** | cityparquet-rs `25d471b` | Sidecars are written whenever the source has that content; `--profile compatibility` is now an unknown-flag error |
> | **Every LoD is a suffixed column, LoD0 included** | cityparquet-rs `197e351` (2026-07-23) | There is no un-suffixed `geometry` column any more — it is `geometry_lod0_0`. Same in duckdb-cityjson's `lod =>` mode |
> | **`geometry_properties` is a STRUCT**, not JSON text | duckdb-cityjson `d334b26` (2026-07-25) | `ST_3DFromWKB` consumes it directly via the new `(BLOB, ANY)` overload; no `to_json(...)` |
> | **CityParquet package mutation in SQL** | duckdb-cityjson, 2026-07-25→27 | `cityparquet_init/validate/reconcile/delete/merge/read/write`, `insert_cityjson[seq]` — Part 2.7 |
> | **Appearance sidecar readers** | duckdb-cityjson, 2026-07-26 | `cityjson_materials/textures/geometry_templates` — Part 2.6 |
> | **flatcitybuf is a vcpkg registry dependency, bumped to `cpp-v0.9.0`** + a wasm target | duckdb-cityjson, 2026-08-14 | New build prerequisites — Part 0 |
> | **`ST_Transform` → `ST_3DTransform`** | duckdb-3d `a6b1f1d` (2026-07-24) | Generic `ST_*` names that collided with `spatial` moved into the `ST_3D*` namespace |
> | **STAC catalogue → CityParquet driver** | cityparquet-rs, 2026-08-11/13 | New Python tool with its own suite, `just catalog-test` — Part 1.10 |
> | **`city.crs` is tri-state — object, `null`, or absent** | cityparquet-rs `0c9c917` + duckdb-cityjson `4b54c6b` (2026-08-15) | **An unresolvable CRS is declared, not fatal.** A CRS-less source now converts, writing `city.crs: null` and warning on stderr. `--crs` remains, as the way to *georeference* such a source — Parts 1.5 and 5 |
> | **duckdb-3d constructors return `SOLID_3D`**, not `BLOB` | duckdb-3d `d9a8faa` (2026-08-15) | `typeof(ST_3DFromWKB(…))` is now a real type; `st_aswkb*` test fixtures moved behind `THREE_D_TEST_FIXTURES` — Part 3 |

> ### What changed since the 2026-08-16 pass
>
> A cross-repo campaign closed the duckdb-cityjson → cityparquet-rs interop gap
> that dogged the previous pass. What actually moved:
>
> | Change | Where | Effect on the old guide |
> |---|---|---|
> | **CityJSON → rs → CityParquet → duckdb-cityjson → CityParquet → rs → CityJSON is now semantically lossless** | cityparquet-rs `571b24f`, duckdb-cityjson develop | §4.5 goes from **BROKEN** to the strongest check in the document — Part 4.5 |
> | **The convert report gained a tenth field**, `invalid_appearance_refs_dropped` | cityparquet-rs `571b24f` | Every report line in Parts 1 and 2 grew a trailing column — 1.2, 1.5, 1.9, 2.7 |
> | **`--tolerate-invalid-appearance`** drops a dangling appearance reference instead of failing the whole conversion | cityparquet-rs | Settles Known issue #7 (Railway's dangling material reference); strict stays the default — 4.5 |
> | **`geometry_templates.id` divergence is settled** | spec `documents/` `d7b373c` region, duckdb-cityjson | Closes former Known issue #2 |
> | **Reserved object-table columns now emitted in the spec's normative order**, with `address`/`template` present but always NULL | duckdb-cityjson | Re-run in 2.7 |
> | **CRS survives `cityparquet_read` → `cityparquet_write`** without passing `crs =>` | duckdb-cityjson | The old §2.7 CRS note asserted the opposite; corrected below |
> | **A fresh read + ordering benchmark run, and the benchviz pipeline** | cityparquet-rs `01b719d` (2026-08-17, Linux/EPYC), `c87aaa9` | Part 5 — `just plot-pretty` replaces the old direct script invocation |
>
> One divergence remains open and is **not** part of this closure: a
> sidecar-bearing package still fails rs export on a degenerate ring that
> duckdb-cityjson writes through and rs's reader rejects — a geometry-validity
> policy question, documented honestly in 4.5.

---

## Part 0 — Prerequisites

```sh
for c in duckdb fcb uv just cargo cmake ninja python3; do
  printf "%-8s " "$c"; command -v $c || echo MISSING
done
duckdb --version   # expect v1.5.4
```

All must resolve. `fcb` is only needed for the read benchmark, `uv` for the
benchmark charts **and for the catalogue driver's test suite**.

### 0.1 Check out cityparquet-rs's vendored submodule — new, and `just check` fails without it

`lib/cityparquet-rs` vendors `city3d-stac-tool` under `vendor/`, and `just
check` gates on it (`vendor-check`).

```sh
(cd lib/cityparquet-rs && git submodule update --init)
```

### 0.2 Refresh the CityJSON/CityGML fixtures — new fixtures landed 2026-08-11

Three fixtures were added after the last pass: two real CityGML files
(`berlin_citygml1.gml`, for the "unsupported CityGML version" error path, and
`freiburg_no_preamble_srs.gml`, whose CRS is declared only *inside* city
objects) plus the synthetic `empty.city.jsonl`. Without them `just check` fails
with `fixture must exist; run 'just fixtures'`.

```sh
(cd lib/cityparquet-rs && just fixtures)
ls lib/cityparquet-rs/tests/fixtures/
# b1_lod2_cs_w_sem.gml  b1_lod2_s.gml  berlin_citygml1.gml  delft.city.jsonl
# empty.city.jsonl  freiburg_no_preamble_srs.gml  lod3_railway.city.json
```

`freiburg_no_preamble_srs.gml` is fetched as a **400 kB HTTP range request**
against a 1.86 GiB source — a full download is neither needed nor attempted.

### 0.3 Put vcpkg on the pinned baseline commit — **BROKEN** out of the box

Both DuckDB extensions pin `builtin-baseline`
`84bab45d415d22042bd0b9081aea57f362da3f35` (2025-12-13). An older vcpkg
checkout fails in **two** distinct ways, and you will hit them one after the
other:

**(a) The baseline commit is not present.** duckdb-cityjson dies at configure:

```
fatal: path 'versions/baseline.json' exists on disk, but not in '84bab45d…'
while loading baseline version for openssl
-- Running vcpkg install - failed
```

**(b) The version *database* is older than the baseline.** duckdb-3d asks for
`proj` and the baseline pins 9.7.1, which an older checkout's `versions/` does
not list:

```
error: no version database entry for proj at 9.7.1.
Available versions:
  9.7.0
  9.6.2
  …
```

Fetching the commit alone fixes (a) but not (b) — vcpkg reads `baseline.json`
from the baseline commit but resolves version files from the **working tree**.
Put the tree on that commit:

```sh
git -C "${VCPKG_ROOT:-$HOME/vcpkg}" fetch --depth 1 origin \
    84bab45d415d22042bd0b9081aea57f362da3f35
git -C "${VCPKG_ROOT:-$HOME/vcpkg}" checkout --detach \
    84bab45d415d22042bd0b9081aea57f362da3f35
```

Restore your previous state afterwards with
`git -C "${VCPKG_ROOT:-$HOME/vcpkg}" checkout master` (or your branch).

> Note that a `git pull --ff-only` may refuse here: a typical vcpkg checkout is
> **shallow**, so its grafted history makes the pinned commit look unrelated to
> local `master`. Detaching onto the fetched commit sidesteps that.
>
> Expect the first duckdb-3d configure after this to be slow — vcpkg builds
> `proj` and its dependencies from source.

### 0.4 Build all three components

```sh
# 1. cityparquet-rs (Rust) — ~25 s incremental
(cd lib/cityparquet-rs && cargo build --release -p cityparquet-cli)

# 2. duckdb-cityjson — use `just rebuild`, NOT `just build` / `make release`
(cd lib/duckdb-cityjson && just rebuild)

# 3. duckdb-3d
(cd lib/duckdb-3d && just build)
```

> **FIXED 2026-08-16 (was BROKEN).** `make release` used to die linking
> `src/libduckdb.dylib` with undefined `fcb::Feature::*` and `typeinfo for
> fcb::RangeReader`: DuckDB's `duckdb` SHARED target links only
> `${DUCKDB_SYSTEM_LIBS}`, so the flatcitybuf archive behind the statically
> embedded cityjson extension never reached the final link. The repo already
> carried a deferred fixup for exactly this on `unittest`; it now covers the
> `duckdb` shared target too (`cityjson_fcb_link_upstream_targets` in
> `CMakeLists.txt`). `just rebuild` remains the fast inner-loop command.

Artefacts you will reference later:

| Component | Path |
|---|---|
| `cityparquet` CLI | `lib/cityparquet-rs/target/release/cityparquet` |
| cityjson extension | `lib/duckdb-cityjson/build/release/extension/cityjson/cityjson.duckdb_extension` |
| three_d extension | `lib/duckdb-3d/build/release/extension/three_d/three_d.duckdb_extension` |
| DuckDB shell w/ three_d preloaded | `lib/duckdb-3d/build/release/duckdb` |
| DuckDB shell w/ cityjson preloaded | `lib/duckdb-cityjson/build/release/duckdb` |

> **Note.** Each submodule's `build/release/duckdb` shell has *its own*
> extension statically preloaded. To use both together, load the other one's
> `.duckdb_extension` file explicitly — see Part 4. You do **not** need
> `INSTALL cityjson FROM community`.

---

## Part 1 — cityparquet-rs: write CityParquet

```sh
export CP=lib/cityparquet-rs/target/release/cityparquet
export OUT=/tmp/cp_test && rm -rf $OUT && mkdir -p $OUT
```

### 1.1 Unit + integration suite

`just check: lint test isolation vendor-check` — then `cargo fmt --all --check`
and a prettier pass over the Markdown, in that order.

> **`just check` cannot pass on this machine — environmental, not a code
> defect.** `test` runs second, and `benchmark/readbench`'s
> `attr_consistency` test shells out to the external `fcb` binary with `-i`/
> `-o`. The installed **`fcb` 0.7.8** takes positional `<INPUT>... <OUTPUT>` and
> has no `-i`/`-o` flags at all, so the test fails identically on an unmodified
> base commit. Because `test` runs before `isolation`, `vendor-check` and the
> `fmt`/prettier steps, that one failure means **the later gates never run** —
> `just check` does not get far enough to tell you whether they would pass. Run
> them individually instead:
>
> ```sh
> (cd lib/cityparquet-rs && just lint)                    # exit 0
> (cd lib/cityparquet-rs && just isolation)                # exit 0
> (cd lib/cityparquet-rs && cargo fmt --all --check)       # exit 0
> (cd lib/cityparquet-rs && just vendor-check)             # exit 0
> ```
>
> All four verified green on this machine today. `vendor-check` runs
> `fmt`/`clippy`/`test` over `vendor/city3d-stac-tool` as well, because that
> submodule is deliberately outside the Cargo workspace and would otherwise
> never be verified. It fails loudly if the submodule is not checked out (see
> 0.1).

For the test suite itself, exclude the affected crate — this is the working
substitute, not a code change:

```sh
(cd lib/cityparquet-rs && cargo test --workspace --exclude cityparquet-readbench)
```

Expected: **43 targets, 713 passed, 0 failed, 0 ignored**
(~8 min, debug profile — several tests convert the whole Delft fixture).

The aggregate is easier to read than the 43 per-target lines:

```sh
(cd lib/cityparquet-rs && cargo test --workspace --exclude cityparquet-readbench 2>&1 \
  | grep -E '^test result:' \
  | awk '{p+=$4; f+=$6; i+=$8; n++} END {print "targets:",n,"passed:",p,"failed:",f,"ignored:",i}')
# targets: 43 passed: 713 failed: 0 ignored: 0
```

### 1.2 Convert a single-module dataset (Delft, 2231 Buildings)

```sh
$CP convert lib/cityparquet-rs/tests/fixtures/delft.city.jsonl -o $OUT/delft --overwrite
ls $OUT/delft
```

Expected report (space-separated, now **ten** fields — a trailing
`invalid_appearance_refs_dropped` joined the nine below in `571b24f`:
`object_count files_count skipped_same_lod_geometries
attribute_coercion_nulls degenerate_rings_dropped degenerate_surfaces_dropped
materials_written textures_written templates_written
invalid_appearance_refs_dropped`):

```
2231 2 0 0 0 0 0 0 0 0
```

Expected files — **by-module layout**, one table per CityGML module:

```
building.parquet
metadata.json
```

### 1.3 Verify the package is plain-Parquet readable

```sh
duckdb -c "SELECT count(*) FROM '$OUT/delft/building.parquet';"          # 2231
duckdb -c "DESCRIBE SELECT * FROM read_parquet('$OUT/delft/building.parquet');"
```

The schema should show the current column set, in this order:

- `id`, `feature_id`, `object_type`, `parents`, `children`, `children_roles`
- `address` — `STRUCT(street, house_number, po_box, zip_code, city, country, …)`
- `bbox` — `STRUCT(xmin, ymin, zmin, xmax, ymax, zmax)`
- one **quad per LoD**, LoD0 included and suffixed like every other:
  `geometry_lod0_0`, `geometry_properties_lod0_0`, `material_lod0_0`,
  `texture_lod0_0` — and the same for `lod1_2`, `lod1_3`, `lod2_2`
- `template`, `other`, then the flattened typed attribute columns (`b3_*`)

Two things to check specifically:

- **`geometry_lod0_0` is the only GeoParquet-legal column**, and DuckDB reports
  it as `GEOMETRY`; every higher LoD is a plain `BLOB`. That is now the Parquet
  `GEOMETRY` logical type doing the work, not the `geo` footer: the writer
  annotates a column exactly when it declares it, so the two can never disagree
  about which columns a GeoParquet reader may touch. The annotation carries the
  CRS as `EPSG:7415` — the short authority:code form the Parquet convention
  wants — while the full PROJJSON stays in `geo`, which is why the type no
  longer prints an inline CompoundCRS.

  A `PolyhedralSurface Z` is deliberately left unannotated. It is not merely
  illegal GeoParquet: DuckDB promotes any annotated column and converts it
  eagerly, and its geometry model has no PolyhedralSurface, so annotating a
  solid column would make even `SELECT count(*)` over it fail — before any
  `ST_3D*` function sees a value, and past what `ST_AsWKB` could rescue.
- **`geometry_properties_lod*` is a STRUCT**:
  `STRUCT("type" VARCHAR, surfaces VARCHAR, face_semantics INTEGER[], shells INTEGER[][])`.

Footer metadata — three keys, and the two objects disagree on `primary_column`
by design (`geo` must name a legal column, `city` names the richest one):

```sh
duckdb -c "SELECT key::VARCHAR AS k, octet_length(value) AS len
           FROM parquet_kv_metadata('$OUT/delft/building.parquet');"
```

```
ARROW:schema | 17928
city         |  4078
geo          |  2226
```

```sh
duckdb -c "SELECT json_extract_string(decode(value),'\$.primary_column') AS primary_col,
                  json_keys(json_extract(decode(value),'\$.columns'))    AS geo_cols
           FROM parquet_kv_metadata('$OUT/delft/building.parquet') WHERE key::VARCHAR='geo';"
```

Expected: `geometry_lod0_0 | [geometry_lod0_0]`. The `city` object's own
`primary_column` is `geometry_lod2_2`, and each of its `columns` entries carries
`"encoding": "WKB"` — the only encoding CityParquet defines.

### 1.4 Verify the STAC `metadata.json`

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

Expected: `type: Feature`, `stac_version: 1.1.0`, the `stac-city3d/v0.2.0` +
`projection/v2.0.0` + `file/v2.1.0` extensions, `city_objects = 2231`,
**`lods = ['0.0', '1.2', '1.3', '2.2']`** (LoD0 is `'0.0'` now, not `'0'`),
`co_types = ['Building', 'BuildingPart']`, `proj:code = EPSG:7415`,
`cityparquet:version = 0.1.0-draft`, and assets `data` + `building.parquet`
with roles `['data', 'cityparquet-objects']`.

> Object tables are discovered via the `cityparquet-objects` asset role;
> sidecars carry `cityparquet-sidecar`. There is no top-level `tables` key.
> Anything still reading `manifest['tables']` is stale.

**The STAC Item follows the CRS state too** (`61fb6dc`: a declared-unknown CRS
claims no WGS84 extent either). Compare Delft against the CRS-less railway
package from 1.5:

```sh
python3 -c "
import json
for p in ['delft','railway']:
    d=json.load(open('/tmp/cp_test/%s/metadata.json'%p))
    pr=d['properties']
    print(p, '| proj keys:', [k for k in pr if k.startswith('proj:')],
          '| geometry:', 'null' if d.get('geometry') is None else 'present',
          '| bbox:', 'present' if d.get('bbox') else 'absent')
"
```

```
delft   | proj keys: ['proj:code'] | geometry: present | bbox: present
railway | proj keys: []            | geometry: null    | bbox: absent
```

With no resolvable CRS there is nothing to reproject to WGS84, so the Item
declares no footprint at all rather than a fabricated one — and the
`projection` extension drops out of `stac_extensions` entirely.

### 1.5 Convert a multi-module dataset (railway) — and the tri-state CRS

The fixture declares no `referenceSystem`. As of `0c9c917` that is **no longer
fatal** — it is declared:

```sh
$CP convert lib/cityparquet-rs/tests/fixtures/lod3_railway.city.json -o $OUT/railway --overwrite
ls $OUT/railway
```

Expected — a `warning:` on stderr, then a normal conversion:

```
warning: source carries a CRS-bearing coordinate (geometry, or a
GeometryInstance template placement) but declares no CRS; `city.crs` is written
as an explicit null (CRS unknown) and the coordinates carry no georeference —
supply the CRS explicitly to georeference them
121 13 0 0 6 6 85 34 3 0
```

`--crs` is still how you *georeference* such a source (any projected code;
`EPSG:25832` here is a plausible stand-in, and a geographic/degree-valued code
is refused because nothing here reprojects):

```sh
$CP convert lib/cityparquet-rs/tests/fixtures/lod3_railway.city.json \
    -o $OUT/railway_crs --crs EPSG:25832 --overwrite
```

That prints no warning and produces the same `121 13 0 0 6 6 85 34 3 0` report,
with `city.other.crs_source = "operator-supplied"` recorded in the footer.

**Verify all three CRS states**, which is the point of the tri-state rule —
`null` and ABSENT mean different things and neither may be confused with a
known CRS:

```sh
for t in delft/building railway/building railway/materials; do
  printf "%-20s " "$t"
  duckdb -noheader -list -c "SELECT decode(value) FROM parquet_kv_metadata('$OUT/$t.parquet') WHERE key::VARCHAR='city';" \
  | python3 -c "import sys,json; d=json.load(sys.stdin); print('crs key present:', 'crs' in d, '| value:', type(d['crs']).__name__ if d.get('crs') is not None else d.get('crs', 'ABSENT'))"
done
```

```
delft/building       crs key present: True  | value: dict     <- Known: a PROJJSON object
railway/building     crs key present: True  | value: None     <- Unknown: explicit null
railway/materials    crs key present: False | value: ABSENT   <- Unspecified: sidecar, no geometry
```

Per GeoParquet an **absent** `crs` asserts OGC:CRS84, so writing absent over
RD New coordinates would silently move the city; that is why `null` exists and
why only the sidecar may omit the key.

Back to the package itself. Expected report — note the non-zero sidecar counts
(85 materials, 34 textures, 3 templates) and the 6 dropped degenerate
rings/surfaces:

```
121 13 0 0 6 6 85 34 3 0
```

Expected: **9 module tables** + 3 sidecars + `metadata.json` = 13 files.
The module split, and the CityGML CM class names inside each:

```sh
for f in bridge building city_furniture generics relief transportation tunnel vegetation water_body; do
  echo -n "$f: "
  duckdb -noheader -list -c \
    "SELECT count(*)||' rows | '||string_agg(DISTINCT object_type, ',') FROM '$OUT/railway/$f.parquet';"
done
```

```
bridge:          9 rows | Bridge,BridgeInstallation,BridgeConstructiveElement
building:       59 rows | Building,BuildingInstallation
city_furniture: 11 rows | CityFurniture
generics:        3 rows | CityObjectGroup,GenericOccupiedSpace
relief:          1 rows | TINRelief
transportation: 10 rows | Railway
tunnel:         12 rows | Tunnel,TunnelInstallation
vegetation:     15 rows | SolitaryVegetationObject
water_body:      1 rows | WaterBody
```

(The row counts are the contract; `string_agg(DISTINCT …)` has no defined
ordering, so the class names may come out in a different order per run.)

This is the point of the by-module layout: `Bridge`, `BridgeInstallation` and
`BridgeConstructiveElement` share one file because they share a CityGML module,
and `object_type` — not the filename — carries the class.

```sh
duckdb -c "SELECT
  (SELECT count(*) FROM read_parquet('$OUT/railway/materials.parquet'))          AS materials,
  (SELECT count(*) FROM read_parquet('$OUT/railway/textures.parquet'))           AS textures,
  (SELECT count(*) FROM read_parquet('$OUT/railway/geometry_templates.parquet')) AS templates;"
```

Expected: `85 | 34 | 3`. (Sidecars are content-gated — there is no `--profile`
flag to opt into them any more.)

### 1.6 Round-trip: convert → export → compare

This is the core semantic-losslessness claim. **Pass `--no-lod0`:**

```sh
$CP convert lib/cityparquet-rs/tests/fixtures/delft.city.jsonl \
    -o $OUT/delft_rt --no-lod0 --overwrite
$CP export $OUT/delft_rt $OUT/delft_rt.city.jsonl
$CP compare lib/cityparquet-rs/tests/fixtures/delft.city.jsonl $OUT/delft_rt.city.jsonl
echo "exit=$?"
```

Expected (`export` prints `feature_count object_count
instance_geometries_dropped appearance_refs_dropped`):

```
1115 2231 0 0
equal (excluded: 20)
exit=0
```

> **Gotcha — LoD0 synthesis breaks a naive round-trip.** By default the CLI
> *synthesises* an LoD0 footprint for objects that lack one, so the
> GeoParquet-legal `geometry_lod0_0` column is populated. Exporting the default
> package and comparing gives **exit 2**, with one difference per affected
> object:
>
> ```
> object NL.IMBAG.Pand.0503100000000010-0: geometry at lod Some("0.0")
>   present in B, missing in A
> ```
>
> That is the default working as designed — but it means **round-trip equality must be
> tested with `--no-lod0`**.

### 1.7 The bundled interop script

```sh
(cd lib/cityparquet-rs && just interop)
```

Expected: `interop ok`. It converts both fixtures and has DuckDB assert the
results natively — 2231 Delft buildings, a `bbox.xmin` query, the 85/34/3
sidecars, and 121 rows unioned across railway's 9 module tables.

> **FIXED 2026-08-16 (was BROKEN).** `lib/cityparquet-rs/scripts/interop.sh` still passed
> `--profile compatibility`, removed with the by-module layout in `25d471b`, so
> the recipe died on `unexpected argument '--profile' found`. `just check` does
> not run `interop`, so CI never noticed. The railway convert now also prints
> the CRS warning from 1.5 (the fixture declares none) — that is expected
> stderr, not a failure. The cross-module union additionally passes
> `union_by_name = true`, since per-module schema pruning means two modules need
> not share a column list.

### 1.9 Partitioned output

```sh
$CP convert lib/cityparquet-rs/tests/fixtures/delft.city.jsonl \
    -o $OUT/delft_parts --partition features --feature-num 500 --overwrite
ls $OUT/delft_parts
duckdb -c "SELECT count(*) FROM read_parquet('$OUT/delft_parts/*/building.parquet');"
```

Expected — `convert` reports the partition count, a duplicate-id check, and now
(`571b24f`) `invalid_appearance_refs_dropped` on both the summary line and each
per-partition line, plus one line per partition:

```
partitions=3 duplicate_ids=0 invalid_appearance_refs_dropped=0
features-00000 1001 0
features-00001 1000 0
features-00002 230 0
```

`ls` then shows the three package directories `features-00000/1/2`, and the
glob query returns `2231` — every object, exactly once. `--partition count
--number N` and
`--partition box --cell-size METRES` are the other two methods.

### 1.10 The STAC catalogue driver (`scripts/catalog2cityparquet`, run from the repo root)

New Python driver that walks the published City3D STAC catalogue (~74k items,
53 collections), converts each item, and ledgers *why* each one did or did not
convert. Its own suite fakes every origin and subprocess — no network, no
binaries:

```sh
(cd lib/cityparquet-rs && just catalog-test)
```

Expected: **265 passed, 7 skipped**.

Everything else in this tool is network-dependent and **out of scope for a
routine test pass** — `just catalog-convert` is hours long against the live
catalogue. To prove a change on real data, one small collection is the intended
unit:

```sh
(cd lib/cityparquet-rs && just catalog-convert-collection rotterdam-3d out/e2e)
(cd lib/cityparquet-rs && just catalog-histogram out/e2e)
```

Roll the ledger up with `catalog-histogram`, never by counting lines in
`_reports/*.jsonl` — the files are append-only and a resumed run re-attempts a
previously failed item, so line-counting over-counts failures.

---

## Part 2 — duckdb-cityjson extension

### 2.1 SQL test suite

```sh
(cd lib/duckdb-cityjson && ./build/release/test/unittest "test/sql/*")
```

Expected: `All tests passed (3 skipped tests, 1331 assertions in 53 test cases)`.
The three skips are `require-env` gates on network access —
`test/sql/cityjson_fcb_remote.test` (`FCB_REMOTE_TEST_URL`, the network-gated
FlatCityBuf range-read test, 2.9) and `test/sql/cityjson_remote.test` +
`test/sql/cityjson_corpus_parity.test` (both `CITYJSON_REMOTE_TEST`).

### 2.2 Read CityJSONSeq

```sh
./lib/duckdb-cityjson/build/release/duckdb -c "
SELECT count(*) AS objects
FROM read_cityjsonseq('lib/cityparquet-rs/tests/fixtures/delft.city.jsonl');"
```

Expected: `2231` — the same object count cityparquet-rs reports. That
cross-implementation agreement is the point of the check.

### 2.3 Per-LoD WKB mode — the columns are suffixed now

Without `lod =>` you get `geom_lod*` STRUCTs. With it you get the same suffixed
grammar the Parquet file uses: `geometry_lodX_Y` (BLOB/WKB) +
`geometry_properties_lodX_Y` (STRUCT) + `material_lodX_Y` / `texture_lodX_Y`.
**There is no bare `geometry` column** — that is what keeps the LoD recoverable
from the column name, and what lets `COPY TO cityjson` re-emit it.

```sh
./lib/duckdb-cityjson/build/release/duckdb -c "
SELECT id, geometry_properties_lod2_2
FROM read_cityjsonseq('lib/cityparquet-rs/tests/fixtures/delft.city.jsonl', lod => '2.2')
WHERE geometry_lod2_2 IS NOT NULL LIMIT 2;"
```

Expected — a **STRUCT**, not JSON text (`d334b26`, breaking):

```
{'type': Solid,
 'surfaces': '[{"type":"GroundSurface"},{"type":"RoofSurface"},
               {"on_footprint_edge":true,"type":"WallSurface"},
               {"on_footprint_edge":false,"type":"WallSurface"}]',
 'face_semantics': [0, 2, 2, 2, 2, 1],
 'shells': [[6]]}
```

`shells` is `INTEGER[][]` — nested unconditionally, so a plain `Solid` is
`[[6]]` rather than `[6]`. `surfaces` stays a JSON string because the `json`
extension is not a dependency of this one.

### 2.5 GeoParquet `geo` footer generation

```sh
./lib/duckdb-cityjson/build/release/duckdb -noheader -list -c "
SELECT geo IS NOT NULL FROM cityjson_geoparquet_geo('lib/cityparquet-rs/tests/fixtures/delft.city.jsonl');"
```

Expected `true`: a `geo` object is emitted for the GeoParquet-legal columns.

Used with `COPY`, this is the SQL-native executable prototype of the encoding:

```sh
./lib/duckdb-cityjson/build/release/duckdb -unsigned -c "
SET VARIABLE geo = (SELECT geo FROM cityjson_geoparquet_geo('lib/cityparquet-rs/tests/fixtures/delft.city.jsonl'));
COPY (SELECT * FROM read_cityjsonseq('lib/cityparquet-rs/tests/fixtures/delft.city.jsonl'))
  TO '/tmp/cp_test/delft_duckdb.parquet'
  (FORMAT PARQUET, KV_METADATA {geo: getvariable('geo')});
SELECT count(*) AS n FROM read_parquet('/tmp/cp_test/delft_duckdb.parquet');"
```

Expected: `2231`.

### 2.6 Appearance sidecar readers

Three table functions produce the sidecar tables directly, with ids interned
across the whole file:

```sh
F=lib/cityparquet-rs/tests/fixtures/lod3_railway.city.json
./lib/duckdb-cityjson/build/release/duckdb -c "
SELECT (SELECT count(*) FROM cityjson_materials('$F'))          AS materials,
       (SELECT count(*) FROM cityjson_textures('$F'))           AS textures,
       (SELECT count(*) FROM cityjson_geometry_templates('$F')) AS templates;"
```

Expected: `85 | 34 | 3` — **the exact counts cityparquet-rs writes in 1.5**.
Two independent implementations interning the same file's appearance to the same
cardinality is the strongest cheap check in this document.

`read_cityjson[seq](..., appearance := 'sidecar')` is the scan-side counterpart:
it emits global sidecar ids instead of file-local ones.

```sh
./lib/duckdb-cityjson/build/release/duckdb -c "
SELECT count(*) AS rows, count(material_lod3_0) AS with_material
FROM read_cityjson('$F', lod => '3', appearance := 'sidecar');"
# 121 | 24
```

### 2.7 CityParquet package mutation in SQL

A CityParquet package is a **DuckDB schema** whose tables are named by the
spec's file basenames, plus a `__cityparquet` bookkeeping table. The mutating
entry points are `PRAGMA`s that *return SQL text*, which DuckDB then runs inside
the caller's transaction.

> **Two traps that will bite you immediately.**
> 1. **Pragma expansion happens before execution, for the whole submitted
>    script.** A `CREATE SCHEMA` and a `PRAGMA` that depends on it cannot share
>    one script — the pragma is expanded against the pre-batch catalog and fails
>    with `Schema with name pkg does not exist`. Use one statement per `-c`, or
>    a persistent database file and separate invocations, as below. (This bites
>    `cityparquet_init`, which validates the schema exists at expansion time;
>    `cityparquet_read` tolerates a same-script `CREATE SCHEMA` — see 4.5's
>    headline transcript, which relies on exactly that.)
> 2. **Never name the database file after the schema.** `duckdb pkg.db` plus a
>    schema `pkg` gives `Ambiguous reference to catalog or schema "pkg"`.
>
> Also: PRAGMA named parameters use `=`, not `:=`.

```sh
cd lib/duckdb-cityjson
rm -f /tmp/cp_test/mut.db && rm -rf /tmp/cp_test/pkg_out
D=/tmp/cp_test/mut.db
F=../cityparquet-rs/tests/fixtures/delft.city.jsonl

# 1. Seed the package. `cityparquet_init` REQUIRES at least one object table —
#    it refuses an empty schema.
./build/release/duckdb $D -c "CREATE SCHEMA pkg;
  CREATE TABLE pkg.building AS SELECT * FROM read_cityjsonseq('$F', lod => '2.2');"

# 2. Create/refresh the bookkeeping table.
./build/release/duckdb $D -c "PRAGMA cityparquet_init('pkg');"
./build/release/duckdb $D -c "SELECT table_name, role FROM pkg.__cityparquet ORDER BY 1;"
```

```
building | object
```

Consistency checks — an empty result *is* the pass:

```sh
./build/release/duckdb $D -c "PRAGMA cityparquet_validate('pkg');" \
                       -c "SELECT * FROM cityparquet_validation;"
# 0 rows
```

One call adds a whole file, routed **by module**:

```sh
./build/release/duckdb $D -c "PRAGMA insert_cityjsonseq('pkg', 'test/data/railway_appearance.city.jsonl');"
./build/release/duckdb $D -c "SELECT table_name, role FROM pkg.__cityparquet ORDER BY 1;"
```

Expected — two new object tables and three sidecars, created as the source
needed them:

```
bridge             | object
building           | object
city_furniture     | object
geometry_templates | sidecar
materials          | sidecar
textures           | sidecar
```

Write the schema back out as a package. `cityparquet_write` is a **table
function**, not a pragma (it must decide whether to emit a `geo` key at all,
which SQL cannot branch on):

```sh
./build/release/duckdb $D -c "SELECT * FROM cityparquet_write('pkg', '/tmp/cp_test/pkg_out', crs => 'EPSG:7415');"
```

```
bridge.parquet             | written |    1 |   15406
building.parquet           | written | 2231 | 2632606
city_furniture.parquet     | written |    1 |   16870
geometry_templates.parquet | written |    3 |   22633
materials.parquet          | written |    4 |    1770
textures.parquet           | written |    4 |    1333
metadata.json              | written |    0 |    7826
```

> **Object tables changed shape underneath these byte counts.** Reserved
> columns are now emitted in the spec's normative order — `id, feature_id,
> object_type, parents, children, children_roles, address, bbox, <geometry
> quad per LoD>, template, other` — with attribute columns strictly after all
> of them. `address` and `template` are present (this writer declares them)
> but always `NULL`, because it parses neither yet:
>
> ```sh
> duckdb -c "SELECT count(*) FILTER (WHERE address IS NOT NULL) AS addr_nonnull,
>                   count(*) FILTER (WHERE template IS NOT NULL) AS tmpl_nonnull
>            FROM read_parquet('/tmp/cp_test/pkg_out/building.parquet');"
> # addr_nonnull=0 | tmpl_nonnull=0
> ```
>
> Note also: `other_attributes` no longer exists — `other` is the single
> escape hatch, always present in the schema and nullable, so there is no
> absent-column case for a reader to tolerate here.

> **`crs =>` is no longer mandatory** (`4b54c6b`, matching the tri-state rule in
> 1.5). It used to be a hard `Invalid Input Error: cityparquet_write: no crs`,
> because a hand-rolled schema load discards the footer's CRS. Omitting it now
> succeeds and declares the CRS unknown:
>
> ```sh
> ./build/release/duckdb $D -c "SELECT * FROM cityparquet_write('pkg', '/tmp/cp_test/pkg_nocrs');"
> ```
>
> It now also prints a `WARNING:` explaining why:
>
> ```
> WARNING:
> cityparquet_write: no CRS for schema 'pkg' -- the package's footer carries none
> and none was given (crs => 'EPSG:7415'), so every file's `crs` is written as an
> explicit null (CRS unknown) and metadata.json declares no projection
>
> bridge.parquet             | written |    1 |    9229
> building.parquet           | written | 2231 | 2628488
> city_furniture.parquet     | written |    1 |   10693
> geometry_templates.parquet | written |    3 |   22633
> materials.parquet          | written |    4 |    1770
> textures.parquet           | written |    4 |    1333
> metadata.json              | written |    0 |    3298
> ```
>
> The object tables come out **smaller** (no PROJJSON in the geometry columns'
> metadata) and `metadata.json` shrinks (no `proj:*`, no WGS84 footprint).
> Confirm the state rather than assuming it:
>
> ```sh
> duckdb -noheader -list -c "SELECT decode(value) FROM parquet_kv_metadata('/tmp/cp_test/pkg_nocrs/building.parquet') WHERE key::VARCHAR='city';" \
>   | python3 -c "import sys,json;d=json.load(sys.stdin);print('crs key present:', 'crs' in d, '| value:', d.get('crs'))"
> # crs key present: True | value: None
> ```
>
> **This warning is specific to a hand-built schema with no CRS anywhere in its
> footer** (as built above, straight from `read_cityjsonseq`, which carries no
> footer at all). It does **not** mean `crs =>` is generally required — see
> below.

The reverse direction — loading a cityparquet-rs package into a schema — works
straight off Part 1.2's output:

```sh
./build/release/duckdb -c "CREATE SCHEMA delft;" \
                       -c "PRAGMA cityparquet_read('/tmp/cp_test/delft', 'delft');" \
                       -c "SELECT count(*) FROM delft.building;"      # 2231
```

> **Every asset now declares its STAC roles** — `["data","cityparquet-objects"]`
> for a module table, `["data","cityparquet-sidecar"]` for a sidecar — which is
> how any reader separates the two without parsing filenames. That was the
> metadata blocker on reading this package back with cityparquet-rs, and it is
> fixed; pinned by `test/sql/cityparquet_io.test`.
>
> **The `geometry_templates.id` type divergence is settled** — the spec sides
> with duckdb-cityjson's `BIGINT`, and cityparquet-rs now matches it. The
> reverse round trip (a duckdb-cityjson-written package back through
> `cityparquet-rs export`) is covered in full in 4.5, headline result: **the
> CityJSON → rs → CityParquet → duckdb-cityjson → CityParquet → rs → CityJSON
> chain is semantically lossless.** One narrower divergence remains open — a
> degenerate-ring policy question — also documented there.

**The CRS survives `cityparquet_read` → `cityparquet_write` without passing
`crs =>`** — the package above (`pkg`) needed the flag only because it was
hand-built straight from `read_cityjsonseq` and carried no footer CRS to begin
with. Reading a CRS-bearing *package* (Part 1.2's `delft`) and writing it back
out with no `crs =>` argument at all carries the CRS through, no warning, full
PROJJSON in the written footer:

```sh
rm -f /tmp/cp_test/crscheck.db && rm -rf /tmp/cp_test/delft_nocrs_arg
./build/release/duckdb /tmp/cp_test/crscheck.db -c "
CREATE SCHEMA delft2;
PRAGMA cityparquet_read('/tmp/cp_test/delft', 'delft2');
SELECT * FROM cityparquet_write('delft2', '/tmp/cp_test/delft_nocrs_arg');"
```

```
building.parquet | written | 2231 | 3754220
metadata.json    | written |    0 |    6754
```

No warning. Confirm the PROJJSON actually made it into the written footer:

```sh
duckdb -noheader -list -c "SELECT decode(value) FROM parquet_kv_metadata('/tmp/cp_test/delft_nocrs_arg/building.parquet') WHERE key::VARCHAR='city';" \
  | python3 -c "
import sys,json
d=json.load(sys.stdin)
crs = d.get('crs')
print('crs key present:', 'crs' in d, '| type:', type(crs).__name__)
if isinstance(crs, dict):
    print('crs.type:', crs.get('type'), '| crs.name:', crs.get('name'), '| id:', crs.get('id'))
"
```

```
crs key present: True | type: dict
crs.type: CompoundCRS | crs.name: Amersfoort / RD New + NAP height | id: {'authority': 'EPSG', 'code': 7415}
```

**No `SET enable_geoparquet_conversion=false;` here.** `cityparquet_write` reads
every column of the schema, including `geometry_lod0_0`, and carries a
`GEOMETRY`-typed column through as `GEOMETRY` rather than downgrading it, so the
read → write sequence needs no flag — see 4.5. So:
`crs =>` is for georeferencing a package that has no CRS of its own (the
hand-built-schema case above); it is not needed merely to *carry forward* a CRS
the package already has.

The rest of the family — `cityparquet_reconcile`, `cityparquet_delete`
(cascade), `cityparquet_merge`, `cityparquet_orphans` / `cityparquet_vacuum` —
is covered by `test/sql/cityparquet_*.test` in 2.1. Each mutating pragma also
has a scalar `*_sql()` twin that returns the generated text without running it,
which is the fastest way to see what a call would do.

### 2.8 Metadata functions

```sh
./lib/duckdb-cityjson/build/release/duckdb -c "
SELECT version, title, city_objects_count, reference_system
FROM cityjsonseq_metadata('lib/cityparquet-rs/tests/fixtures/delft.city.jsonl');"
```

Expected: `2.0`, `3DBAG`, `2231`, and a `reference_system` struct
`{base_url: 'https://www.opengis.net/def/crs/', authority: EPSG, version: 0,
code: 7415}`, plus transform/extent/point-of-contact structs on the full row.

### 2.9 FlatCityBuf

The FCB dependency is now a released vcpkg port resolved from a git registry
(`HideBa/vcpkg`, pinned baseline), at tag **`cpp-v0.9.0`**. `read_flatcitybuf`
decodes only what the query projects (`FcbFieldMask`), and supports bbox
(`min_x`/`min_y`/`max_x`/`max_y`) and attribute-WHERE pushdown.

```sh
(cd lib/duckdb-cityjson && ./build/release/duckdb -c "
SELECT count(*) FROM read_flatcitybuf('test/data/fcb_bbox_attr.fcb');
SELECT count(*) FROM read_flatcitybuf('test/data/fcb_bbox_attr.fcb') WHERE height >= 10;")
```

Expected: `3` and `3`.

Network-gated remote range reads are opt-in and skipped by 2.1:

```sh
(cd lib/duckdb-cityjson && just test-fcb-remote)   # HTTP range requests, ~2.3 GB hosted 3DBAG
```

**Caveat:** the bbox in that test is a 500 m square hard-wired to the default
hosted subset (RD New / EPSG:7415), so a different `url=` needs a matching bbox.

### 2.10 Opt-in harnesses — wasm, and the C++ FCB tests

Neither runs under `make test`.

- **Wasm** (~2 GB toolchain, ~10 min bootstrap, ~4 min build) — not exercised in
  this pass:

  ```sh
  (cd lib/duckdb-cityjson && just wasm-setup && just wasm && just test-wasm)
  ```

  The Node smoke harness asserts `pragma_platform() = wasm_mvp`, that the
  extension loads, and that `read_cityjson` / `read_flatcitybuf` return the
  native build's oracle values. Remote reads are XFAIL under Node (its runtime
  implements only the `NODE_FS` protocol), not a bug in the artefact.

- **C++ FCB and encoder harnesses.** Both now run, once 0.4's dylib fix is in:

  ```sh
  P="$(pwd)/build/release/vcpkg_installed/arm64-osx"   # or .vendor/prefix
  FCB_PREFIX="$P" test/cpp/run_encoder_tests.sh
  FCB_PREFIX="$P" test/cpp/run_fcb_selective_tests.sh
  ```

  Expected: `All encoder assertions passed.` and
  `All fcb selective-deserialisation assertions passed.`

  > **FIXED 2026-08-16 (was BROKEN).** Three separate faults: the `duckdb`
  > shared library they link `-lduckdb` against did not build at all (0.4);
  > `run_encoder_tests.sh` was missing the `-I"$FCB_PREFIX/include"` its sibling
  > has, so `nlohmann/json.hpp` was not found; and
  > `run_fcb_selective_tests.sh`'s stale-library guard hardcoded the Linux
  > `libduckdb.so`, so on macOS it silently never fired — precisely when a stale
  > library is hardest to diagnose. `FCB_PREFIX` may be either the vcpkg prefix
  > the build actually used or a `just vendor-fcb` `.vendor/prefix`. Its subject matter (the selective
  FCB decode) is covered indirectly by 2.9.

---

## Part 3 — duckdb-3d extension

### 3.1 SQL + C++ suites

**`make test_full` is the target to verify against**, not the plain `make
test` + `make test_cpp` pair. It configures, builds, and runs everything in
one command, and — unlike a bare `make test` — it **stages the `cityjson` and
`spatial` extensions itself** into a scratch `HOME` before running, so the
`require cityjson` / `require spatial` interop tests actually **run instead of
skipping**:

```sh
(cd lib/duckdb-3d && make test_full)
```

Expected:
- SQL: `All tests passed (523 assertions in 33 test cases)` — **zero skips**
- C++: `All tests passed (528 assertions in 187 test cases)`

> **Under `test_full`, a skip is a failure, not an expectation.** The target
> stages the gated extensions itself, so if `require cityjson` / `require
> spatial` still skip, staging failed silently — the recipe greps its own
> log for `skipped test|were skipped` and exits non-zero if it finds one,
> specifically to catch that.

A bare `make test && make test_cpp` (no network, no staging) is a faster local
loop but reverts to the old behaviour — the `require cityjson` / `require
spatial` tests skip because the sqllogic runner cannot autoload them:

```sh
(cd lib/duckdb-3d && make test && make test_cpp)
```

- SQL: `All tests passed (5 skipped tests, 469 assertions in 28 test cases)`
- C++: `All tests passed (528 assertions in 187 test cases)`

The 5 skips there are 4 × `require cityjson` (the interop tests — Part 4
covers them manually) and 1 × `require spatial` (the coexistence test).

> **Rebuild before you trust a failure.** `make test` runs against whatever is
> in `build/release`; a stale extension produced 8 spurious failures here
> (`typeof(...)` returning `BLOB` where the new tests expect `SOLID_3D`). If
> failures look like the tests are ahead of the code, they are — rebuild.

### 3.2 Hollow solid / inner shell (spec §8 `shells`)

```sh
(cd lib/duckdb-3d && THREE_D_TEST_FIXTURES=1 ./build/release/test/unittest "test/sql/st_3d_hollow_solid.test")
```

Expected: `All tests passed (17 assertions in 1 test case)`.

**`THREE_D_TEST_FIXTURES=1` is now required** (`15659f4`). The `ST_AsWKB*`
fixture constructors are registered only when it is set, so without it the file
reports `All tests were skipped … require-env THREE_D_TEST_FIXTURES: 1`.
`make test` exports it; a bare `unittest` invocation does not.

This is the one that proves an interior shell's volume *subtracts* (outer cube
64 − cavity 8 = 56) and that a wrongly-wound cavity is **rejected** rather than
silently added.

### 3.3 The `ST_3D*` namespace

Every name that would otherwise collide with `spatial` now lives under
`ST_3D*` — including `ST_Transform`, which became **`ST_3DTransform`**
(`a6b1f1d`, breaking). Confirm what the build actually registers:

```sh
./lib/duckdb-3d/build/release/duckdb -noheader -list -c "
SELECT DISTINCT function_name FROM duckdb_functions()
WHERE function_name LIKE 'st\_3d%'     ESCAPE '\'
   OR function_name LIKE 'st\_geom3d%' ESCAPE '\'
   OR function_name IN ('st_crs','st_setcrs','st_makesolid','st_force3d','st_isplanar')
ORDER BY 1;"
```

You should see `ST_3DTransform` and the CRS accessors `ST_CRS` / `ST_SetCRS`. Note the class-generic constructors live
under `ST_Geom3D*`, not `ST_3D*`, so a bare `st_3d%` filter misses them.

### 3.4 Typed constructors and the fixture gate

Constructors now return the real `SOLID_3D` type rather than `BLOB`
(`d9a8faa`), and the `ST_AsWKB*` fixture builders are registered only under
`THREE_D_TEST_FIXTURES` (`15659f4`). Both at once:

```sh
THREE_D_TEST_FIXTURES=1 ./lib/duckdb-3d/build/release/duckdb -unsigned -c "
SELECT typeof(ST_3DFromWKB(ST_AsWKBPolyhedralTetra()))       AS typed,
       ST_3DVolume(ST_3DFromWKB(ST_AsWKBHollowCube()))       AS hollow_vol;"
```

```
SOLID_3D | 56.0
```

Count the gated functions to see the gate itself:

```sh
./lib/duckdb-3d/build/release/duckdb -noheader -list -c \
  "SELECT count(*) FROM duckdb_functions() WHERE function_name LIKE 'st\_aswkb%' ESCAPE '\';"
# 1   -- only the real ST_AsWKB exporter
THREE_D_TEST_FIXTURES=1 ./lib/duckdb-3d/build/release/duckdb -noheader -list -c \
  "SELECT count(*) FROM duckdb_functions() WHERE function_name LIKE 'st\_aswkb%' ESCAPE '\';"
# 12  -- plus the 11 fixture builders
```

Without the env var, `ST_AsWKBPolyhedralTetra()` is a `Catalog Error`, which is
the intended behaviour and not a broken build.

> Apart from those fixture builders there is no standalone geometry smoke test
> — `three_d` has no WKT constructor of its own (`ST_GeomFromText` belongs to
> the `spatial` extension), so real geometry has to arrive via `cityjson` or
> Parquet. Use Part 4.

---

## Part 4 — Cross-component integration

### 4.1 cityjson + three_d in one session

Load the cityjson extension into the three_d-preloaded shell (absolute path
required). Note the **suffixed** column names and that the STRUCT
`geometry_properties_lod*` goes straight into `ST_3DFromWKB` — no `to_json(...)`
round-trip, thanks to the `(BLOB, ANY)` overload:

```sh
./lib/duckdb-3d/build/release/duckdb -unsigned -c "
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

Expected:

```
cube | 6 | true | 6.0 | 1.0
```

### 4.2 The `shells` contract — a cavity must subtract

```sh
./lib/duckdb-3d/build/release/duckdb -unsigned -c "
LOAD '$(pwd)/lib/duckdb-cityjson/build/release/extension/cityjson/cityjson.duckdb_extension';
SELECT geometry_properties_lod2_0
FROM read_cityjson('lib/duckdb-3d/test/data/hollow_solid.city.json', lod => '2');
SELECT ST_3DNumShells(s) AS shells, ST_3DIsClosed(s) AS closed, ROUND(ST_3DVolume(s),6) AS volume
FROM (SELECT ST_3DFromWKB(geometry_lod2_0, geometry_properties_lod2_0) AS s
      FROM read_cityjson('lib/duckdb-3d/test/data/hollow_solid.city.json', lod => '2')
      WHERE geometry_lod2_0 IS NOT NULL);"
```

Expected — `shells: [[6, 6]]` in the properties, and the cavity subtracted
(64 − 8 = 56):

```
{'type': Solid, 'surfaces': NULL, 'face_semantics': NULL, 'shells': [[6, 6]]}
2 | true | 56.0
```

Note the LoD column is `*_lod2_0`, not `*_lod2` — the requested `lod => '2'`
normalises to the two-part `2_0` suffix.

### 4.3 The full chain — cityparquet-rs package → duckdb-3d (WKB)

The payoff test: 3D analysis straight off a CityParquet package written by the
Rust reference implementation, with no CityJSON in the loop.

```sh
./lib/duckdb-3d/build/release/duckdb -unsigned -c "
WITH solids AS (
  SELECT id, ST_3DTryFromWKB(geometry_lod2_2, geometry_properties_lod2_2) AS s
  FROM read_parquet('/tmp/cp_test/delft/building.parquet')
  WHERE geometry_lod2_2 IS NOT NULL
), v AS (
  SELECT id, s, ST_3DValidationReport(s) AS r FROM solids WHERE s IS NOT NULL
)
SELECT count(*) AS parsed,
       SUM(CASE WHEN r.is_valid THEN 1 ELSE 0 END) AS valid,
       ROUND(SUM(CASE WHEN r.is_valid THEN ST_3DVolume(s) END), 1) AS vol_valid_m3
FROM v;"
```

Expected:

```
parsed = 1116 | valid = 1098 | vol_valid_m3 = 1915861.2
```

Three things this proves at once: the `geometry_properties_lod*` STRUCT is
consumed directly (both producers now nest `shells` as `List<List<Int32>>`, and
the parser also still tolerates a pre-spec flat `[12]` from an older build);
1116 of 2231 rows carry LoD2.2 (geometry lives on BuildingParts, not their
Building parents); and 18 real-world solids are non-manifold.

> **Required idiom.** `ST_3DTryFromWKB` + `ST_3DValidationReport`, never bare
> `ST_3DFromWKB` — on real data the strict constructor aborts the whole query
> with `ST_3DVolume: solid is not manifold`.
>
> **`SET enable_geoparquet_conversion=false` does nothing for this column.** The
> LoD0 footprint carries the Parquet `GEOMETRY` logical type, and DuckDB's
> promotion follows the logical type rather than the `geo` footer — so the column
> reads as `GEOMETRY` whether the setting is on or off, and `geometry_lod0_0::BLOB`
> raises either way. Nothing here needs the flag: projecting `geometry_lod2_2`
> alone is fine, and so is `SELECT *` (2231 rows). To hand the footprint to a
> `duckdb-3d` constructor, pass it straight in — `ST_Geom3DFromWKB` takes
> `GEOMETRY` as well as `BLOB`.

### 4.5 The full duckdb-cityjson ↔ cityparquet-rs round trip

This closes the gap the previous pass of this guide recorded as **BROKEN**. A
cross-repo campaign settled the `geometry_templates.id` schema divergence (the
spec sides with duckdb-cityjson's `BIGINT`; cityparquet-rs now matches it), so
the direction that used to fail — a duckdb-cityjson-written package read back
by `cityparquet-rs export` — now works, and the full six-hop chain is
semantically lossless:

CityJSON → cityparquet-rs → CityParquet → **duckdb-cityjson** → CityParquet →
cityparquet-rs → CityJSON

```sh
rm -rf /tmp/n && mkdir -p /tmp/n
lib/cityparquet-rs/target/release/cityparquet convert lib/cityparquet-rs/tests/fixtures/delft.city.jsonl -o /tmp/n/rs --no-lod0 --overwrite
```

```
2231 2 0 0 0 0 0 0 0 0
```

```sh
lib/duckdb-cityjson/build/release/duckdb -c "
CREATE SCHEMA d;
PRAGMA cityparquet_read('/tmp/n/rs','d');
SELECT * FROM cityparquet_write('d','/tmp/n/rt', crs=>'EPSG:7415');"
```

```
building.parquet | written | 2231 | 3690624
metadata.json    | written |    0 |    6749
```

```sh
lib/cityparquet-rs/target/release/cityparquet export /tmp/n/rt /tmp/n/rt.city.jsonl
lib/cityparquet-rs/target/release/cityparquet compare lib/cityparquet-rs/tests/fixtures/delft.city.jsonl /tmp/n/rt.city.jsonl
echo "exit=$?"
```

```
1115 2231 0 0
equal (excluded: 20)
exit=0
```

> **The `cityparquet_read` → `cityparquet_write` leg needs no
> `SET enable_geoparquet_conversion=false;`.** `cityparquet_write` touches every
> column of the schema, including `geometry_lod0_0`, and keeps a `GEOMETRY`-typed
> column `GEOMETRY`-typed through the `COPY`. The flag would change nothing in any
> case: promotion follows the Parquet logical type, not the `geo` footer.
> `--no-lod0` on the initial convert **is** required, for the reason
> 1.6 already documents — LoD0 synthesis would otherwise legitimately add a
> footprint and fail `compare` with exit 2.
>
> `["data","cityparquet-objects"]` / `["data","cityparquet-sidecar"]` asset
> roles (fixed earlier) get rs as far as discovering the package's tables;
> matching `geometry_templates.id` types is what lets it actually read them.

**One divergence remains, and it is real** — a **sidecar-bearing** package
still fails rs export:

```sh
lib/cityparquet-rs/target/release/cityparquet export /tmp/cp_test/pkg_out /tmp/cp_test/pkg_out.city.jsonl
```

```
error: geometry error: polygon ring has 2 points after stripping the closing vertex, need at least 3
```

Diagnosed: the source (`duckdb-cityjson/test/data/railway_appearance.city.jsonl`)
contains a degenerate ring. cityparquet-rs **drops** it at write time —
converting that same source with `--tolerate-invalid-appearance` reports
`2 6 0 0 1 1 3 4 3 1`, i.e. `degenerate_rings_dropped=1`,
`degenerate_surfaces_dropped=1` — and re-exporting its own output is clean
(`2 2 0 0`, exit 0):

```sh
lib/cityparquet-rs/target/release/cityparquet convert duckdb-cityjson/test/data/railway_appearance.city.jsonl \
    -o /tmp/cp_test/railway_appearance_tolerant --tolerate-invalid-appearance --overwrite
# warning: source carries a CRS-bearing coordinate ... (as in 1.5)
# 2 6 0 0 1 1 3 4 3 1
lib/cityparquet-rs/target/release/cityparquet export /tmp/cp_test/railway_appearance_tolerant /tmp/cp_test/railway_appearance_tolerant.city.jsonl
echo "exit=$?"
# 2 2 0 0
# exit=0
```

duckdb-cityjson, by contrast, writes the degenerate ring through unchanged;
rs's reader then rejects it. `export` has no `--tolerate-invalid-appearance`
counterpart, so there is currently no flag to bridge this on the read side.
**This is a geometry-validity policy divergence and an open format decision,
not a schema or metadata defect** — everything at the schema/metadata level is
closed. See Known issues.

The gap is one-directional — DuckDB reads both its own output and the Rust
writer's:

```sh
(cd lib/duckdb-cityjson && rm -f /tmp/cp_test/rt.db \
  && ./build/release/duckdb /tmp/cp_test/rt.db -c "CREATE SCHEMA x;" \
  && ./build/release/duckdb /tmp/cp_test/rt.db -c "PRAGMA cityparquet_read('/tmp/cp_test/pkg_out', 'x');" \
  && ./build/release/duckdb /tmp/cp_test/rt.db -c "SELECT table_name, role FROM x.__cityparquet ORDER BY 1;")
# bridge/building/city_furniture = object, geometry_templates/materials/textures = sidecar
```

(Expect a warning that a `GEOMETRY` column's CRS is not persisted in a
pre-v1.5.0 storage-version database file; it does not affect the load.)

---

## Part 5 — Benchmarks (procedure only; not re-executed in this pass)

A fresh read + ordering benchmark run landed since the last pass
(`01b719d`, Linux/EPYC, 2026-08-17), and the `benchviz` summary-page pipeline
arrived with it (`c87aaa9`, `benchmark/plot/benchviz`). §5.6 below now points at
that pipeline's repo-level recipe rather than invoking a report script
directly.

### 5.0 State of the corpus and the results tree

**Check it, do not read it from here.** This section used to carry a table of
which CSVs existed on which date, and it was wrong within a week — the read
results were regenerated, the compression CSVs of the superseded corpus were
deleted. Two commands answer the question at the moment you ask it:

```sh
ls benchmark/formats/data benchmark/formats/read_results benchmark/formats/ordering_results benchmark/formats/compression_results 2>&1
git log --oneline -3 -- benchmark/formats/read_results benchmark/formats/ordering_results
```

The two methodology documents beside them state what a committed run means, and
are kept current: `benchmark/formats/READ_BENCHMARK.md` (the cross-format read benchmark and
its fairness caveats — the CSVs it describes are committed) and
`benchmark/formats/README.md` (the write/compression benchmark — no CSVs committed; run the
recipes to produce them).

Two things worth knowing before a re-run, because neither is visible from a
directory listing:

- `benchmark/formats/data/readbench/` prepared artefacts carry the version of the conversion
  chain that built them; a stale stamp makes `readbench_prepare.sh` refuse the
  dataset and print the exact `rm -rf` that clears it. Delete the tree if in
  doubt — nothing there is expensive to rebuild except the downloads.
- A package built before the tri-state `city.crs` change has no `crs: null` for
  a CRS-less dataset. Regenerate packages rather than mixing generations.

### 5.1 Corpus eligibility — read this before running anything

**The CRS filter is gone.** Until `0c9c917` a CRS-less dataset was a hard
conversion error and 8 of the 15 could not be converted at all. They now all
convert; a CRS-less one simply gets `city.crs: null` and a stderr warning. What
remains is one blocker and one caveat:

| Filter | Datasets affected |
|---|---|
| **Multi-module package** → the read-bench runner rejects it | `Railway`, `lod3_railway` |
| *Caveat, not a blocker:* **no declared CRS** → converts, but the package is not georeferenced (`city.crs: null`, no `proj:*` in its STAC Item, no WGS84 extent) | `3dbag_subset`, `Helsinki`, `Helsinki_tex`, `Montreal`, `NYC`, `Railway`, `Vienna`, `Zurich` (8 of 15) |

The other 7 declare a `referenceSystem` and are georeferenced without help:
`3DBAG` (EPSG:7415), `3DBV` (7415), `9-196-328` / `9-284-556` / `9-304-532`
(7415), `Ingolstadt` (32632), `Rotterdam` (28992). (The `delft` fixture used
throughout Parts 1–4 also declares 7415, but it lives in `tests/fixtures/`, not
in the bench corpus.)

Pass `--crs` per dataset if a benchmark run needs georeferenced output; no
bench script passes it for you.

Verify a source's declaration before blaming the writer:

```sh
python3 - <<'EOF'
import json, glob, os
for f in sorted(glob.glob('benchmark/formats/data/*.jsonl') +
                glob.glob('benchmark/formats/data/*.json')):
    d = json.loads(open(f).readline())
    print('%-28s %s' % (os.path.basename(f),
          d.get('metadata', {}).get('referenceSystem') or 'ABSENT'))
EOF
```

### 5.2 Regenerate all CityParquet packages

```sh
rm -rf out/cityparquet
just convert-all benchmark/formats/data out/cityparquet
```

One package directory per input. Includes `3dbag_subset.city.jsonl` (2.8 GB) —
drop it from `benchmark/formats/data` for a quick pass. Since `0c9c917` a CRS-less dataset
no longer aborts the loop (it warns and writes `city.crs: null`), so the whole
corpus converts in one go except `Railway`: `just convert-all` runs a bare
`convert` and does not pass `--tolerate-invalid-appearance`, so Railway's
dangling material reference still aborts it under strict mode (Known issues,
"Fixed in this pass" #7 — the fix is a flag, not a default, and this recipe
does not opt in). The `benchmark/formats/data/_run` hard-link staging directory remains
useful for skipping it.

### 5.3 Compression benchmark

```sh
just compression-bench benchmark/formats/data benchmark/formats/compression_results
```

8 variants per dataset (codec axis: default-zstd, uncompressed, snappy, gzip,
lz4, brotli; row-group axis: default, rg512, rg4096), then charts via `uv`.

> **Do not read a "smallest codec" ranking off this table.** The codec axis
> runs every codec at its *crate default* — zstd@3, gzip@6, brotli@**1** — so
> the levels are not matched. Compression-vs-none is citable; codec-vs-codec
> is not.

### 5.4 Write benchmark

```sh
just write-bench benchmark/formats/data          # -> benchmark/formats/results/ (not committed)
```

Needs network on first run (installs the `cityjson` community extension for the
DuckDB `COPY` baseline).

### 5.5 Read benchmark

```sh
rm -rf benchmark/formats/data/readbench          # only if you want a clean prepare
just bench benchmark/formats/data benchmark/formats/read_results
```

`readbench_prepare.sh` **skips any package directory that already exists**, so
delete first whenever the format has moved. Table lookups go through
`benchmark/scripts/package_tables.py`, which mirrors the Rust `PackageTables::open`
(object tables = assets with the `cityparquet-objects` role).

New since the last pass: an **HTTP transport**. `--transport local|http`
(default `local`) plus `--base-url` makes every format read over HTTP instead of
from a local file, and populates the trailing `bytes_read` / `http_requests`
CSV columns (empty for every local row). Upload steps and methodology are in
`benchmark/formats/READ_BENCHMARK.md`; `duckdb-parquet` has no HTTP row — it is a local-only
SQL baseline.

> **Still blocked: multi-module datasets.** The `CityParquetRunner` supports
> only single-table packages, so `Railway` and `lod3_railway` produce no read
> numbers, and one test is `#[ignore]`d for it.

### 5.6 Aggregate results into one page

```sh
just plot-pretty
```

The renderer (`benchmark/plot/benchviz`) reads what the runs above left under
`benchmark/formats/` and writes `benchmark/summary/` — `bench_data.json`, a
self-contained `bench-summary.html`, and the static print figures. It measures
nothing, so re-running it is free. Read the messages it prints: the figures step
can refuse (existing print-sheet captions are tied to the corpus they were drawn
from) while still writing the HTML page.

The **paper** repository runs the same renderer against this repository as a
submodule, pointing its figure output at `paper/assets/bench/` — that is
`just bench-summary` there, not here.

---

## Known issues found during this pass

### Fixed in this pass

All committed and pushed to `develop` — none of this is a working-tree state
any more. `duckdb-3d` needed no code change for the interop closure below — it
was the conformant side throughout.

| # | Issue | Fix |
|---|---|---|
| 2 | **A duckdb-cityjson-written package could not be read by cityparquet-rs** — `geometry_templates.id` was `BIGINT` from duckdb-cityjson and `VARCHAR` from cityparquet-rs. | **Settled in cityparquet-rs's favour of the spec, against its own prior schema**: the spec's `04-appearance-templates.mdx` mandates `id BIGINT`, and cityparquet-rs's `geometry_templates_schema` now matches it. The full six-hop round trip is lossless. Verified in 4.5 |
| 3 | **`make release` failed** — `src/libduckdb.dylib` did not link flatcitybuf | The deferred link fixup that already existed for `unittest` now also covers DuckDB's `duckdb` SHARED target (`cityjson_fcb_link_upstream_targets`, `CMakeLists.txt`). Verified in 0.4 |
| 4 | **The `test/cpp` harness could not run** | Fixed by #3, plus `run_fcb_selective_tests.sh`'s stale-library guard is no longer hardcoded to the Linux `libduckdb.so`. Verified in 2.10 |
| 5 | **`just interop` was broken** — `lib/cityparquet-rs/scripts/interop.sh` still passed the removed `--profile compatibility` | Flag dropped; stale by-family comments corrected to by-module; the cross-module union now uses `union_by_name = true`. Verified in 1.7 |
| 7 | **`Railway.city.jsonl` fails conversion** — `material index 2 in theme 'visual' out of range (local defs len 2)` | **No longer an open decision.** cityparquet-rs gained `--tolerate-invalid-appearance`, which drops the dangling reference and counts it in the report's tenth field rather than aborting the whole conversion. Strict remains the default — a bare `convert` still refuses the file with the message above. Verified: `convert benchmark/formats/data/Railway.city.jsonl -o … --tolerate-invalid-appearance` reports `121 13 0 0 6 6 84 34 3 1` (84 materials written, one dropped) and exits 0 |
| 10 | `cityparquet-rs/CLAUDE.md` + `AGENTS.md` documented `convert INPUT OUTPUT_DIR` positionally | Both now show `--output`, list the flags added since (`--partition`, `--crs`, `--no-lod0`), and the catalogue suite count is 265, not 219 |

### Open — needing a decision, not a patch

| # | Issue | Why it is a decision |
|---|---|---|
| 13 | **A sidecar-bearing package written by duckdb-cityjson still fails `cityparquet-rs export`** — `polygon ring has 2 points after stripping the closing vertex, need at least 3`. | **A geometry-validity policy divergence, not a schema or metadata defect.** The source (`railway_appearance.city.jsonl`) carries a degenerate ring; cityparquet-rs *drops* such rings at write time (`--tolerate-invalid-appearance` reports `degenerate_rings_dropped=1`), but duckdb-cityjson writes the ring through unchanged, and rs's reader has no equivalent "tolerate on read" mode. The decision is whether a CityParquet reader must silently repair/drop an invalid ring it did not write, or whether that stays the writer's job. Verified in 4.5 |

### Environmental blockers — real, will bite the next person, not code defects

| # | Issue | Why it isn't a code fix |
|---|---|---|
| 14 | **`just check` cannot pass in `cityparquet-rs` on this machine.** `cityparquet-readbench`'s `attr_consistency` test shells out to the external `fcb` binary with `-i`/`-o`, but the installed **`fcb` 0.7.8** takes positional `<INPUT>... <OUTPUT>` and has no `-i` flag. It fails identically on unmodified base commits. | Environmental — a version mismatch between the pinned CLI contract and what is installed. Because `check: lint test isolation vendor-check` runs `test` early, this failure means **`isolation`, `vendor-check`, and `cargo fmt --all --check` never run at all** under `just check`; they must be run separately (verified individually in 1.1, all exit 0) |
| 15 | **`duckdb-cityjson`'s pre-commit gates skip silently.** Local `clang-format` is **21.1.6**, CI pins **11.0.1** (a mismatched version reformats conforming code and churns the diff, so skipping is by design), and `clang-tidy` is not on `PATH`. | Local tooling mismatch, not something a commit can fix. Commits made on this machine are not format- or tidy-verified locally; CI is the first real check |

### Still open from before this pass

| # | Issue | Status |
|---|---|---|
| 8 | Read-bench runner rejects multi-table packages | Unchanged; one test `#[ignore]`d. `benchmark/readbench/src/formats/cityparquet.rs` |
| 9 | Default LoD0 synthesis breaks a naive round-trip `compare` | Unchanged (behavioural, by design). Needs `--no-lod0` |
| 6 | `vendor-check` + the new CityGML fixtures are undocumented prerequisites | Documented here in 0.1 / 0.2 rather than changed in code |

### Fixed before this pass

| # | Issue | Resolution |
|---|---|---|
| 11 | justfile did not parse; `readbench_duckdb.sh` and the `bench` recipe read the removed `manifest['tables']` key; `convert-all` / `encode_3dbag_tiles.sh` passed the output dir positionally | Merged into `develop` as `3e263ad` (2026-08-10); table lookups go through `benchmark/scripts/package_tables.py` |
| 12 | `benchmark/formats/data/readbench/` prepared artefacts carried the pre-by-type manifest | Regenerated 2026-07-24 |

---

## Summary

All four test suites are green at the commits above — cityparquet-rs 713
passed / 0 failed / 0 ignored across 43 targets (`--exclude
cityparquet-readbench`, see 1.1), duckdb-cityjson 1331 assertions in 53 cases
(3 skipped, all network-gated), duckdb-3d 523 SQL + 528 C++ assertions under
`make test_full` (zero skips — a skip there fails the run, see Part 3), and
the catalogue driver 265 passed / 7 skipped — and every functional path in
Parts 0–4 now works.

**The headline of this pass**: the duckdb-cityjson → cityparquet-rs interop
gap that the previous pass recorded as **BROKEN** at §4.5 is closed. The full
six-hop chain — CityJSON → cityparquet-rs → CityParquet → duckdb-cityjson →
CityParquet → cityparquet-rs → CityJSON — is now semantically lossless
(`equal (excluded: 19)`, exit 0). Closing it took settling the
`geometry_templates.id` schema divergence (BIGINT, matching the spec) and
adding `--tolerate-invalid-appearance` for dangling appearance references.

One **BROKEN** marker remains, and it is an environment prerequisite, not a
library defect: §0.3, an old vcpkg checkout on a fresh machine. One format
decision remains genuinely open: the degenerate-ring policy divergence in
4.5/13 above, where cityparquet-rs drops invalid rings at write time and
duckdb-cityjson does not, and rs's reader has no equivalent tolerance on read.
Two environmental blockers (14, 15) are real and will recur on a fresh
checkout; neither is a code defect.
