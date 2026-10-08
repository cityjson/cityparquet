# Library gap closing — 2026-10-08

Source: `../notes/cityparquet-monorepo-todo.md` (library items only). Spec is frozen:
any change it seems to need is deferred to the author. All work on `develop` in the
owning repo; parent bumps submodule pointers.

## Decisions (author, 2026-10-08)

- LoD0 synthesis becomes CLI opt-in (`--lod0`); default encodes only what the source carries.
- `other` and `geometry_properties.surfaces` are written as the Parquet **JSON** logical type
  by both writers; readers accept JSON and plain UTF8.
- LoD4 → CityJSON export writes `"4.0"` and warns; CityGML export keeps `lod4Solid`.
- In scope: FlatCityBuf input (rs, feature-gated), `cityparquet collection` (via the
  city3d-stac-tool crate, no duplicate impl), duckdb-3d implicit geometry, release items
  (each outward-facing step confirmed first).

## Milestones

| # | Milestone | Done when |
|---|-----------|-----------|
| M1 | **Conformance backbone** — `cityparquet validate` (logical types, `city`/`geo` footers, sidecars) in rs; regenerate stale `cityparquet_rs_minimal` fixture; cross-writer test both ways | duckdb-written package passes rs validate + rs read; rs package passes `cityparquet_read`/`cityparquet_validate` |
| M2 | **JSON logical type** in rs + duckdb-cityjson writers; readers + duckdb-3d accept it | M1 validator checks JSON type and both writers pass |
| M3 | **duckdb-cityjson read/write gaps** — `address`, `implicit_geometry` + `geometry-templates` both directions; no degenerate rings on write; stale FUNCTIONS.md | CityJSON → package → CityJSON keeps address + templates (real fixture) |
| M4 | **duckdb-cityjson Hilbert order** — same key as rs (`order.rs`) | same input → same row order from both writers |
| M5 | **Append** — appearance-bearing data into an rs-written package | test on real fixture passes |
| M6 | **rs**: LoD0 opt-in; LoD4 fixture + export; README CityGML scope; `collection` subcommand; FlatCityBuf input | tests per item; `just check` green |
| M7 | **duckdb-3d**: STRUCT overload for GEOMETRY, flat `shells` on STRUCT path, ear-clipping silent stop → error/fix, implicit geometry | `make test_full` green, no skips |
| M8 | **E2E on real data** — `test/TESTING.md` + each module's TESTING.md (Delft, Railway, Helsinki, 3DBAG tile); fix stale pins/paths/counts | walkthrough re-run, results recorded |
| M9 | **Optimise (measured)** — rs convert (CityGML 3× parse, buffering, per-row JSON), duckdb-3d import/triangulation | before/after timings on a real dataset |
| M10 | **Release** — community extension rebuild, tag 0.1.0, crates.io (`city3d-stac-types` first) | each step confirmed with author |

## Status

- [x] M1 · [x] M2 · [x] M3 · [x] M4 · [x] M5 · [x] M6 · [x] M7 · [x] M8 · [x] M9 · [ ] M10
- M1 left over: an automated cross-writer check (root `just conformance`: rs↔duckdb write,
  validate, read) — do in M8. duckdb nullability: DuckDB writer can't declare REQUIRED (warnings).
- duckdb-cityjson bfe27d7..9c547ee (M5: append into rs packages, bbox kept); duckdb-3d 33bf4e1..cc355fb (Codex-reviewed, findings fixed).
- M1/M2 rs done (a4ea5cc..f9e0374): `cityparquet validate`, JSON logical type; duckdb packages
  fail only `stac.datetime` → sent to duckdb agent with fixture regen.
- Conformance: `just conformance` (dc11ef2) green — rs↔duckdb on delft + railway, 18 checks.
- M8 duckdb-cityjson done (616930b..c39d1bc): notebook/remote/fcb-remote suites green,
  TESTING.md current + 5 real-data cells; insert 6.8 s → 4.8 s (Delft).
- M9 duckdb-3d done (cc355fb..faff8fe), results byte-identical: Helsinki import 1.36→0.74 s,
  distance 2.0→0.22 s, DWithin 0.23→0.005 s (bbox pruning); bench in scripts/bench/.
- M8 duckdb-3d done (d36b91b..94268e7): test_full 37/37, debug test_all, CI format-check; docs
  re-run; new cityparquet_round_trip.test (both writers). Volumes shifted ≤1st decimal (triangulation).
- duckdb-cityjson 939e1f1: geometry_properties.type = CM name (MultiCurve) both ways.
- M9 rs done (c6586a8..8d12563), packages byte-identical: Montreal seq 13.2→4.7 s, 5.7→1.9 GB;
  NYC gml 7.5→3.3 s. Open: rs fails on Helsinki_tex ("44 uv indices for 45 distinct vertices").
- M8 monorepo walkthrough done (5225c63): run-all 11/11 green, six-hop chain equal, #13/#14
  closed. Follow-ups: #16 duckdb object attr → JSON; railway degenerate count 6→0 (rs, check).
- rs e35e9a3/b455a50/9e5de83: Helsinki_tex converts (71 rings one UV short → untextured + warning
  under --tolerate-invalid-appearance); LoD4 warning exact; collection item_assets = Items' assets.
  Railway degenerate 6→0 is a5b4883 (Aug, deliberate; MAY-drop; both writers agree) — kept.
- M10 extra: hosted CityParquet sample (18 Aug) predates the spec (`template` column) → republish.
- M7 follow-ups: `cityjson_interop.test:45` pins `surfaces JSON` (depends on read_cityjson's
  type); debug `make test_all` + clang-format 11 check not run; volumes in EXAMPLE/TESTING to
  re-check in M8; MCP corpus refresh + pointer bump in parent.

## Notes

- `collection`: the logic lives in `city3d-stac-gen` (upstream develop/main `c8ab8d9`):
  `StacCollectionBuilder::aggregate_from_items` is public, `handle_update_collection_command`
  is private. gen is heavy (stac 0.16, object_store 0.11 + cloud, fcb_core 0.6, reqwest) →
  feature-gate in the CLI, or lift a pub fn upstream.
- Release blockers: vendored `vendor/city3d-stac-tool` pinned to pre-split `c5a0eb3` while the
  cargo dep is `743199c` (split); crates.io ignores `[patch.crates-io] cjseq` (patched 0.4.1)
  → upstream the cjseq fix or publish a fork before `cargo publish`.

- M6 rs a0ba0fc..d529c7b: --lod0 opt-in, LoD4 (SIG3D fixture, xlink CompositeSurface fix),
  `collection` via city3d-stac-gen (feature; vendored stac-tool → c8ab8d9; geo 0.31), FCB input
  (feature). Caveats: gen's collection adds `item_assets.data` though rs Items have no `data`
  asset (upstream); no offline STAC schema validation of collection.json.

## Deferred to author (spec)

- Implicit geometry (04): order of matrix vs reference point unstated — duckdb-3d implements
  `M·v + p`, matrix translation included (matches cityjson.org figure).
- Implicit geometry: must the matrix be affine (last row 0,0,0,1)? duckdb-3d rejects others.
- `shells` (03): a `0` face count (dropped shell) is not mentioned by the spec.
- An object with several implicit geometries cannot be represented (already open).
- (05) GEOMETRY annotation `crs` MUST agree with `city.crs`, but absent `crs` = OGC:CRS84 in
  Parquet, so unsatisfiable when `city.crs` is null — validator accepts the pair.
- (02) reserved column order "normative for writers" without a MUST → validator warns.
- (03) "face" undefined for MultiPoint/MultiLineString → face invariants only on polygonal.
- (05) `.parquet` asset without `data` role: validator errors. Is that intended?
- (02) "non-null" required columns: value rule or declaration? Validator: null value = error,
  OPTIONAL declaration = warning (DuckDB declares every column OPTIONAL).
- `06-resources/02-software.mdx` status table should list `validate` (documents/ frozen).
- Addresses: member→field mapping unstated; real Helsinki `Country`/`Locality` map to nothing
  (both impls use CityJSON 2.0.1's example names + postBox/administrativeArea/freeText).
- Should `bbox` cover a placed implicit geometry? Both writers leave it NULL.
- "Fewer than three distinct positions": both writers count indices, so `[a,b,a]` passes.
- Append: does attribute type promotion apply across appends, must a writer report widening?
  List vs scalar under one name without JSON? Must insert keep other writers' bbox?
- `06-resources/02-software.mdx` also: `--no-lod0` → `--lod0`, `collection` exists, FCB input.
- (07) CityJSON mapping for `geometry_lod4_*` (CityJSON has no LoD4; impl writes "4.0" + warns).
- (05) add `"FlatCityBuf"` to the `source_format` tokens?
- (02/06) Reserved-name collision case-sensitive? Source attr `ID` vs reserved `id`: rs writes an
  `ID` column, duckdb diverts to `other`; DuckDB is case-insensitive → rs→duckdb→rs fails (#17).
- (03) `[a,b,a,a]` meets "closed, ≥4 points" yet has <3 distinct positions — well-formed or not?
- Should a writer repair a texture ring one UV short (e.g. duplicate the repeated index's UV)?
- Hilbert key not normative; rs keys on the whole vertex pool. STAC `datetime` fallback to
  write time is a writer convention. Address `location` MultiPoint `lod` member not emitted.
