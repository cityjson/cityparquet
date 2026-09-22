# Bloom filters in CityParquet — design

Status: draft for review (2026-09-22). Supersedes the "bloom filter on `id` —
out of scope" note in `2026-08-25-readbench-query-design-design.md`.

## Problem

`query::id_lookup` and `query_async::id_lookup_async` decode the `id` column of
every row group until the first match — **all** of them on a miss: the row filter is an opaque Arrow closure, and `id` min/max
statistics cannot prune — on the 1M-row 3DBAG slice every one of the 16 row
groups spans roughly `NL.IMBAG.Pand.0202…`–`…1945…`, and Hilbert ordering makes
this worse. Nothing in the stack writes or reads Parquet bloom filters today
(`bloom_filter_offset` is NULL on every column of every file).

## Decisions (agreed 2026-09-22)

1. **Columns:** module-table `id` and `feature_id`, plus scalar string attribute
   columns detected as high-cardinality at write time (rule below). Everything
   else off. Sidecars off.
2. **FPP:** 0.01 default, configurable; the benchmark sweeps it.
3. **Placement:** all filters together after the last row group, before the page
   indexes and footer (`BloomFilterPosition::End`), with `bloom_filter_offset`
   and `bloom_filter_length` set — DuckDB's layout.
4. **Async reader:** one coalesced range request for all filters it needs.
5. **Default:** on in the default `CityParquet` recipe; explicitly disableable.
6. **Scope:** both writers (cityparquet-rs and duckdb-cityjson) write filters
   under the same policy; `feature_id` lookup is in scope (it is the join key
   between parts and their feature, used as often as `id`).
7. **Benchmark:** one `bloom` family, one pair — with and without filters.
8. **Crates, not hand-rolled code:** parquet-rs does sizing, hashing, folding,
   serialisation and metadata; the reader decodes with `Sbbf::from_bytes` /
   `get_row_group_column_bloom_filter` and probes with `Sbbf::check`.

We follow DuckDB's **placement**, not its **column rule**: DuckDB writes a
filter only for dictionary-encoded chunks, and our `id`/`feature_id` are
DELTA_BYTE_ARRAY with dictionary off (`recipe.rs:337-343`), so that rule would
skip exactly the columns that matter.

## Writer

### Column policy

| Column | Filter |
|---|---|
| `id`, `feature_id` (every module table) | always (when bloom is enabled) |
| Scalar `Utf8` attribute columns, not tagged `arrow.json` | if high-cardinality (below) |
| `object_type`, Boolean/Int64/Float64/Date32/Timestamp attributes | never |
| `List<Utf8>` attributes, `parents`/`children`/`children_roles`, `address.*`, `template.*`, material/texture maps | never |
| `bbox.*`, `geometry*`, `geometry_properties*`, JSON attributes, `other` | never |
| Sidecars (`materials`, `textures`, `geometry_templates`) | never — `sidecar_writer_properties()` unchanged |

Numeric attributes are excluded on purpose: the existing Int64 equality predicate
compares through `f64` (`query_core.rs:138`), so an exact-integer bloom probe
could reject a row the predicate would accept (false negative above 2^53).

### High-cardinality rule (proposed — please confirm)

During the existing single `scan` pass (`scan.rs`, where the attribute type
inferer already `observe`s every value), keep for each candidate `Utf8`
attribute column a non-null count and a distinct-count estimate. At the end of
the scan a column qualifies when

    estimated_distinct >= 0.2 × non_null_count

and `non_null_count > 0` (an all-null column, which inference types as `Utf8`,
never qualifies). This is *analogous to*, not identical with, DuckDB's
dictionary cut-off (row-group rows / 5): ours is dataset-wide and over non-null
values, so a sparse column or a vocabulary repeated across row groups can be
decided differently than a per-row-group rule would — an accepted limitation,
since `WriterProperties` are fixed before the first row group is written. Low-cardinality
columns stay in dictionary + statistics territory, where a filter adds little.

The estimate uses a HyperLogLog crate (candidates: `cardinality-estimator`,
`hyperloglogplus`; implementer picks on maintenance and licence and reports)
so memory stays bounded at ~100 attribute columns × 1M objects; precision
around 2 % standard error (e.g. HLL p = 12) is ample for a 0.2 threshold. On 3DBAG this
selects `identificatie` (4,999 distinct of 4,999 non-null) and not `status`.

The qualifying names are a new `ScanResult` field, **intersected with the
final string attribute columns after collision diversion** (`scan.rs:473`).
`writer_properties` currently receives only `&scan_result.schema`
(`package.rs:1396`); it gains a second argument, `bloom_attributes:
&BTreeSet<String>`, passed from `scan_result`. Attribute columns are
top-level, single-component paths — `ColumnPath::new(vec![name.clone()])`,
even when the name contains a literal `.` (`model.rs:454`). The decision is dataset-wide; every module
file of the package shares one `WriterProperties`, and settings for columns
absent from a module's projection are inert (current behaviour).

### Properties

In `WriterRecipe::writer_properties` (`recipe.rs:267`), for each selected leaf
`ColumnPath` (built with `ColumnPath::new(vec![..])`, never dotted
`ColumnPath::from`): `set_column_bloom_filter_enabled(path, true)` and
`set_column_bloom_filter_fpp(path, fpp)`; globally
`set_bloom_filter_position(BloomFilterPosition::End)`. NDV is **not** set:
parquet-rs resolves it to `max_row_group_row_count` (the recipe already sets
it) and `fold_to_target_fpp` shrinks each filter to the values actually present.

Writer memory: `End` keeps every completed filter in memory until the file is
closed (`parquet writer.rs:262,333`), per open module writer. Report peak RSS
with and without bloom in the benchmark; no mitigation unless it matters.

`WriterRecipe` gains `bloom: BloomPolicy { enabled: bool, fpp: f64 }`
(default `{ true, 0.01 }`). The `ParquetDefaults` preset returns before any
per-column tuning and therefore writes no filters (it means "parquet-rs
defaults"). `NoDictionary` and `CityParquet` honour the policy.

### Surfaces

- CLI `convert`: `--no-bloom`, `--bloom-fpp <f64>` (0 < fpp < 1, validated
  with a clear error, not the parquet-rs panic).
- Variant grammar (`variant.rs`): one new token, `+nobloom`. `id()` must
  round-trip. (No FPP token: the benchmark does not sweep FPP.)

## Reader

The arrow builders keep their input reader and row-group selection private, so
pruning is **free functions in `query_core`/`query`/`query_async`**, run
**before** the builder is configured, on the builder's public
`metadata()` and a reader the caller retains:

    // shared, pure: which leaf, which (rg, offset, length) to fetch
    fn bloom_targets(meta: &ParquetMetaData, column: &ColumnPath,
                     candidates: &[usize]) -> BloomTargets
    // sync: local file (ChunkReader), one read per row group
    fn bloom_keep_row_groups(file, meta, column, values, candidates) -> BloomPrune
    // async: a separately held AsyncFileReader (e.g. a second ParquetObjectReader
    // on the same store/path — cheap), all filters in one get_byte_ranges call
    async fn bloom_keep_row_groups_async(reader, meta, column, values, candidates) -> BloomPrune

`BloomPrune { keep: Vec<usize>, total, pruned, without_filter }`; `keep` goes to
`builder.with_row_groups(keep)`. Candidates are explicit (all row groups for
`id_lookup`; the bbox-surviving set when composed with a bbox query).

- Resolve the **Parquet leaf index by column path** from the file schema, not
  the Arrow field ordinal.
- No filter / no offset → **keep**. Filter present → keep iff `sbbf.check(v)`
  is true for **any** value (IN semantics). Probe with the raw UTF-8 bytes
  (`&str`).
- The exact `RowFilter` stays: a positive bloom result is never a match.

**Sync** (`query.rs`): `Sbbf::read_from_column_chunk` or
`get_row_group_column_bloom_filter` per candidate — local `pread`s.

**Async** (`query_async.rs`): collect `(offset, length)` of the target column's
filter in every candidate row group from metadata and fetch them with **one
`AsyncFileReader::get_byte_ranges(ranges)` call**; `ParquetObjectReader`
forwards that to `object_store`'s `get_ranges`, which coalesces nearby ranges.
Decode each with `Sbbf::from_bytes`. Under `End` placement the writer lays out
filters row group by row group, then column by column (`parquet
writer.rs:333,482`), so successive `id` filters are separated only by that row
group's other filters (`feature_id`, qualifying attributes) — small gaps that
coalesce. If a candidate lacks `bloom_filter_length`, fall back to
`get_row_group_column_bloom_filter` for that row group.

**Diagnostics.** `id_lookup`/`id_lookup_async` keep their signatures; new
`id_lookup_with_stats`/`id_lookup_async_with_stats` return
`(Option<DecodedObject>, LookupStats { row_groups_total, bloom_pruned,
filter_bytes })`, where `filter_bytes` is the bitset bytes of every filter
examined. There is deliberately no "row groups read" counter: a batch carries
no row-group index, and reading one builder per row group to count them would
change the read pattern behind the existing cross-format id-lookup figures
(decision 2026-09-22). A hit still stops at its first match. The readbench adapter
(`benchmark/readbench/src/formats/cityparquet.rs:264`) calls the `_with_stats`
form and writes the counters into its result record; the plain forms delegate.

### `feature_id` lookup (new)

`query::feature_lookup(table_path, meta, feature_id) -> Vec<Object>` and
`query_async::feature_lookup_async(..)`, plus `_with_stats` forms returning
`LookupStats`. Unlike `id_lookup` it returns **every** row with that
`feature_id` (a feature and all its parts), so it never stops early: it
bloom-prunes on `feature_id`, then reads the surviving row groups to the end
with an exact equality `RowFilter`. It is table-level, like `id_lookup`;
a package-level form iterates the module tables named in the package's
`metadata.json`. CLI: a `lookup --feature-id <ID>` form beside the existing
id lookup, if the CLI has one (implementer checks and mirrors it).

For SQL joins on `feature_id`/`id` through DuckDB, nothing is added in the
reader: DuckDB already consults bloom filters for `=`/`IN` filters, including
those it pushes down from a join's build side.

**Call sites:** `id_lookup*` (column `id`), `feature_lookup*` (column
`feature_id`), and `attr_filter` when the
predicate is `Utf8` equality on a column that carries a filter. Fix
`attr_filter`'s doc comment, which wrongly claims statistics pruning today
(`query.rs:106-116`).

## Specification (documents/)

`03-specification/02-object-table-schema.mdx`, next to the physical-encoding
freedom (`:147-160`):

> Writers MAY include standard Parquet bloom filters. Writers targeting
> selective identifier lookups SHOULD include filters on `id` and `feature_id`.
> Filters MUST conform to the Parquet bloom-filter specification; writers
> SHOULD set `bloom_filter_length`. Readers MUST accept files without filters,
> MUST NOT depend on filter placement, and MUST NOT treat a positive bloom
> result as a match.

`05-metadata.mdx` "Recommended writer defaults (informative)": FPP 0.01,
filters placed after the last row group, the high-cardinality rule.
`04-design-decisions/`: a short decision page (why `id`/`feature_id`, why not
DuckDB's dictionary rule, why End placement). `06-resources/02-software.mdx`:
the duckdb-cityjson divergence described below.

## duckdb-cityjson writer

Same policy, written through DuckDB's `COPY … (FORMAT PARQUET, …)` in
`src/cityjson/cityparquet_write.cpp:630-633`. Constraint, verified on DuckDB
v1.5.4 (the submodule's version) with 200k 3DBAG rows at 65,536-row groups:
DuckDB writes a filter only for dictionary-encoded chunks, and its bloom and
dictionary options (`WRITE_BLOOM_FILTER`, `BLOOM_FILTER_FALSE_POSITIVE_RATIO`
default 0.01, `DICTIONARY_SIZE_LIMIT` default rows/5) are **file-wide**, with no
per-column control.

| COPY options | Filters on | Cost |
|---|---|---|
| defaults | `object_type`, `status` (low-card) — not `id`, `feature_id`, `identificatie` | — |
| `DICTIONARY_SIZE_LIMIT` ≥ row-group rows | every column, geometry WKB included | `id` +39 %, `identificatie` +32 %, WKB dictionary-encoded |

**Decision (2026-09-22): COPY options, with the divergence documented.** The
object-table `COPY` gains

    DICTIONARY_SIZE_LIMIT <row-group rows>, STRING_DICTIONARY_PAGE_SIZE_LIMIT 8388608,
    WRITE_BLOOM_FILTER true, BLOOM_FILTER_FALSE_POSITIVE_RATIO 0.01

where `<row-group rows>` is the row-group size that `COPY` uses (DuckDB's
default unless the write function sets one — the implementer makes the two
agree explicitly). Verified on the same 200k rows: filters on `id`,
`feature_id`, `identificatie` and the other short strings; WKB geometry and the
`surfaces` JSON stay PLAIN without a filter because their dictionary page
exceeds the 8 MiB cap (except a small final row group that fits under it);
cost ≈ +0.4 MB on a 98 MB file (`id` dictionary- rather than plain-encoded).

The resulting column set is a **superset** of the Rust policy: DuckDB also
filters low-cardinality strings (as it does by default) and lists of short
values. That difference, and the occasional tail-row-group blob filter, go into
`06-resources/02-software.mdx`. The write function gains a `bloom` option
(default true) mapping to `WRITE_BLOOM_FILTER`, mirroring `--no-bloom`.
Sidecar `COPY`s are unchanged. Reading needs no change: DuckDB already uses
filters for `=`/`IN` pushdown. Work happens in the `lib/duckdb-cityjson`
submodule (its own `CLAUDE.md`, its own commits; the monorepo bumps the
pointer).


## Benchmark — a `bloom` family

Beside `codec` (codec choice) and `rowgroup` (row-group size), a third family,
`bloom` (effect of bloom filters on query performance), on the same machinery
(`manifest.toml`, root `justfile` `variant-bench`, `bench_suite.py` family
map, `bench_recipe_test.sh`, `benchviz/prep.py`, `figures.py`):

- **Variants:** `cityparquet` (filters, the default) and
  `cityparquet+nobloom`. Nothing else.
- **Scenarios:** `id-lookup` with probes `id-50pct` and `id-miss`, and the new
  `feature-lookup` with probes `feature-50pct` and `feature-miss` (a verified
  absent `feature_id`, generated like `id-miss`).
- **Datasets:** the 3DBAG scaling slices (the multi-row-group ones matter:
  n100000 upward) and the corpus datasets.
- **Metrics:** latency (local and HTTP), bytes and requests
  (`CountingObjectStore`), row groups total / bloom-pruned, filter bytes and
  package-size overhead, write time.
- **Disclosed caveats:** id lookup fetches metadata twice
  (`benchmark/readbench/src/formats/cityparquet.rs:264`) — equal across
  variants, stated not hidden; the runner reads single-table packages
  (`cityparquet.rs:40`); `CountingObjectStore` counts logical requests.

**Knock-on effect:** with bloom on by default, the existing `codec` and
`rowgroup` baselines now carry filters and their `id-50pct` numbers use bloom
pruning. Their published figures must be re-run before they are compared with
the new ones; the family READMEs say so.

Generated inputs and results go under `benchmark/runs/` only.

## Acceptance

1. DuckDB `parquet_metadata()` on a converted 3DBAG package: non-null
   `bloom_filter_offset` and `bloom_filter_length` on exactly `id`,
   `feature_id` and the qualifying attributes; every offset past the last row
   group's data; none on sidecars.
2. No false negatives: every `id` of `3dbag_n10000` written at `+rg512` is
   found by `id_lookup` and `id_lookup_async`.
3. A miss on the 1M slice written with `--row-group-size 512` (a test, not a
   benchmark variant): bloom-pruned row groups ≥ 95 % of 1954, for both `id`
   and `feature_id`.
4. The async lookup fetches filters with one `get_byte_ranges` call, and
   `CountingObjectStore` shows filter requests far fewer than row groups (same
   file: ≤ 8 requests for 1954 filters).
4b. `feature_lookup` returns every row of a multi-part 3DBAG feature (Building
   plus its BuildingParts), identical with and without filters.
4c. A duckdb-cityjson-written package passes check 1 under the chosen option.
5. `+nobloom` files are byte-identical in layout to today's (no filter bytes).
6. `cd lib/cityparquet-rs && just check`, `just plot-test`,
   `just scripts-test` and the readbench gate pass.

## Out of scope

Numeric-attribute filters (blocked on typed Int64 equality), sidecar filters,
page-index pruning, and an FPP sweep.
