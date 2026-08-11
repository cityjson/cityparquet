# catalog2cityparquet

Convert the published **City3D STAC catalogue** of 3D city models into a mirror of
**CityParquet** packages, and measure — item by item — how much of it converts at all.

The catalogue lives at `https://storage.googleapis.com/city3d-stac`. It is a STAC
catalogue of roughly **74,000 items across 53 collections**: national and municipal 3D
city models published as CityJSON, CityJSONSeq or CityGML, by several dozen
organisations that agree on very little. This tool walks it, downloads each item's
source bytes, hands them to the `cityparquet` converter, and assembles the results back
into a STAC catalogue of its own — one that points at CityParquet packages instead of
the original archives.

Two things come out of a run:

1. **The mirror** — a directory tree of CityParquet packages with a STAC catalogue over
   the top of it.
2. **The measurement** — a per-item ledger of what converted, what did not, and *why
   not*, drawn from a closed vocabulary so that two runs are comparable. Roughly half
   the catalogue's collections are expected not to convert; a measured statement of
   which and why is the artefact this tool exists to produce.

Failure is therefore data, not an abort. An item that fails is recorded and the next one
starts. A collection that fails is recorded and the next one starts. **The process exits
non-zero only when nothing at all could be attempted** (see [Exit codes](#exit-codes)).

Nothing in this tool interprets CityJSON, CityGML or Parquet. The Rust `cityparquet`
binary owns every format decision and `city3dstac` owns every aggregation decision; this
is orchestration — which bytes to fetch, in what order, and how to record what happened.

## Quick start

From the **repository root** (not from this directory — the default paths are relative
to the root):

```bash
just catalog-tools                                    # build the two binaries
just catalog-convert-collection rotterdam-3d out/e2e  # one small collection
just catalog-convert                                  # the whole catalogue (hours)
just catalog-test                                     # the driver's own tests
```

You need [`uv`](https://docs.astral.sh/uv/) on `PATH`, a Rust toolchain, and the
`vendor/city3d-stac-tool` submodule checked out (`git submodule update --init`). The
`catalog-*` recipes are all network-dependent and are deliberately kept out of
`just check`; `just catalog-test` is the exception — it fakes every origin and every
subprocess, and touches no network.

Running the driver directly, if you would rather not go through `just`:

```bash
uv run --project tools/catalog2cityparquet python -m catalog2cityparquet \
    --out out/cityparquet-catalog --jobs 8
```

### What a run produces

```
OUT/
  catalog.json                            the mirror's root, linking every collection
  <collection>/
    collection.json                       aggregated from the packages below
    items.parquet                         stac-geoparquet index (optional; see below)
    items/
      <item-id>/                          one CityParquet package per catalogue item
        building.parquet                  … and any other per-family table
        textures.parquet                  … and any appearance/template sidecars
        metadata.json                     the package's STAC Item
  _reports/
    <collection>.jsonl                    append-only, one line per item outcome
    summary.csv                           per-collection roll-up of THIS run
  _configs/*.yaml                         generated city3dstac configs (an artefact)
  _work/                                  per-item scratch, swept at the start of a run
```

## The pipeline

Five stages. Each is a module, each is separately tested, and each can fail without
taking the run with it.

### 1. Discover — `discover.py`

Read `catalog.json` at the catalogue root, take every `rel=child` link, and reduce each
href to a collection id. Then, for each collection, enumerate its items — see
[the reconciliation policy](#item-enumeration-the-reconciliation-policy) below, which is
the one part of discovery with a real decision in it.

A child link that cannot be turned into a usable collection id is skipped with a note on
stderr rather than raised: one malformed link must not cost the other 52 collections.

### 2. Fetch — `fetch.py`

Stream the item's `data` asset to a working directory. This stage is defensive
throughout, because the catalogue's hosts are not:

- **Media types are wrong.** One collection advertises `application/gml+xml` and serves
  a 468 MB ZIP. Format is decided by **magic bytes** and nothing else.
- **Filenames hide in query strings.** Estonia publishes hrefs such as `dl.ashx?f=x.gml`.
  The saved name matters, because the next stage decides convertibility from the suffix —
  a payload saved as `dl.ashx` would be discarded as unconvertible.
- **One origin 403s** without a browser `User-Agent`, so one is sent.
- **PLATEAU responses carry no `Content-Length`**, so no size can be checked up front.

### 3. Normalise — `fetch.normalise`

Unpack whatever arrived and return the files the converter can read (`.json`, `.jsonl`,
`.gml`, `.xml`). Archives nest — one item is a ZIP inside a ZIP — so unpacking recurses,
to a depth of 3 by default.

Three guards matter here, all of them because the payloads are third-party and some are
enormous:

- **A 20 GiB budget across the whole payload**, charged as bytes are *written*, not as
  headers declare them. A per-archive cap would multiply with nesting.
- **No member may escape** the working directory, and a repeated member name is refused
  rather than silently overwritten.
- **ZIP-shaped documents stay shut.** `.xlsx`, `.docx`, `.kmz` and friends are ZIPs
  underneath, and they ship beside the GML in PLATEAU and several German packages.
  Opening one would hand the converter its OOXML parts.

An empty result means "nothing convertible in here" — a fact about the payload, recorded
as `unsupported_archive`. Anything hostile or lossy raises instead, because quietly
skipping it would report a partial conversion as a complete one.

### 4. Convert — `convert.py`

Run `cityparquet convert` over every normalised input at once (the converter merges
them, which is what a multi-tile archive needs), then **stamp** the STAC Item the
converter wrote with the two things a single package cannot know: which collection it
belongs to, and where its bytes came from (`rel=via`, `rel=derived_from`).

Footer-derived properties are never edited. The CityParquet specification makes the
Parquet footer authoritative wherever Item and footer disagree, and the Item is built
from that footer by construction — so a package whose CRS cannot be reprojected to WGS84
carries no `geometry` and no `bbox`, and none is invented here.

A non-zero exit is classified against the ledger's vocabulary by reading the converter's
own stderr. Classification is the point of the run: "it failed" measures nothing.

### 5. Aggregate — `aggregate.py`

Run the vendored `city3dstac` over the packages on disk to write `collection.json` and,
optionally, a stac-geoparquet `items.parquet`; then link every collection into the
mirror's root `catalog.json`.

Aggregation runs over the output **directory**, not over the current run's successes.
That is what makes resumption correct: a resumed run rebuilds each collection from
everything ever converted into it, not just from what it converted today.

Two deliberate degradations:

- **Extent and summaries are recomputed** from the generated items rather than copied
  from the published collection, so they describe the mirror rather than the source.
- **A GeoParquet encode failure is retried without the sidecar.** The stac-geoparquet
  encoder refuses a null geometry, so one unlocated Item is enough to fail the index for
  a whole collection. Losing the optional index beats losing the collection — but the
  run counts how many collections ended up without one, and says so at the end.

## Item enumeration: the reconciliation policy

A collection may publish its own stac-geoparquet index at `<collection>/items.parquet`.
Reading it is enormously cheaper than the alternative: DuckDB range-reads it over HTTPS
in one request, where the object-store listing costs a page request per 1,000 objects
plus one GET per item document — about 60,000 requests for the largest collection.

**The fast path is used only when its row count agrees with the object listing.**

The reason is `japan-plateau-3d`. It publishes an `items.parquet` describing **306** of
its **60,471** items. Preferring the published index unconditionally would convert one
item in two hundred and report complete success — the exact failure this tool exists to
prevent. So both sources are consulted, their counts compared, and on any disagreement
the listing wins and the discrepancy is recorded as `stale_item_index` against the
collection. A stale index can slow a run down; it can never silently truncate one.

Order of preference, in full:

1. `items.parquet`, **iff** its row count equals the object listing's count of `.json`
   items.
2. Otherwise the object listing, with a `stale_item_index` note if an index existed and
   disagreed.
3. If nothing is listable, an index that could not be cross-checked (better than
   nothing).
4. Failing that, the `rel=item` links in `collection.json`.

## What the run records

### Conformance versus environment — read this first

The ledger makes one distinction above every other:

> **"This dataset could not be converted" is not "this machine could not complete the
> run."**

A **conformance** failure is a statement about the data: a CityGML version the reader
does not support, a source that declares no CRS, an archive with nothing convertible in
it, an origin that would not serve the bytes. These make the **reasons histogram**, and
the histogram is the number this project publishes.

An **environment** failure is a statement about the host: a full disk, an unwritable
directory, a `city3dstac` that was never built, a broken stdout. It says *nothing* about
whether the dataset converts. It has its own status, its own column in `summary.csv`, and
it is excluded from the histogram **by construction** — the ledger will not even let the
status and the reason drift apart.

Why it matters this much: without the distinction, every local failure has to be recorded
as a conversion failure, and that is how a run whose log volume filled comes to publish
half a catalogue as unconvertible, with the real packages sitting on disk beside the
claim. A run that hit environment failures is an **incomplete** run, and the summary says
so in as many words:

```
!! 3 environment failure(s): this machine, not the data.
   They are excluded from the reasons above and say nothing about
   whether these collections convert; this run is incomplete.
```

If you see that block, the run's coverage numbers are not publishable. Fix the machine
and re-run; resumption means you only pay for what is missing.

### The reason vocabulary

Closed set. A reason outside it is a programming error, not a new category — silently
admitting typos would make the histogram meaningless.

| Reason | Kind | What it means |
|---|---|---|
| `download_failed` | conformance | The origin would not serve the bytes: a transport error, an HTTP error status, or a timeout. The publisher's availability, not the converter's competence. |
| `unsupported_archive` | conformance | The payload unpacked cleanly and held nothing the converter reads. |
| `unsupported_citygml_version` | conformance | CityGML the reader does not support (it implements 2.0). |
| `unsupported_cityjson_version` | conformance | CityJSON the reader rejects as invalid or out of version range. |
| `no_crs` | conformance | The source carries CRS-bearing coordinates but declares no CRS a writer can resolve. Fixable per collection with `--crs`. |
| `geographic_crs` | conformance | The source declares a geographic (degrees) CRS, which CityParquet does not accept for 3D city geometry. |
| `convert_failed` | conformance | The converter refused for a reason the classifier does not recognise, or it timed out. The catch-all: a large count here means the classifier needs another rule, not that the data is uniquely broken. |
| `empty_collection` | conformance | The collection publishes no items at all — 20 of the 53 hold only a `collection.json`. |
| `duplicate_bundle` | conformance | Skipped before downloading: Japan's 381 whole-city ZIPs repackage the same data as the 60,090 per-module tiles, and converting both would encode Japan twice. |
| `stale_item_index` | conformance | The collection's published `items.parquet` disagreed with the object listing; the listing was used. A fact about the catalogue, recorded rather than merely logged. |
| `environment` | **environment** | This machine failed here. Excluded from the histogram; see above. |

### Where the records live

- **`_reports/<collection>.jsonl`** — append-only, one JSON object per outcome, with the
  item id, status, reason, the error text (truncated to 2,000 characters), bytes
  downloaded and seconds spent. **This is the durable record**, and it accumulates
  across runs.
- **`_reports/summary.csv`** — a per-collection roll-up of *this run only*, rewritten
  wholesale at the end of each run. See the caveat under [Resuming](#resuming).
- **stdout** — the summary table, the reasons histogram, the environment block if any,
  and the count of collections that ended without a GeoParquet index. The stdout report
  is the primary artefact; the CSV is a convenience, and it is written best-effort
  precisely so that a lost roll-up cannot turn a completed measurement into a traceback.

## Resuming

A run is resumable, which matters because a full run takes hours and a large collection
takes most of them.

**An item is considered done when its package holds a parseable STAC Item**
(`metadata.json` with `"type": "Feature"` and a `stac_version`). That test is safe
because the Rust writer commits atomically and renames `metadata.json` last, so a
half-finished conversion never leaves a parseable Item behind. Anything unparseable — or
absent beside a directory full of Parquet — means "not finished", and the item is
re-attempted.

A skipped item produces **no ledger record at all**. It is not an outcome of *this* run,
and counting it would make a resumed run look like a fresh success.

> **Caveat — `summary.csv` is per-run, not cumulative.** It is written from what the
> current process observed, so a fully resumed run (every item already done) rewrites it
> with nothing but a header, while `_reports/*.jsonl` keeps every line ever recorded.
> **After a multi-session run, the coverage evidence is the JSONL files, not the CSV.**
> Roll them up yourself, or force a single measuring pass with `--no-skip-existing`
> (which re-converts everything, at full cost).

Working directories are swept at the *start* of a run rather than at the end, because the
run that made the mess is by definition not around to clean it up. `--keep-downloads`
suppresses the sweep as well as the per-item cleanup: an operator who asked to keep them
means across runs too.

## Locking

A run claims **both** the output directory and the working directory with a `.c2cp-lock`
file, and refuses to start if another live run holds either:

```
cannot start: another run holds the output directory out/e2e (pid 1 on host.example).
Wait for it to finish, use a different --out, or delete out/e2e/.c2cp-lock if no other
run is active.
```

Both are locked, not just one. Two runs sharing a working directory would sweep each
other's live downloads; two runs sharing an output directory would write two ledger lines
per collection while `summary.csv` — rewritten by whichever finishes last — reports one.

**A killed run leaves its `.c2cp-lock` behind.** The lock records the owning pid and
hostname, so a stale lock left by a dead process *on the same host* is reclaimed
automatically. Two cases are not reclaimed, deliberately: a lock written by another host
(over a shared filesystem, where the pid means nothing), and a lock whose contents cannot
be read. Refusing to start is recoverable — the message names the file to delete — whereas
sweeping a live run's directory destroys a download in progress, which reappears in the
ledger as `download_failed`, indistinguishable from an origin having a bad day. To clear
one by hand once you are sure no run is active:

```bash
rm -f OUT/.c2cp-lock OUT/_work/.c2cp-lock   # or WORKDIR/.c2cp-lock if you used --work-dir
```

## Exit codes

| Code | Meaning |
|---|---|
| `0` | The run measured something. **However many items failed**, and however badly the report itself fared. |
| `1` | Nothing could be attempted: the output directory could not be prepared, another run holds a directory this one needs, or the catalogue root was unreachable. |

There is no exit code for "some items failed". That is the normal, expected outcome and
it is what the ledger is for.

## Flags

| Flag | Default | Meaning |
|---|---|---|
| `--out` | `out/cityparquet-catalog` | Output tree root |
| `--collection` | all | Repeatable; restrict to these collection ids |
| `--limit-per-collection` | none | Convert at most N items per collection |
| `--jobs` | 8 | Concurrent items; bounded out of politeness to origins |
| `--crs` | none | Repeatable `COLLECTION=EPSG:xxxx` override for CRS-less sources |
| `--keep-downloads` | off | Retain temp downloads for debugging |
| `--no-skip-existing` | off | Reconvert items that already have a package |
| `--aggregate-only` | off | Rebuild STAC from an existing tree, no downloads |
| `--work-dir` | `<out>/_work` | Where downloads are unpacked; needs room for the largest single payload, and must not be shared with a concurrent run |

Plus three that exist so the tool can be pointed somewhere else — at a test double, a
staging bucket, or a locally built binary:

| Flag | Default |
|---|---|
| `--binary` | `target/release/cityparquet` |
| `--tool` | `vendor/city3d-stac-tool/target/release/city3dstac` |
| `--base-url` | `https://storage.googleapis.com/city3d-stac` |
| `--bucket-api` | `https://storage.googleapis.com/storage/v1/b/city3d-stac/o` |

Notes on the ones whose names promise more than they deliver:

- **`--aggregate-only` rebuilds only the mirror's root `catalog.json`**, from the
  `collection.json` files already on disk. It does not re-aggregate the collections
  themselves. To rebuild one collection's `collection.json`/`items.parquet` offline there
  is currently no flag: run `catalog-convert --collection <id>`, which skips the
  already-converted items but does re-enumerate the collection over the network.
- **`--jobs` is a politeness bound, not a capacity one.** The catalogue is served from a
  handful of origins, some of them small national portals. Throttling one of them fails
  items that would otherwise convert, which corrupts the very measurement the run exists
  to produce.
- **`--limit-per-collection 0` is an error, not "no limit".** Counts must be 1 or more,
  so a typo cannot quietly produce a plausible-looking run that measured the wrong thing.

Timeouts are not exposed on the command line: 30 minutes per download, 60 minutes per
conversion, 60 minutes per `city3dstac` call, 2 minutes per metadata request.

## A worked full run

```bash
# One terminal, from the repository root. Expect hours.
just catalog-convert out/cityparquet-catalog --jobs 8 2>&1 | tee run.log
```

What to expect:

- **53 collections attempted.** Around 20 of them publish no items and are recorded as
  `empty_collection` in seconds.
- **`japan-plateau-3d` dominates.** 60,471 items, of which 381 are skipped as
  `duplicate_bundle` and 60,090 tiles are converted; its published index is stale, so
  enumeration alone costs roughly 60,000 requests before the first byte is converted.
- **`netherlands-3d-bag` is the second-largest** at 8,941 items, all gzipped CityJSON.
- Everything else is small. As a single calibration point, measured against the live
  catalogue: `rotterdam-3d`'s one item is a 2.7 MB download converting 853 city objects
  in **0.98 s** end to end.

A full run had **not** been made at the time of writing, so no wall-clock figure is
quoted here — see [Known limitations](#known-limitations), which explains why it should
not be attempted yet.

When it does finish, check the outcome against the design's probe: **15 collections
converting, ~69,100 items**. A materially different number means either the catalogue has
changed or the driver has a bug — investigate before quoting the figures.

Resume after an interruption by re-running the identical command. To re-measure without
re-downloading, there is no shortcut: the ledger records outcomes, not a cache.

## Known limitations

Four, all of them affecting how the measurement should be read. The first two are queued
for the whole-branch review; the last two were found by the first end-to-end runs against
the live catalogue.

1. **A converter host failure is misclassified as conformance.** If the `cityparquet`
   subprocess fails because of *its* environment — its disk filling as it writes a
   package, say — the driver sees only a non-zero exit and an unrecognised stderr, and
   records `convert_failed`: a conformance reason. A volume that filled mid-run would
   therefore stamp every remaining item as unconvertible, with no warning anywhere in the
   summary. (The aggregation subprocess already gets this right — `aggregate.HostFailure`
   reads `city3dstac`'s stderr for kernel wording — and the converter needs the same
   treatment.)

2. **A corrupt `.json.gz` is recorded as an environment failure.** `gzip.BadGzipFile`
   subclasses `OSError`, and the item loop treats `OSError` as "this machine". A
   genuinely truncated or corrupt gzip payload — a fact about the data — is therefore
   routed away from the histogram and counted against the host. This matters more than it
   sounds: `netherlands-3d-bag` is 8,941 items and *entirely* `.json.gz`.

3. **A collection in which every item failed is recorded twice.** After the items are
   attempted, aggregation runs unconditionally; with no package on disk, `city3dstac`
   exits with "No STAC item files provided" and the orchestrator ledgers a *second*,
   collection-level record with the catch-all reason `convert_failed`. Observed on
   `luxembourg-3d`, whose single item fails `no_crs`: `summary.csv` reports 2 failures for
   a 1-item collection, and the histogram gains a `convert_failed` that describes the
   driver rather than the data. Aggregation should be skipped when nothing converted.

4. **`summary.csv` is per-run, not cumulative** — see [Resuming](#resuming). Harmless
   once known; badly misleading if not.

> **Do not run the full ~74,000-item measurement until (1) and (2) are fixed.** Between
> them they can move a large number of items across the one line that the published
> result depends on — (1) can silently convert a local disk problem into thousands of
> conformance failures, and (2) can hide a real data failure in the largest gzipped
> collection in the catalogue. A run made now would have to be thrown away and repeated.

## Tests

```bash
just catalog-test          # 219 tests, no network, no binaries
```

Every origin, subprocess and catalogue document is faked, so the suite is safe to run
anywhere and is fast. It is not part of `just check` (which is the Rust workspace's
gate) — run both.
