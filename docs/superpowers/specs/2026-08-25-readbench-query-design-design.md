# Read-benchmark query design: id-lookup probes and real-selectivity bbox windows

Date: 2026-08-25
Area: `benchmark/readbench`, `benchmark/scripts/readbench_duckdb.sh`,
`benchmark/formats/READ_BENCHMARK.md`

## The problem

Two of the read benchmark's seven scenarios are measured with query
parameters that do not exercise what the scenario claims to measure.

**`bbox-query` windows are frequently empty.** `coordinator.rs`'s
`bbox_window` takes 1%/5%/25% of the dataset bbox's x/y _extent_, anchored at
its **lower-left corner**. A bounding-box corner is rarely where the objects
are, so on a diagonal or irregular tile the smallest windows enclose empty
space.

Across all six datasets in `read_results/`, `bbox-1pct` returns **zero rows
every time**. `bbox-5pct` returns zero on four of the six (`ingolstadt`,
`rotterdam_delfshaven`, `vienna_102081`, `zurich_building_lod2`), and 2 and 17
rows on the other two. `bbox-25pct` never reaches its nominal fraction: it
captures 10.9% on `nyc_da13_buildings`, 0.27% on `3dbag_9-284-556`, and
0.0091% — 18 rows of roughly 199,000 — on `zurich_building_lod2`. The
committed `scaling_read_results/` show the same shape: `bbox-1pct` zero on
every format and every cardinality, `bbox-25pct` capturing 2.4% of rows.

Every count above holds for **all five formats**, not merely CityParquet: on
each of those ten window-and-dataset combinations, `citygml`, `cityjson`,
`cityjsonseq`, `flatcitybuf` and `cityparquet-hilbert` all return zero. A
feature-grain bbox is the union of its objects' extents and so could in
principle intersect a window no single object touches, but on this corpus none
does.

So of the eighteen bbox measurements in the local read results, ten timed an
empty result, and none measured the selectivity its tag names. (Those CSVs are
not committed — `benchmark/README.md` says so — but the committed scaling
family shows the same defect.) The `selectivity` column records an accident of
tile shape rather than a controlled variable, and the three windows do not form
a selectivity axis.

**`id-lookup` conflates the mechanism with the target's position.** The
coordinator picks one target — the first non-null `id` in the CityParquet
table — and each runner answers it by a different convention.
`formats/cityjsonseq.rs` early-returns on the first hit; `formats/citygml.rs`
deliberately does not, because `stream_members` ends with an
`ensure_every_member_was_mapped` integrity guard. The result is that
CityJSONSeq's published `id-lookup` time is a function of where that one id
happens to sit in the stream, and is not comparable with CityGML's.

Both defects are in the _parameters_, not the runners: the scenario dispatch
in each runner does genuinely different work per scenario. Where a format has
a mechanism that escapes the parse floor, the CSV already shows it —
FlatCityBuf answers `count` from its header in 0.0001 s against a 0.037 s full
read, and CityParquet differentiates every scenario.

## Scope

In scope: the query parameters both scenarios are driven with, the mechanism
each runner is permitted to use for `id-lookup`, where parameter derivation
lives, and the resulting re-runs and methodology edits.

Out of scope, deliberately:

- **A bloom filter on the `id` column.** CityParquet's writer emits none
  (`recipe.rs` sets no `set_column_bloom_filter_enabled`) and
  `query::id_lookup` is an unpruned `RowFilter` over the whole id column.
  Adding one changes what a CityParquet package _is_, so it belongs in its own
  design, argued from the `id-miss` figure this work produces — not decided in
  the same change as the benchmark that motivates it.
- **`lib/cityparquet-rs/crates/cli/src/bench.rs`.** It carries a third copy of
  the lower-left window construction, but it is a different tool answering a
  different question (row-group pruning effectiveness at a given
  `--window-frac`). It shares the empty-window blind spot and should be named
  in the bloom-filter follow-up.

## Design

### `id-lookup`: four probes per format

The scenario emits four rows per format, distinguished by a `notes` tag —
the same mechanism the three bbox windows already use:

| tag        | target                                 |
| ---------- | -------------------------------------- |
| `id-10pct` | the id at 10% of the canonical order   |
| `id-50pct` | the id at 50%                          |
| `id-90pct` | the id at 90%                          |
| `id-miss`  | an id verified absent from the dataset |

**The canonical order is the CityJSONSeq stream.** `readbench_prepare.sh`
cuts the gzipped, FlatCityBuf and CityParquet artefacts from that one seq
file, so it is genuinely their parent order. At decile _p_ the probe is the
feature at index `floor(p * n_features)`, identified by that feature's own
top-level `id` — which is also a key in its `city_objects` map, so the same
string resolves in every format and names a real row in the CityParquet table.

The `id-miss` target is derived from the 50% id with a non-colliding suffix
and then **verified absent** against the id column during derivation. It is
never assumed absent.

Each decile probe is verified **present** in both the CityParquet table and
the CityGML artefact. The CityGML is synthesised separately by `citygml-tools`
and `benchmark/README.md` already records that `3dbag_9-284-556` loses an LoD
in that round trip, so its member set is not guaranteed to match the seq
stream's. Without that check a decile id absent from the CityGML would be
timed as a hit and silently recorded as a miss. A probe that fails
verification is replaced by the nearest feature that passes, and the
substitution is noted.

Derivation therefore requires the **CityJSONSeq artefact as well as the
CityParquet package**, extending the coordinator's existing contract that the
CityParquet package is required regardless of `--formats`. Both are named
requirements, and a missing one is a hard failure at derivation time rather
than a fabricated parameter.

Each runner uses the best mechanism its format affords:

| format                               | mechanism                                  | position-sensitive |
| ------------------------------------ | ------------------------------------------ | ------------------ |
| `cityjsonseq`, `cityjsonseq-gz`      | early return on first hit                  | yes                |
| `citygml`                            | early return on first hit                  | yes                |
| `cityjson`                           | hash lookup after the whole-document parse | no                 |
| `flatcitybuf`                        | B+-tree attempt, then full walk            | yes                |
| `cityparquet`, `cityparquet-hilbert` | `RowFilter` over the id column             | no                 |

CityGML gains the early exit it does not have today. Its
`ensure_every_member_was_mapped` guard is a property of the _artefact_, not of
any one scenario, and the coordinator already spawns an **untimed** `Count`
child per format per dataset (`total_count_for`) whose full pass exercises it.
The guard therefore keeps running once per format per dataset; only the timed
`id-lookup` pass stops early.

FlatCityBuf's fallback stays, and stays disclosed. The artefacts are built
`fcb ser -A`, indexing every attribute, but `id` is a CityObject's map key and
never a member of the CityJSON `attributes` map FCB's schema covers. It is
structurally unindexable in FCB as the format stands, and `id-miss` is where
that shows honestly: every format without an id index pays a full pass.

### `bbox-query`: windows that hit a target row fraction

Targets become **1%, 5% and 25% of CityParquet rows** rather than of area.

During the pass that already scans the `bbox` column for the dataset extent,
the derivation also collects one centroid per row. It then binary-searches the
half-width of a window centred on the **median centroid**, with the x/y aspect
scaled to the dataset's own extent ratio so a long, thin tile does not receive
a degenerate window. The `z` span stays the dataset's full range, which
`readbench_duckdb.sh` relies on — it tests x/y overlap only.

Row counts are discrete, so a target is not always exactly reachable: 1% of
`ingolstadt`'s 379 rows is 3.79 rows. The search accepts a relative tolerance
of ±10%, runs to an iteration cap, and on non-convergence takes the nearest
achievable window and appends `approx` to the row's `notes`. A missed target
is disclosed in the artefact, never silent.

Row tags stay target-named (`bbox-1pct`), and the **achieved** fraction is
what the `selectivity` column already records, so target against achieved is
checkable per row without any new column.

The grain caveat is unchanged and still applies: targets are expressed in
CityParquet row space, so feature-grained formats report a different achieved
selectivity for the identical window.

### Structure: `params.rs` and a resolved-parameters sidecar

Derivation moves out of `coordinator.rs` into a new
`benchmark/readbench/src/params.rs`, whose entry point is
`resolve(cp_table, seq_path) -> ResolvedParams`. `ResolvedParams` holds the
bbox windows (target, achieved, window, tag, `approx` flag), the id probes
(tag, id, verified presence), the `object_type` choice and its count, the
numeric attribute if one exists, and the CityObject denominator.

Its internals are split so each is testable on its own:

- `scan_bbox_and_centroids` — one pass, replacing `scan_dataset_bbox`
- `window_for_target(centroids, bbox, target)` — a pure function over an
  in-memory slice, needing no file
- `decile_ids(seq_path, fractions)` — one stream over the seq artefact
- `miss_id(seed, existing)`
- `most_frequent_object_type` and `pick_numeric_attribute`, moved unchanged

`coordinator.rs` keeps orchestration alone: `run`, `spawn_child`,
`run_measurement`, `median`/`mad`, `Row` and its rendering, and the
`AttrFilter` self-consistency check. It loses `bbox_window` and
`BBOX_FRACTIONS` entirely.

The coordinator then writes `<out>.params.json` beside the results CSV, and
`readbench_duckdb.sh` gains a `--params` argument and reads its windows,
attribute choice and id probes from it with `jq` — already a dependency of
`just scripts-test`. The script's own bash `bbox_window` and `object_type`
tie-break are deleted, which turns the parity claim in its header from a
promise into a structural fact.

This imposes an ordering constraint — the script must run after the
coordinator for a given dataset — which already holds, because the coordinator
truncates the CSV the script appends to. **A missing or unreadable params file
is a hard failure**, never a fall back to bash-side derivation; a silent
fallback would reintroduce precisely the drift the sidecar removes.

Two divergences are fixed while the sidecar is wired in. The script's
`project` scenario hardcodes `count(object_type)` where the coordinator uses
the numeric attribute column, contradicting its own stated parity; it reads
`numeric_attr` from the sidecar instead and skips when there is none.
DuckDB's missing `id-lookup` scenario is left as a follow-up, noted here so
its absence is a decision rather than an oversight.

## Testing

Written before the implementation, in the repository's usual TDD order.

`params.rs` unit tests, the ones that would have caught the original defect:

- `window_for_target` over a uniform centroid grid, where each target is
  exactly achievable
- over a bimodal cluster, where the median centroid falls between the clusters
- over a long, thin aspect ratio, asserting the window is not degenerate
- over a cloud too small for the target, asserting the `approx` flag rather
  than a silent miss
- no window returns zero rows on any prepared fixture
- `decile_ids` yields monotonically increasing positions whose ids are all
  present
- `miss_id` returns a string absent from a supplied set

Integration:

- `tests/coordinator.rs` asserts the sidecar is written and round-trips
- a `scripts/tests/` case asserts `readbench_duckdb.sh` fails loudly with no
  params file, and that its windows equal the sidecar's when given one
- `tests/strip_extension.rs` continues to guard the four per-dataset recipes,
  which need the new argument

## Re-runs and documentation

The read corpus is prepared locally, so `just bench` regenerates all six
datasets with no fetch; those CSVs are not committed and need no
reconciliation. The scaling family needs `just fetch-scaling-data`, a prepare
pass, and a re-run of all four cardinalities, replacing the committed
`scaling_read_results/`.

`benchmark/formats/READ_BENCHMARK.md` carries the documentation work. The
numbered caveats covering the lower-left window and id-lookup position are
rewritten, and four are added:

1. the CityGML artefact's member order is derived independently through
   `citygml-tools` and need not match the seq stream the deciles are cut from,
   so a probe's position within the CityGML is only approximately its decile —
   presence is verified, position is not
2. FlatCityBuf's `id` is structurally unindexable, so its `id-lookup` is a
   full walk regardless of `fcb ser -A`
3. a bbox row's target and its achieved selectivity can differ, and `approx`
   marks the rows where they do
4. targets are in CityParquet row space; feature-grained formats report a
   different achieved fraction for the same window

The summary page quotes those caveats verbatim, so they are edited at source.
`benchmark/README.md`'s load-bearing-caveats list needs the same treatment,
and `benchmark/formats/archive/2026-08-17-catalogue-corpus/` gains one line
recording that it predates this construction.
