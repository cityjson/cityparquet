# Retired: the 30-dataset city3d-STAC catalogue corpus

Everything here measured the **previous** read-benchmark corpus — 30 published
city models sampled from the city3d STAC catalogue, 6.5 GB on the wire, run on
**2026-08-17**. It was retired on **2026-08-23** and replaced by the six-dataset
cityjson.org corpus now pinned in `scripts/fetch_benchmark.sh`.

Nothing in here is read by any recipe. `just plot` / `just plot-pretty` chart
`bench/read_results/` and `bench/ordering_results/`, which this directory has
been emptied out of; the CSVs below are kept so a number already quoted
elsewhere can still be traced to the run that produced it.

## Why it was retired

The corpus optimised for **variety** — geographies, publishers, CityGML modules
— at the cost of the one property the paper actually needs: **every dataset
carrying every compared format**. It did not, for a structural reason.

`readbench_prepare.sh` built the `citygml` artefact only from a CityGML _input_
and never synthesised one, so the 8-format comparison was complete only for the
`.gml` entries. The `.city.json` entries — 3DBAG, Rotterdam, Vienna, NYC, Zürich
— silently produced **seven** rows, not eight, and the `citygml` column was
therefore missing from exactly the datasets a reader is most likely to recognise.
Two further entries (`riga_atgazene_lod2.gml`, `plateau_chuo_brid.gml`) could not
serve a default-set run at all and needed the `--only no-citygml` escape hatch.

The replacement inverts the trade: fewer datasets, but each one produces all
eight format rows from a single source document, so any cross-format gap is a
property of the format rather than of which artefacts happened to exist. The
`citygml` artefact is now synthesised with `citygml-tools from-cityjson -v 2.0`;
`bench/READ_BENCHMARK.md` states what that costs and what it buys.

## What is here

| path                           | what                                                                                                                                                                                     |
| ------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `read_results/*.csv`           | format comparison, 21 datasets + `sizes.csv`                                                                                                                                             |
| `ordering_results/*.csv`       | ordering comparison, 28 datasets + `sizes.csv`                                                                                                                                           |
| `catalogue_benchmark_urls.txt` | per-URL provenance: which STAC collection each came from, verification dates, and the BLOCKED / EXCLUDED sections recording what the catalogue offered but the harness could not measure |
| `corpus.manifest`              | the retired pinned table, in `CORPUS_MANIFEST` format                                                                                                                                    |

`bench/scaling_*_results/` are **not** archived and were not affected: they
measure the 3DBAG FlatCityBuf slice ladder (`just scaling-corpus`), which holds
the data constant and varies only size, and is independent of which published
datasets the format comparison uses.

## Re-running it

The old corpus is still fetchable — `corpus.manifest` is a live input, not a
transcript:

```sh
CORPUS_MANIFEST=bench/archive/2026-08-17-catalogue-corpus/corpus.manifest \
  ./scripts/fetch_benchmark.sh --only all bench/data/legacy
just bench bench/data/legacy bench/data/legacy_results
```

Two caveats. The pinned byte sizes were measured 2026-08-16 and are re-verified
on every fetch, so an origin that has re-published since will **hard-fail**
rather than quietly hand back different bytes — that failure is the check
working. And `--only all` includes the two entries that abort a default-set run,
so pair it with an explicit `--formats` list that omits `citygml`.
