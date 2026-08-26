# Examples

Small, committable inputs for trying CityParquet out. Anything large enough to
be worth measuring lives outside git and is fetched by a script — see
[`benchmark/README.md`](../benchmark/README.md).

`data/` is currently empty. The two corpora the stack is exercised against are
both fetch-driven, and deliberately so:

| Corpus | Where it comes from | How to get it |
|---|---|---|
| Reader/writer fixtures (Delft, an LoD3 railway, five CityGML 2.0 samples) | pinned public URLs | `just fixtures` in `lib/cityparquet-rs/` |
| Benchmark corpora (3DBAG tiles, scaling subsets, ~24 GB) | pinned public URLs, byte sizes checked | `just fetch-data` / `just fetch-scaling-data` from the repo root |

Nothing here is a fixture *copy*: a committed duplicate of a pinned download is
a second thing to keep in sync, and the pins already make the originals
reproducible.
