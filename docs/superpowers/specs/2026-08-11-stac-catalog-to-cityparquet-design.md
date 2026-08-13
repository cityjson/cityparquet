# Converting the City3D STAC catalog to CityParquet

**Date:** 2026-08-11
**Status:** design, awaiting approval
**Scope:** a driver that walks the published City3D STAC catalog, converts every
item's source asset to a CityParquet package, and reassembles the result into a
mirror-image STAC catalog; plus the three small `cityparquet-rs` fixes and the one
additive `city3d-stac-tool` flag the driver needs.

---

## 1. Purpose

The published catalog at `https://storage.googleapis.com/city3d-stac/catalog.json`
describes 53 open 3D city model collections (**~74,600 items**) whose assets are
CityJSON, CityJSONSeq and CityGML files hosted by their original publishers. This
work produces the **CityParquet mirror** of that catalog: the same catalog /
collection / item tree, but with each item's data re-encoded as a CityParquet
package whose `metadata.json` *is* the STAC Item.

The mirror is a paper artefact in two ways:

1. It is by a wide margin the largest real-world corpus of CityParquet packages
   produced to date — the input for read/write benchmarks at national scale.
2. The run itself is a **conformance harness**. Every failure is recorded with its
   reason, so the output doubles as a measured statement of what fraction of the
   world's open 3D city data the reference implementation can ingest.

## 2. Findings from the conformance probe

One item per collection was downloaded and converted (2026-08-10/11); three items
were probed for `japan-plateau-3d` because it dominates the catalog.

### 2.1 Catalog inventory

Of the 53 collections, **33 publish any items**; the other 20 publish only a
`collection.json`. Item counts are extremely skewed:

| Collection | Items | Item index |
|---|---:|---|
| `japan-plateau-3d` | **60,471** | `items.parquet` present but **stale (306 rows)** |
| `netherlands-3d-bag` | 8,941 | `items.parquet` |
| `american-cities-3d` | 5,229 | `items.parquet` |
| `linz-3d` | 156 | `items.parquet` |
| `montreal-3d` | 77 | `items.parquet` |
| `craig-aura-zae-2024` | 21 | `items.parquet` |
| `craig-chatel-guyon-riom-2024` | 6 | `items.parquet` |
| `estonia-3d` | 5 | `items.parquet` |
| `ingolstadt-3d` | 3 | `items.parquet` |
| 26 others | 1 each | mixed; `stadt-freiburg-2024` has no index at all |

**Four collections are 99.8% of the catalog.** `japan-plateau-3d` alone is 81%.

### 2.2 Convertibility

**15 of 33 non-empty collections convert**, covering **~69,100 of ~74,600 items
(≈92%)** once the three fixes in §8 land. The item-level figure is dominated by
Japan; the collection-level figure is the honest measure of breadth.

Converting: `japan-plateau-3d` (60,090 tiles), `netherlands-3d-bag` (8,941),
`estonia-3d` (5), `craig-aura-zae-2024` (21), `craig-chatel-guyon-riom-2024` (6),
`stadt-freiburg-2024` (1, after §8.3), `rotterdam-3d`, `the-hague-3d`,
`stadt-hannover-2024`, `municipal-kuopio-2021`, `georiga-riga-2022`,
`craig-aura-eleven-sectors-2021`, `craig-clermont-ferrand-2024`,
`craig-isere-2025`, `craig-vichy-unesco-2024`.

Scale evidence: `estonia-3d` is a single **9.2 GB** CityGML → **917,882 objects**;
a Japanese whole-city ZIP → **32,582 objects**; Freiburg (with the §8.3 fix) →
**131,036 objects**; `municipal-kuopio-2021` → 65,593. A 3DBAG tile converts in
**0.92 s**. The CityGML 2.0 and CityJSON paths both scale.

### 2.3 Failure taxonomy

| # | Reason | Collections | Nature |
|---|---|---|---|
| 1 | **CityGML 1.0 / 3.0 unsupported** | 11 — `hamburg-3d`, `montreal-3d`, `lgb-brandenburg`, `geobremen-bremen`, `stadt-leipzig`, `lgln-niedersachsen`, `lvermgeo-rheinland-pfalz`, `lvermgeo-sachsen-anhalt`, `laiv-mecklenburg-vorpommern`, `geobasis-nrw` (1.0); `ingolstadt-3d` (3.0) | Converter gap. `citygml/sniff.rs:32` accepts only the 2.0 core namespace; anything else falls through to the CityJSON branch in `source.rs` and dies with a **misleading** `invalid CityJSON: expected value at line 1 column 1`. Blocks most of the German national LoD2 corpus. |
| 2 | **Source declares no resolvable CRS** | 4 — `luxembourg-3d`, `new-york-doitt-3d`, `vienna-3d`, `zurich-3d` (CityJSON with no `referenceSystem`; Luxembourg CityGML has `srsDimension` but no `srsName` anywhere) | Deliberate spec rule, not a bug. Unlocked by §8.2. |
| 3 | **Legacy CityJSON 1.0** (`"lod": 1` as integer, not string) | 1 — `singapore-hdb-3d` | Converter gap. |
| 4 | **Geographic CRS** (`EPSG:4979`) | 1 — `american-cities-3d` (**5,229 items**) | Converter gap: the CityGML reader supports projected, metre-based CRS only, because coordinates are quantised at 1 mm. |
| 5 | **Not a converter problem** | `linz-3d` (upstream HTTP 404), plus the 20 collections publishing zero items | Upstream data. Recorded as `empty_collection`; no `collection.json` is emitted. |

### 2.4 Transport quirks the driver must handle

These are why the fetch layer cannot be naive. Each was observed, not anticipated:

- **Item indexes can be stale.** `japan-plateau-3d`'s `items.parquet` has **306
  rows against 60,471 published items** (0.5%), and its `collection.json` carries
  the same 306 `rel=item` links. Trusting the fast path here would convert 1 item
  in 200 and report success.
- **Media types lie.** `hamburg-3d` advertises `application/gml+xml` with a `.GML`
  extension and serves a 468 MB **ZIP**. Freiburg advertises `application/gml+xml`
  and serves `application/octet-stream`. Format detection **MUST** sniff magic
  bytes, never the declared type or extension.
- **Nested archives.** `geobremen-bremen` is a **zip inside a zip**.
- **Archives can be dataset directories.** Japan's whole-city ZIPs hold 723 entries
  (136 `.gml` under `udx/<module>/`, plus `codelists/`, `metadata/`, a spec PDF);
  97 MB compressed expands to 1.3 GB.
- **User-Agent gating.** `montreal-3d` returns HTTP 403 without a browser UA.
- **No filename in the URL.** `estonia-3d` hrefs are query-string URLs with no path
  extension.
- **No `Content-Length`.** PLATEAU `.gml` responses are chunked, so size is unknown
  until the download completes — no pre-flight size guard is possible.
- **Compression is the norm.** 3DBAG is all `.json.gz`, american-cities all `.zip`.
- **Slow origins and very large files.** Estonia: 330 MB in ~11 min. Freiburg:
  1.86 GiB at ~9 MB/s (223 s). Japan's largest ZIP is 1.15 GB → 1.29M objects.
  Timeouts must be per-phase and generous; a 300 s budget is far too small.

## 3. Decisions

| Decision | Choice | Rationale |
|---|---|---|
| Host repo | `cityparquet-rs` | The driver sits next to the converter it drives. The registry repo's `CLAUDE.md` forbids code ("metadata-first, not code-first"). |
| Collection metadata source | The **published `collection.json`** | Already fetched during traversal; ids always match; no registry dependency and no filename→id mapping problem. |
| Run scope | All collections; resumable; filterable | `--collection`, `--limit-per-collection`, `--skip-existing`. |
| Language | **Python + uv** | Orchestration only — all format logic stays in Rust. Precedent: `bench/plot` is already a uv project. |
| Item enumeration | **Reconcile, prefer the listing on mismatch** | Keeps the fast path where it is correct; can never silently truncate (§5.2). |
| Japan whole-city ZIPs | **Skip; convert the 60,090 tiles only** | The 381 ZIPs are the same data re-packaged. Tiles give finer granularity, smaller packages, and better row-group-pruning benchmarks. Recorded as `duplicate_bundle`, not as a failure. |
| `american-cities-3d` | Skip + log | Geographic-CRS support is a real encoding change (quantisation must adapt from 1 mm to ~1e-7°) deserving its own TDD cycle. |
| Downloaded sources | Delete after each item | Peak disk ≈ parallelism × largest file. `--keep-downloads` for debugging. |
| Converter fixes in scope | CityGML-version error message; `--crs` override; `srsName` fallback | All three are small, testable, and each unblocks or clarifies real datasets. |
| Reporting | JSONL ledger + `summary.csv` | Machine-readable; feeds the paper directly. |

## 4. Architecture

```
cityparquet-rs/
├── vendor/city3d-stac-tool/              # NEW submodule
├── tools/catalog2cityparquet/            # NEW uv project
│   ├── pyproject.toml
│   └── src/catalog2cityparquet/
│       ├── __main__.py     # CLI, orchestration, summary
│       ├── discover.py     # catalog → collections → items (+ reconciliation)
│       ├── fetch.py        # download, sniff, decompress, extract
│       ├── convert.py      # invoke `cityparquet convert`, stamp metadata.json
│       ├── aggregate.py    # invoke `city3dstac` for collections + catalog
│       └── ledger.py       # JSONL records, summary.csv, histogram
└── justfile                              # catalog-convert recipes
```

Each module has one job and a narrow interface: `discover` yields `Item` records,
`fetch` turns an `Item` into local convertible paths, `convert` turns those into a
package directory, `aggregate` turns package directories into STAC. None imports
another's internals, so each is testable alone.

### Why Python is not a compromise here

The driver does no format work. It resolves URLs, moves bytes, sniffs magic
numbers, shells out, and tallies results. Every byte of CityJSON/CityGML/Parquet
semantics stays in the Rust crates where the spec lives. Putting orchestration in
Rust would mean re-litigating retries, thread pools and archive handling in a
codebase whose discipline exists to protect *the encoding*, not a batch job.

## 5. Pipeline

### 5.1 Discover collections

Fetch `catalog.json`; follow `rel=child` links. `--collection ID` (repeatable)
filters.

### 5.2 Enumerate items — with reconciliation

Two independent sources, **both consulted**:

1. **`items.parquet`** — read remotely via DuckDB `read_parquet('https://…')`. No
   download; DuckDB range-requests the footer. `assets` is a struct whose keys vary
   per collection, so read it generically via `to_json(assets)` and pull
   `$.data.href` / `$.data.type`.
2. **GCS object listing** of the `<collection-id>/items/` prefix, **fully
   paginated** via `nextPageToken`.

**Policy:** compare the two counts. If they agree, use `items.parquet` (fast). If
they disagree, use the listing and record `stale_item_index` with both counts in
the ledger. If neither exists, fall back to `collection.json`'s `rel=item` links.

Pagination is mandatory, not an optimisation: the API caps a page at 1000 objects
and Japan has 60,471, so ignoring `nextPageToken` truncates to 1.6%. Both the
pagination and the reconciliation get direct tests — these are silent-wrong
failures, the only kind the ledger cannot catch.

The fetched `collection.json` is cached; it is the metadata seed for §5.5.

### 5.3 Fetch and normalise (parallel, bounded)

Per item, into a temp dir:

1. `GET` the `data` asset href with a browser User-Agent, redirects followed,
   generous per-phase timeouts, bounded retries with backoff.
2. **Sniff magic bytes** (`PK\x03\x04`, `\x1f\x8b`) to classify. Never trust the
   declared media type or the URL extension.
3. Decompress gzip; extract zip **recursively** (bounded depth, guarded against
   path traversal and against zip bombs by an uncompressed-size cap).
4. Select convertible members: `.json`, `.jsonl`, `.city.json`, `.gml`, `.xml`.
   `cityparquet convert` accepts multiple inputs and merges them into one dataset —
   exactly right for a multi-tile archive (proven on Japan: 136 GMLs, one
   invocation, 32,582 objects).
5. **Skip rule**: a `japan-plateau-3d` item whose id matches `*_citygml_*` is
   recorded `duplicate_bundle` and skipped before download — it is the whole-city
   re-packaging of tiles already being converted. This saves ~380 downloads
   totalling several hundred GB.
6. If nothing convertible is found → `unsupported_archive`, continue.

### 5.4 Convert and stamp

Run `cityparquet convert <inputs…> -o <out>/<coll>/items/<item-id> --overwrite`,
capturing stdout/stderr and wall time. On success, post-process `metadata.json`:

- set `collection` to the collection id;
- add links: `rel=collection`/`rel=parent` → `../../collection.json`, `rel=root` →
  `../../../catalog.json`;
- add `rel=via` → the original source asset URL and `rel=derived_from` → the
  upstream STAC item URL, so provenance survives;
- leave every `city3d:*` / `proj:*` / `cityparquet:*` property **untouched** — they
  are derived from the Parquet footer, and the spec makes the footer authoritative
  where Item and footer disagree.

Delete the temp dir (unless `--keep-downloads`).

The emitted Item's `id` already equals the package directory name, so naming the
directory after the STAC item id makes ids line up with no rewriting.

### 5.5 Aggregate

Per collection, translate the cached published `collection.json` into a config YAML
(id, title, description, license, keywords, providers, extent, summaries, links,
assets — all fields `CollectionConfigFile` already supports) and run:

```
city3dstac update-collection \
  --items-dir <out>/<coll>/items \
  --config <cached>.yaml \
  -o <out>/<coll>/collection.json \
  --geoparquet
```

Then, once every collection is done:

```
city3dstac update-catalog <out>/*/collection.json -o <out> --config <catalog>.yaml
```

Extent and summaries are recomputed by the tool from the *generated* items, so they
describe the CityParquet mirror rather than the source catalog. Note this means the
mirror's `items.parquet` will be accurate where the source catalog's is not.

## 6. Output layout

Mirrors the published catalog so the result can be served as a drop-in replacement:

```
out/cityparquet-catalog/
├── catalog.json
├── <collection-id>/
│   ├── collection.json
│   ├── items.parquet
│   └── items/
│       └── <item-id>/
│           ├── building.parquet          # + other per-family tables
│           ├── …
│           └── metadata.json             # the STAC Item
└── _reports/
    ├── <collection-id>.jsonl
    └── summary.csv
```

## 7. Failure isolation and resumption

- **Per item**: any exception → ledger record `{item_id, status, reason, error,
  bytes, seconds}`; the loop continues.
- **Per collection**: any exception → the collection is abandoned, recorded, and
  the driver moves to the next. This is the brief's hard requirement.
- **Exit code**: non-zero only when the catalog root itself is unreachable — i.e.
  when nothing could be attempted. A run with failures is a *successful run that
  measured failures*.
- **Resume**: `--skip-existing` (default on) skips an item whose
  `items/<id>/metadata.json` exists and parses as a STAC Item. This is why
  `--items-dir` (walking the output tree) is the correct aggregation primitive
  rather than a list of this run's successes — a resumed run must aggregate
  everything ever converted.
- **Conformance reason vocabulary** (closed set, so the histogram is
  meaningful): `download_failed`, `unsupported_archive`,
  `unsupported_citygml_version`, `unsupported_cityjson_version`, `no_crs`,
  `geographic_crs`, `convert_failed`, `empty_collection`, `duplicate_bundle`,
  `stale_item_index`. Every one of these is a statement about the *data*.
- **`environment`** is a status and reason of its own, outside that vocabulary:
  the run hit a *local* failure (a full disk, an unwritable `_configs`, a
  missing `city3dstac`, a broken stream) and the record says nothing about
  whether the dataset converts. It never enters the conformance histogram and
  gets its own column in `summary.csv`; without it every environment failure
  has to be recorded as `convert_failed`, which is how a run with a full log
  volume comes to publish half a catalogue as unconvertible.
- **How a local failure is recognised**: an `OSError` raised in this process,
  whether it reaches the item handler or the collection handler; and a
  host-failure marker in a *tool's* stderr (`no space left`, `read-only file
  system`, `disk quota exceeded`, `too many open files`), because a subprocess
  whose own volume filled exits non-zero exactly as it does when it refuses the
  data, and only its stderr tells the two apart. An `httpx` failure is always
  the origin's and is classified *before* the `OSError` test, so a transport
  failure wrapping a lower-level error keeps its conformance reason
  (`download_failed`) rather than being reclassified as this machine.

## 8. Changes to `cityparquet-rs`

### 8.1 Clear CityGML-version error

A CityGML 1.0 file currently reports `invalid CityJSON: expected value at line 1
column 1`, because `citygml::is_citygml` returns false for any non-2.0 namespace
and the file falls through to the CityJSON branch.

Change: detect a `CityModel` root element in **any** `opengis.net/citygml/*`
namespace; if the version is not 2.0, fail with `unsupported CityGML version 1.0
(only CityGML 2.0 is supported)`. Unlocks no data, but turns 11 collections'
failures from baffling into diagnostic — and it is the error a reference
implementation should have anyway.

### 8.2 `--crs` override

Add `--crs <EPSG:xxxx | PROJJSON path>` to `cityparquet convert`. When the source
declares no resolvable CRS **and** `--crs` is given, use the supplied CRS instead of
failing at `scan.rs:407`.

**Spec position.** The rule (`05-metadata.mdx`, "CRS rules") forbids two specific
things: writing with `crs` **absent**, and **guessing** a CRS. An explicit
operator-supplied CRS is neither — it makes the CRS resolvable before the writer
runs, exactly as an EPSG code in the source would. To keep the output honest about
provenance, record it in `city.other` (free-form and explicitly informational per
the spec), e.g. `other.crs_source = "operator-supplied"`. Absent `--crs`, behaviour
is unchanged. Flagged for sign-off in §12.

Unlocks `luxembourg-3d`, `new-york-doitt-3d`, `vienna-3d`, `zurich-3d`.

### 8.3 `srsName` fallback beyond the preamble

`scan_envelope` (`crates/cityparquet/src/citygml/header.rs:63`) stops at the first
`cityObjectMember`, on the assumption that a CityGML envelope always precedes it.
Freiburg's 1.86 GiB export has no `CityModel`-level `gml:boundedBy`; its first
`cityObjectMember` starts at byte 2537, and `srsName="urn:ogc:def:crs:EPSG::25832"`
appears **60,108 times** — once per building — but never in the preamble. The
scanner therefore sees no CRS and the §8.2 rule hard-fails a file that plainly
declares one.

Change: when the preamble yields no `srsName`, continue scanning into city objects
and adopt the first `srsName` found; fail only if none exists anywhere. Verified:
splicing a root envelope into a scratch copy makes the current binary convert the
file cleanly → **131,036 objects**, `building.parquet` 46.5 MB, ~60 s.

This is strictly better than forcing `--crs` for this case: the CRS comes from the
data, so no operator provenance stamp is warranted.

### 8.4 Discipline

All three follow the repo's strict red-green TDD: failing test first, smallest
change to pass, then refactor. Fixtures are real files, never inlined — a CityGML
1.0 fixture and a preamble-less CityGML fixture are added to `just fixtures`.
`just check` (clippy `-D warnings`, tests, schema isolation, fmt) must be green.

## 9. Change to `city3d-stac-tool`

Add `--items-dir <DIR>` to `update-collection`, composing with the existing
positional `items` and `--items-from-file` (the union of all three is used).

Semantics: walk `DIR` recursively; accept any `.json` file that parses as a STAC
Item (`type == "Feature"` with a `stac_version`); skip `collection.json` and
`catalog.json`. Deliberately *not* a filename glob, because `cityparquet` names its
Item `metadata.json` while the tool's own convention is `*_item.json` — content
sniffing handles both and whatever a future writer chooses.

**Backward compatibility is preserved**: the flag is additive and optional; every
existing invocation behaves identically. This matters because the registry repo and
its CI already depend on this tool.

No other tool change is needed. `CollectionConfigFile` already carries
id/title/description/license/keywords/providers/extent/summaries/links/assets — a
superset of what a published `collection.json` holds — and is serde-serialisable,
so the driver emits the config YAML itself.

The submodule is added at `vendor/city3d-stac-tool`, tracking a branch with this
change; the workspace's existing `city3d-stac-types` git dependency is repinned to
the same revision so driver and converter agree on STAC types.

## 10. `just` recipes

```
# Convert the whole published catalog (resumable)
catalog-convert OUT='out/cityparquet-catalog' *ARGS

# Convert a single collection
catalog-convert-collection ID OUT='out/cityparquet-catalog' *ARGS

# Re-aggregate collections + catalog from an existing output tree, no downloads
catalog-aggregate OUT='out/cityparquet-catalog'
```

`*ARGS` forwards `--jobs`, `--limit-per-collection`, `--keep-downloads`, `--crs`,
`--no-skip-existing`.

Default `--jobs` is `min(8, cpu_count)`. Politeness to origin servers, not local
capacity, is the binding constraint: several collections are single-origin, and the
probe already saw one 403 and one 11-minute download. Japan's 60,090 tiles are the
long pole; at 8 jobs and ~1 s/tile plus download this is hours, not minutes, which
is what `--skip-existing` is for.

## 11. Testing

**Driver (pytest, in the uv project).** Tests run against a local fixture tree
served over `http://127.0.0.1` by a throwaway server — no network:

- `discover`: items.parquet path; **paginated** listing fallback; collection.json
  fallback; **reconciliation** (stale index → listing wins, discrepancy recorded).
- `fetch`: gzip, zip, **nested zip**, plain; magic-byte sniffing overriding a lying
  media type (the hamburg case); zip-bomb cap; path-traversal rejection;
  `duplicate_bundle` skip rule.
- `convert`: metadata.json stamping (collection, links added; properties untouched).
- `ledger`: closed reason vocabulary, summary roll-up.
- **Failure isolation**: a collection that raises must not stop the next one — the
  brief's central requirement gets a direct test.

Fixtures reuse the repo's real CityJSON/CityGML in `tests/fixtures/`; archives are
built from those real files at test time.

**`--items-dir`**: a test in the tool repo asserting recursive discovery of
`metadata.json`-named Items, and that existing positional / `--items-from-file`
invocations are unaffected.

## 12. Open points needing sign-off

1. **The `--crs` spec reading (§8.2).** An operator-supplied CRS is argued to be
   compliant because it is neither an absent CRS nor a guess. On a stricter reading,
   drop `--crs` and those four collections stay unconverted. Note §8.3 removes the
   *Freiburg* need for it entirely.
2. **Per-collection CRS table.** Sourcing the override from each collection's
   published `proj:code` summary is a convenience; it trusts registry metadata the
   source file itself does not carry.

## 13. Non-goals and follow-ups

Out of scope here, tracked as separate work:

- CityGML **1.0 / 3.0** reader support — unlocks 11 collections and most of the
  German national LoD2 corpus. By collection count, the highest-value follow-up in
  the backlog.
- **Geographic-CRS** support — unlocks `american-cities-3d` (5,229 items).
- **CityJSON 1.0** reader support — unlocks `singapore-hdb-3d`.
- Uploading the generated mirror to object storage; this design writes a local tree.
- Upstream data problems (`linz-3d` 404s; 20 collections publishing zero items;
  `japan-plateau-3d`'s stale `items.parquet`) — those belong in the registry repo.
  The stale index is worth reporting there: the published catalog under-represents
  Japan by a factor of 200.
