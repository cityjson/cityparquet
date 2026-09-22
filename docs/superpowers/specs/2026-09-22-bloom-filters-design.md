# Bloom filters in CityParquet — design

Status: draft for review (2026-09-22). Supersedes the "bloom filter on `id` —
out of scope" note in `2026-08-25-readbench-query-design-design.md`.

## Problem

`query::id_lookup` and `query_async::id_lookup_async` decode the `id` column of
**every** row group: the row filter is an opaque Arrow closure, and `id` min/max
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
6. **Crates, not hand-rolled code:** parquet-rs does sizing, hashing, folding,
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

i.e. exactly where DuckDB would *give up* the dictionary (its default limit is
row-group rows / 5) and therefore write no filter at all. Low-cardinality
columns stay in dictionary + statistics territory, where a filter adds little.

The estimate uses a HyperLogLog crate (candidates: `cardinality-estimator`,
`hyperloglogplus`; implementer picks on maintenance and licence and reports)
so memory stays bounded at ~100 attribute columns × 1M objects. On 3DBAG this
selects `identificatie` (4,999 distinct of 4,999 non-null) and not `status`.

The qualifying column names are carried on `ScanResult` (new field) into
`WriterRecipe::writer_properties`. The decision is dataset-wide; every module
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

`WriterRecipe` gains `bloom: BloomPolicy { enabled: bool, fpp: f64 }`
(default `{ true, 0.01 }`). The `ParquetDefaults` preset returns before any
per-column tuning and therefore writes no filters (it means "parquet-rs
defaults"). `NoDictionary` and `CityParquet` honour the policy.

### Surfaces

- CLI `convert`: `--no-bloom`, `--bloom-fpp <f64>` (0 < fpp < 1, validated
  with a clear error, not the parquet-rs panic).
- Variant grammar (`variant.rs`): `+nobloom`, and `+fpp<N>` with N in
  thousandths (`+fpp50` = 0.05, `+fpp10` = 0.01, `+fpp1` = 0.001) — no `.` in a
  variant id, because ids become file names (`<name>.<variant>.parquet`).
  `id()` must round-trip.

## Reader

A new `CityParquetReaderBuilder` method beside `with_bbox_row_groups`
(`reader.rs:178`):

    with_bloom_row_groups(column_path, &[values]) -> keep: Vec<usize>

- Resolve the **Parquet leaf index by column path** from the file schema, not
  the Arrow field ordinal.
- For each row group (after any row-group selection already applied): no
  filter / no offset → **keep**; filter present → keep iff `sbbf.check(v)` is
  true for **any** value (IN semantics). Probe with the raw UTF-8 bytes
  (`&str`).
- The exact `RowFilter` stays: a positive bloom result is never a match.
- Returns counts for reporting (row groups total / bloom-pruned / kept),
  mirroring `bbox_row_group_counts` (`query_core.rs:248`).

**Sync** (`query.rs`): `get_row_group_column_bloom_filter(rg, col)` per row
group — a local file, one `pread` each.

**Async** (`query_async.rs`): from the already-loaded metadata, collect
`(offset, length)` of the target column's filter in every candidate row group;
issue **one** `get_bytes(min_offset..max(offset+length))` on the
`AsyncFileReader`; slice each filter and decode with `Sbbf::from_bytes`. If any
candidate lacks `bloom_filter_length`, fall back to
`get_row_group_column_bloom_filter` per row group. If the span would exceed a
bound (e.g. 8 MiB — filters are not contiguous if a foreign writer used
`AfterRowGroup`), fall back to per-row-group fetches.

**Call sites:** `id_lookup` / `id_lookup_async` (column `id`), and
`attr_filter` when the predicate is `Utf8` equality on a column that carries a
filter. Fix `attr_filter`'s doc comment, which wrongly claims statistics
pruning today (`query.rs:106-116`).

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
the divergence that duckdb-cityjson (DuckDB `COPY`) writes filters only on
dictionary-encoded columns, so not on `id`.

## Benchmark — a `bloom` family

Beside `codec` and `rowgroup`, same machinery (`manifest.toml`, root `justfile`
`variant-bench`, `bench_suite.py` family map, `bench_recipe_test.sh`,
`benchviz/prep.py`, `figures.py`):

- **Variants** (pairs, all else equal):
  `cityparquet+nobloom`, `cityparquet`,
  `cityparquet+rg8192+nobloom`, `cityparquet+rg8192`,
  `cityparquet+rg512+nobloom`, `cityparquet+rg512`,
  `cityparquet+fpp50`, `cityparquet+fpp1`.
- **Scenarios:** `id-lookup` with probes `id-50pct` **and `id-miss`**; the
  miss id is verified absent and should fall inside the observed `id` range.
- **Datasets:** the 3DBAG scaling slices (the multi-row-group ones matter:
  n100000 upward, and n1000000) and the corpus datasets.
- **Metrics:** latency (local and HTTP), bytes and requests
  (`CountingObjectStore`), row groups bloom-pruned / scanned, empirical false
  positives, total filter bytes and package-size overhead, write time.
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
3. `id-miss` on the 1M slice at `+rg512`: bloom-pruned row groups ≥ 95 % of 1954.
4. The async lookup issues one filter request (asserted through
   `CountingObjectStore`) for a file written by this writer.
5. `+nobloom` files are byte-identical in layout to today's (no filter bytes).
6. `cd lib/cityparquet-rs && just check`, `just plot-test`,
   `just scripts-test` and the readbench gate pass.

## Out of scope

`feature_id` lookup API (its filter is written now; a lookup path is later),
numeric-attribute filters (blocked on typed Int64 equality), sidecar filters,
page-index pruning, and changing duckdb-cityjson's writer.
