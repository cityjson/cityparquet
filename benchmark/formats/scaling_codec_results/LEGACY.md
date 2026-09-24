# Legacy read results

These CSVs predate two definition changes made on 2026-09-24 and must not be
mixed with runs produced after them:

- The `bbox-Npct` rows used a window sized to select a target fraction of
  **rows**, centred on the median row centre. The current window covers a
  fixed fraction of the dataset's x/y **area**, anchored at its lower-left
  corner, the same window `benchmark/databases` uses.
- `time_s` here is a **median**. The current definition is the arithmetic
  **mean**, with the population standard deviation in `time_std_s`. This
  directory's dispersion column is the older `time_mad_s`, a median absolute
  deviation about that median.

The current `benchviz` loader requires the `time_std_s` column and refuses
these files, so they cannot be plotted alongside new results. Re-run the
family to replace them; until then they are a record only.
