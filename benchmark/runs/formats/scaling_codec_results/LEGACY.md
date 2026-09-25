# Legacy read results

These CSVs predate the timing definition adopted on 2026-09-24 and must not be
mixed with runs produced after it:

- `time_s` here is a **median**. The current definition is the arithmetic
  **mean**, with the population standard deviation in `time_std_s`. This
  directory's dispersion column is the older `time_mad_s`, a median absolute
  deviation about that median.

The `bbox-Npct` windows are unchanged: they were, and still are, sized to
select a target fraction of **rows**, centred on the median row centre
(`benchmark/formats/READ_BENCHMARK.md`).

The current `benchviz` loader requires the `time_std_s` column: it refuses
these files as read results, and reports a configuration-axis directory that
holds them as a gap rather than plotting it, so they cannot be shown
alongside new results. Re-run the family to replace them; until then they are
a record only.
