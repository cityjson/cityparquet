"""``bench_data.json`` -> a self-contained ``bench-summary.html``.

The page is fully self-contained: the data set is embedded as a
``<script type="application/json">`` block, the styling is inline CSS and all
panels are drawn as inline SVG by one inline script.  No external request of
any kind is made, so the page works from ``file://``.

Python's job here is deliberately small — read the JSON, escape it into the
document, splice the three literal blocks (CSS, BODY, JS) into the template.
All rendering logic lives in the JS block below.
"""

from __future__ import annotations

import json
from pathlib import Path

from .paths import DEFAULT_DATA_PATH, DEFAULT_HTML_PATH

# --------------------------------------------------------------------------
# CSS
# --------------------------------------------------------------------------

CSS = r"""
:root {
  color-scheme: light dark;
  --bg: #fffff8;
  --fg: #111111;
  --muted: #666666;
  --faint: #8a8a80;
  --rule: #dddddd;
  --rule-strong: #999999;
  --accent: #e41a1c;
  --mark: #666666;
  --band: rgba(120, 120, 100, 0.16);
  --heat-pos: #1b7837;
  --heat-neg: #b2182b;
  --chip: rgba(120, 120, 100, 0.10);
}
:root[data-theme="light"] {
  --bg: #fffff8; --fg: #111111; --muted: #666666; --faint: #8a8a80;
  --rule: #dddddd; --rule-strong: #999999; --accent: #e41a1c; --mark: #666666;
  --band: rgba(120, 120, 100, 0.16); --heat-pos: #1b7837; --heat-neg: #b2182b;
  --chip: rgba(120, 120, 100, 0.10);
}
@media (prefers-color-scheme: dark) {
  :root:not([data-theme="light"]) {
    --bg: #151515; --fg: #e9e9e1; --muted: #999999; --faint: #7d7d75;
    --rule: #333333; --rule-strong: #555555; --accent: #fc8d62; --mark: #999999;
    --band: rgba(200, 200, 180, 0.12); --heat-pos: #66bd63; --heat-neg: #ef6548;
    --chip: rgba(200, 200, 180, 0.08);
  }
}
:root[data-theme="dark"] {
  --bg: #151515; --fg: #e9e9e1; --muted: #999999; --faint: #7d7d75;
  --rule: #333333; --rule-strong: #555555; --accent: #fc8d62; --mark: #999999;
  --band: rgba(200, 200, 180, 0.12); --heat-pos: #66bd63; --heat-neg: #ef6548;
  --chip: rgba(200, 200, 180, 0.08);
}

* { box-sizing: border-box; }
html { -webkit-text-size-adjust: 100%; }
body {
  margin: 0;
  padding: 0 1.25rem 6rem;
  background: var(--bg);
  color: var(--fg);
  font-family: "Palatino Linotype", Palatino, "Book Antiqua", Georgia, serif;
  font-size: 17px;
  line-height: 1.5;
}
main { max-width: 74rem; margin: 0 auto; }
h1, h2, h3 { font-weight: 400; line-height: 1.2; }
h1 { font-size: 2.05rem; margin: 2.2rem 0 0.4rem; max-width: 36ch; }
h2 { font-size: 1.45rem; margin: 3.2rem 0 0.3rem; }
h3 { font-size: 1.08rem; margin: 1.6rem 0 0.3rem; }
p { max-width: 62ch; }
p, li, figcaption, .note, .verbatim { overflow-wrap: break-word; min-width: 0; }
a { color: inherit; text-underline-offset: 2px; }
.lede { font-size: 1.12rem; color: var(--muted); max-width: 60ch; margin: 0 0 0.6rem; }
.sub { color: var(--muted); font-size: 0.9rem; }
.rule { border: 0; border-top: 1px solid var(--rule); margin: 2.4rem 0 0; }
.small { font-size: 0.86rem; color: var(--muted); }
.sans, .num { font-family: system-ui, -apple-system, "Segoe UI", sans-serif; }
.num { font-variant-numeric: tabular-nums; }

/* --- header / theme toggle ------------------------------------------- */
.topbar {
  display: flex; align-items: baseline; justify-content: space-between;
  gap: 1rem; flex-wrap: wrap; padding-top: 0.9rem;
  border-bottom: 1px solid var(--rule);
}
.topbar .kicker {
  font-family: system-ui, sans-serif; font-size: 0.74rem;
  letter-spacing: 0.14em; text-transform: uppercase; color: var(--muted);
}
button.theme {
  font-family: system-ui, sans-serif; font-size: 0.76rem;
  background: transparent; color: var(--muted);
  border: 1px solid var(--rule-strong); border-radius: 3px;
  padding: 0.25rem 0.6rem; cursor: pointer;
}
button.theme:hover, button.theme:focus-visible { color: var(--fg); }

/* --- how to read ------------------------------------------------------ */
.howto { display: grid; gap: 0.55rem 2rem; grid-template-columns: repeat(auto-fit, minmax(19rem, 1fr)); margin: 1rem 0 0; }
.howto p { margin: 0; max-width: 46ch; font-size: 0.95rem; }
.howto b { font-weight: 400; border-bottom: 1px solid var(--rule-strong); }

/* --- controls --------------------------------------------------------- */
.controls {
  display: flex; flex-wrap: wrap; gap: 0.4rem 1.6rem; align-items: center;
  font-family: system-ui, sans-serif; font-size: 0.82rem; color: var(--muted);
  margin: 0.9rem 0 0.2rem; padding: 0.5rem 0;
  border-top: 1px solid var(--rule); border-bottom: 1px solid var(--rule);
}
.controls select { font: inherit; color: var(--fg); background: var(--bg);
  border: 1px solid var(--rule-strong); border-radius: 3px; padding: 0.15rem 0.3rem; }
.controls fieldset { border: 0; margin: 0; padding: 0; display: flex; gap: 0.8rem; align-items: center; }
.controls legend { float: left; padding: 0 0.5rem 0 0; }
.controls label { display: inline-flex; gap: 0.3rem; align-items: center; }

/* --- panel grids ------------------------------------------------------ */
.grid { display: grid; gap: 1.1rem 1.2rem; margin: 1.1rem 0 0;
  grid-template-columns: repeat(auto-fill, minmax(260px, 1fr)); }
.grid.wide { grid-template-columns: repeat(auto-fill, minmax(300px, 1fr)); }
figure.panel { margin: 0; min-width: 0; }
figure.panel figcaption { line-height: 1.25; margin-bottom: 0.15rem; }
figure.panel figcaption .name { font-size: 1rem; }
figure.panel figcaption .sub { display: block; font-size: 0.76rem; }
figure.panel svg { width: 100%; height: auto; display: block; overflow: hidden; }
figure.panel .note { margin: 0.15rem 0 0; font-size: 0.74rem; color: var(--muted);
  font-family: system-ui, sans-serif; }
figure.panel.dim { opacity: 0.55; }
.badge { display: inline-block; font-family: system-ui, sans-serif; font-size: 0.7rem;
  border: 1px solid var(--accent); color: var(--accent); border-radius: 3px;
  padding: 0 0.3rem; vertical-align: 1px; }

/* --- svg chart primitives -------------------------------------------- */
svg text { fill: var(--fg); }
svg .tick { font-family: system-ui, sans-serif; font-size: 8px; fill: var(--faint); }
svg .axis { stroke: var(--rule-strong); stroke-width: 0.6; fill: none; }
svg .refline { stroke: var(--rule-strong); stroke-width: 0.5; stroke-dasharray: 3 3; fill: none; }
svg .frontier { stroke: var(--mark); stroke-width: 0.7; stroke-dasharray: 2 2; fill: none; opacity: 0.7; }
svg .band { fill: var(--band); }
svg .axlabel { font-family: system-ui, sans-serif; font-size: 8px; fill: var(--muted); }
svg .dlabel { font-size: 9px; fill: var(--fg); }
svg .mk-gray { fill: var(--mark); stroke: var(--mark); }
svg .mk-accent { fill: var(--accent); stroke: var(--accent); }
svg .mk-open { fill: none; stroke: var(--accent); stroke-width: 1.1; }
svg .mk-gray-open { fill: none; stroke: var(--mark); stroke-width: 1.1; }
svg .leader { stroke: var(--rule-strong); stroke-width: 0.4; fill: none; }
svg .mk-line { fill: none; stroke-width: 1.1; }
svg .bar { fill: var(--mark); }
svg .bar.muted { opacity: .3; }
svg .line { fill: none; stroke: var(--mark); stroke-width: 1.2; }
svg .line.accent { stroke: var(--accent); stroke-width: 1.6; }
svg .endlab { font-size: 8px; font-weight: 500; fill: var(--muted); }
svg .endlab.accent { fill: var(--accent); }
svg .floorband { fill: var(--band); }
table.corpus { border-collapse: collapse; font-size: .82rem; width: 100%; }
table.corpus caption { text-align: left; color: var(--muted); font-size: .78rem;
  padding: 0 0 .45rem; }
table.corpus th, table.corpus td { padding: .22rem .5rem; text-align: right;
  border-bottom: 1px solid var(--rule); white-space: nowrap; }
table.corpus th[scope="row"], table.corpus thead th:first-child { text-align: left; }
table.corpus tbody th { font-weight: 400; }
svg .bar.accent { fill: var(--accent); }
svg .bar.accent-open { fill: none; stroke: var(--accent); stroke-width: 1; }
svg .pt { cursor: crosshair; }
svg .pt:focus { outline: none; }
svg .pt:focus .halo, svg .pt:hover .halo { stroke: var(--fg); stroke-width: 0.8; fill: none; opacity: 0.9; }

/* --- view 0: overview -------------------------------------------------- */
.rationale { font-style: italic; color: var(--muted); font-size: 0.92rem; max-width: 60ch; }
figure.solo { margin: 1.1rem 0 0; min-width: 0; max-width: 52rem; }
figure.solo svg { width: 100%; height: auto; display: block; overflow: visible; }
figure.solo .note { margin: 0.35rem 0 0; font-size: 0.78rem; color: var(--muted);
  font-family: system-ui, sans-serif; max-width: 62ch; }
.grid.strips { grid-template-columns: repeat(auto-fill, minmax(330px, 1fr)); }
svg .dtick { stroke: var(--faint); stroke-width: 0.7; fill: none; opacity: 0.42; }
svg .dtick.acc { stroke: var(--accent); opacity: 0.3; }
svg .dot { fill: var(--faint); opacity: 0.6; stroke: none; }
svg .dot-open { fill: none; stroke: var(--faint); stroke-width: 0.7; opacity: 0.5; }
svg .slope { fill: none; stroke-width: 1.2; stroke: var(--mark); }
svg .slope.accent { stroke: var(--accent); }
svg .slope.dash { stroke-dasharray: 4 2.5; }
svg .axtitle { font-family: system-ui, sans-serif; font-size: 9.5px; fill: var(--fg); }
svg .warn { fill: var(--accent); }

/* --- heatmap ---------------------------------------------------------- */
table.heat { border-collapse: collapse; width: 100%; font-family: system-ui, sans-serif;
  font-size: 0.72rem; font-variant-numeric: tabular-nums; }
table.heat th { font-weight: 400; color: var(--muted); text-align: right;
  padding: 0.1rem 0.25rem; border-bottom: 1px solid var(--rule-strong); }
table.heat th.scen { text-align: left; }
table.heat td { padding: 0.15rem 0.25rem; text-align: right; border: 0; color: var(--fg); }
table.heat td.scen { text-align: left; color: var(--muted); font-size: 0.7rem;
  white-space: nowrap; padding-right: 0.4rem; }
table.heat td.na { color: var(--faint); }
table.heat td.floor { color: var(--muted); font-style: italic; }
.dag { text-decoration: none; color: var(--muted); }
.dag:hover, .dag:focus-visible { color: var(--accent); }

/* --- data tables ------------------------------------------------------ */
details { margin: 0.8rem 0 0; }
details summary { cursor: pointer; font-family: system-ui, sans-serif;
  font-size: 0.8rem; color: var(--muted); }
.scroll { overflow-x: auto; max-width: 100%; }
table.data { border-collapse: collapse; font-size: 0.8rem; margin-top: 0.4rem;
  font-family: system-ui, sans-serif; }
table.data th { font-weight: 400; color: var(--muted); font-size: 0.7rem;
  letter-spacing: 0.04em; text-transform: uppercase; text-align: left;
  padding: 0.25rem 0.7rem; border-bottom: 1px solid var(--rule-strong); white-space: nowrap; }
table.data td { padding: 0.18rem 0.7rem; border: 0; white-space: nowrap; }
table.data td.n { text-align: right; font-variant-numeric: tabular-nums; }
table.data tr.sep td { border-top: 1px solid var(--rule); }

/* --- caveats ---------------------------------------------------------- */
ol.caveats { padding-left: 1.6rem; max-width: 74ch; }
ol.caveats li { margin: 0 0 1.1rem; }
.verbatim { white-space: pre-wrap; font-size: 0.86rem; line-height: 1.42;
  font-family: system-ui, -apple-system, "Segoe UI", sans-serif;
  overflow-wrap: anywhere; }
.callout { border-left: 2px solid var(--rule-strong); padding: 0.1rem 0 0.1rem 0.9rem;
  margin: 0.9rem 0; max-width: 66ch; }
.callout.warn { border-left-color: var(--accent); }
ul.notes { max-width: 66ch; padding-left: 1.2rem; }
ul.notes li { margin-bottom: 0.35rem; font-size: 0.92rem; }

/* --- de-emphasized section ------------------------------------------- */
section.deemph { opacity: 0.88; }
section.deemph h2 { font-size: 1.12rem; color: var(--muted); }
section.deemph .grid { grid-template-columns: repeat(auto-fill, minmax(210px, 1fr)); }
/* the accent belongs to the headline views only — view 4 is entirely neutral */
section.deemph .badge { border-color: var(--fg); color: var(--fg); }
section.deemph .callout.warn { border-left-color: var(--fg); }

/* --- tooltip ---------------------------------------------------------- */
#tip {
  position: fixed; z-index: 20; pointer-events: none; max-width: 22rem;
  background: var(--bg); color: var(--fg);
  border: 1px solid var(--rule-strong); border-radius: 3px;
  padding: 0.35rem 0.5rem; font-family: system-ui, sans-serif; font-size: 0.75rem;
  line-height: 1.35; white-space: pre-line; box-shadow: 0 1px 4px rgba(0,0,0,0.18);
}
#tip[hidden] { display: none; }

.sr-only {
  position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px;
  overflow: hidden; clip: rect(0 0 0 0); white-space: nowrap; border: 0;
}
:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
@media (prefers-reduced-motion: reduce) {
  * { animation: none !important; transition: none !important; scroll-behavior: auto !important; }
}
@media (max-width: 420px) {
  body { padding: 0 0.8rem 4rem; font-size: 16px; }
  h1 { font-size: 1.6rem; }
  .grid { grid-template-columns: 1fr; }
}
"""

# --------------------------------------------------------------------------
# Body skeleton — every data-bearing node is filled in by the script.
# --------------------------------------------------------------------------

BODY = r"""
<div id="tip" hidden role="status" aria-live="polite"></div>
<main>
  <header class="topbar">
    <span class="kicker">CityParquet benchmarks &middot; read, size, compression</span>
    <button class="theme" id="theme-btn" type="button" aria-live="polite">Theme: auto</button>
  </header>

  <h1 id="headline"></h1>
  <p class="lede" id="lede"></p>
  <p class="small" id="provenance"></p>

  <h2>How to read this page</h2>
  <div class="howto" id="howto"></div>

  <hr class="rule">

  <section id="view-corpus" aria-labelledby="h-corpus">
    <h2 id="h-corpus">1 &middot; The corpus &mdash; what was measured, and how big it is</h2>
    <p class="small" id="corpus-lede"></p>
    <div id="corpus-table"></div>
  </section>

  <hr class="rule">

  <section id="view-formats" aria-labelledby="h-formats">
    <h2 id="h-formats">2 &middot; Read time and peak memory, per dataset</h2>
    <p class="small" id="formats-lede"></p>
    <div class="controls" id="formats-controls" role="group" aria-label="Per-dataset format controls">
      <label for="formats-scen">Scenario
        <select id="formats-scen"></select>
      </label>
      <span id="formats-scen-note"></span>
    </div>
    <div class="grid" id="formats-grid"></div>
  </section>

  <hr class="rule">

  <section id="view-config" aria-labelledby="h-config">
    <h2 id="h-config">3 &middot; Configuration axes &mdash; ordering, codec, row-group size</h2>
    <p class="small" id="config-lede"></p>

    <h3 id="h-config-order">3a &middot; Row ordering, across the whole ordering run</h3>
    <p class="small" id="config-order-lede"></p>
    <div class="controls" id="config-order-controls" role="group" aria-label="Ordering controls">
      <label for="config-order-scen">Scenario
        <select id="config-order-scen"></select>
      </label>
    </div>
    <figure class="solo" id="config-order-fig"></figure>

    <h3 id="h-config-var">3b &middot; Codec and row-group size, on the scaling corpus</h3>
    <p class="small" id="config-var-lede"></p>
    <div id="config-var-table"></div>
  </section>

  <hr class="rule">

  <section id="view-scaling" aria-labelledby="h-scaling">
    <h2 id="h-scaling">4 &middot; How each format scales with CityObject count</h2>
    <p class="small" id="scaling-lede"></p>
    <div class="controls" id="scaling-controls" role="group" aria-label="Scaling controls">
      <label for="scaling-scen">Scenario
        <select id="scaling-scen"></select>
      </label>
      <span id="scaling-scen-note"></span>
    </div>
    <div class="grid" id="scaling-grid"></div>
  </section>

  <hr class="rule">

  <section id="view-overview" aria-labelledby="h-overview">
    <h2 id="h-overview">5 &middot; Format profiles across the whole corpus</h2>
    <p class="rationale" id="overview-rationale"></p>

    <h3 id="h-slope">5a &middot; The trade-off, at a glance</h3>
    <p class="small" id="slope-lede"></p>
    <div class="controls" id="slope-controls" role="group" aria-label="Trade-off slopegraph controls">
      <label for="slope-scen">Scenario
        <select id="slope-scen"></select>
      </label>
      <span id="slope-scen-note"></span>
    </div>
    <figure class="solo" id="slope-fig"></figure>

    <h3 id="h-strips">5b &middot; Is the win consistent across datasets?</h3>
    <p class="small" id="strip-lede"></p>
    <div class="controls" id="strip-controls" role="group" aria-label="Dot-strip controls">
      <fieldset>
        <legend>Metric</legend>
        <label><input type="radio" name="stripmetric" value="time" checked> read time</label>
        <label><input type="radio" name="stripmetric" value="rss"> peak RSS</label>
        <label><input type="radio" name="stripmetric" value="heap"> peak heap</label>
      </fieldset>
      <span id="strip-metric-note"></span>
    </div>
    <div class="grid strips" id="strip-grid"></div>
  </section>

  <hr class="rule">

  <section id="view-pareto" aria-labelledby="h-pareto">
    <h2 id="h-pareto">6 &middot; Speed against memory, per dataset</h2>
    <p class="small" id="pareto-lede"></p>
    <div class="controls" id="pareto-controls" role="group" aria-label="Pareto grid controls">
      <label for="scen-select">Scenario
        <select id="scen-select"></select>
      </label>
      <fieldset>
        <legend>Memory</legend>
        <label><input type="radio" name="mem" value="rss" checked> peak RSS</label>
        <label><input type="radio" name="mem" value="heap"> peak heap</label>
      </fieldset>
      <span id="pareto-scen-note"></span>
    </div>
    <p class="small" id="pareto-metric-note"></p>
    <div class="grid" id="pareto-grid"></div>
  </section>

  <hr class="rule">

  <section id="view-heat" aria-labelledby="h-heat">
    <h2 id="h-heat">7 &middot; Read speedup against CityJSONSeq, every scenario</h2>
    <p class="small" id="heat-lede"></p>
    <div class="grid wide" id="heat-grid"></div>
    <div id="heat-details"></div>
  </section>

  <hr class="rule">

  <section id="view-size" aria-labelledby="h-size">
    <h2 id="h-size">8 &middot; Bytes on disk, as a fraction of CityJSONSeq</h2>
    <p class="small" id="size-lede"></p>
    <div class="grid" id="size-grid"></div>
  </section>

  <hr class="rule">

  <section id="view-comp" class="deemph" aria-labelledby="h-comp">
    <h2 id="h-comp">9 &middot; Compression and row-group variants (not citable as a codec ranking)</h2>
    <div class="callout warn" id="codec-note"></div>
    <p class="small" id="comp-lede"></p>
    <div class="grid" id="comp-grid"></div>
    <div id="comp-notes"></div>
    <h3>Round-trip status</h3>
    <p class="small" id="roundtrip-strip"></p>
  </section>

  <hr class="rule">

  <section id="view-caveats" aria-labelledby="h-caveats">
    <h2 id="h-caveats">Fairness caveats, verbatim</h2>
    <p class="small">Quoted at generation time from <span class="num" id="caveat-src"></span>.
    The page cannot drift from the methodology it reports.</p>
    <ol class="caveats" id="caveats"></ol>
    <h3>Baseline geometry coverage (compression corpus), verbatim</h3>
    <div id="caveats-comp"></div>
    <h3>Codec levels</h3>
    <div class="verbatim" id="codec-note-verbatim"></div>
  </section>

  <hr class="rule">

  <section id="view-coverage" aria-labelledby="h-coverage">
    <h2 id="h-coverage">Coverage notes &mdash; what is missing, and why</h2>
    <ul class="notes" id="coverage"></ul>
  </section>
</main>
"""

# --------------------------------------------------------------------------
# JS — all rendering.
# --------------------------------------------------------------------------

JS = r"""
(function () {
  "use strict";

  var DATA = JSON.parse(document.getElementById("bench-data").textContent);
  var META = DATA.meta;
  var DATASETS = DATA.datasets;
  var FLOOR = META.citation_floor_s;

  /* The FORMAT-COMPARISON axis, from META.format_axis (mirroring the harness's
     own Format::DEFAULT_SET): the formats a city model can ship as, one per
     family, CityParquet as the Hilbert-ordered package it would ship as.
     cityjsonseq-gz (a compression variant of a format already here) and
     duckdb-parquet (an SQL-engine baseline) are not formats and are not on it —
     a panel putting gzipped CityJSONSeq beside CityJSONSeq compares a codec.
     Their labels stay defined for the coverage notes. */
  var FORMATS = (META.format_axis || []).slice();
  var SIZE_FORMATS = FORMATS.slice();
  var ABBR = {
    "cityparquet": "cpq", "cityparquet-hilbert": "cpq-h", "flatcitybuf": "fcb",
    "cityjsonseq": "cjs", "cityjsonseq-gz": "cjs-gz", "duckdb-parquet": "ddb",
    "citygml": "gml", "cityjson": "cj"
  };
  var SHORT = {
    "cityparquet": "CityParquet", "cityparquet-hilbert": "CityParquet (Hilbert)",
    "flatcitybuf": "FlatCityBuf", "cityjsonseq": "CityJSONSeq (baseline)",
    "cityjsonseq-gz": "CityJSONSeq+gz", "duckdb-parquet": "DuckDB on CityParquet",
    "citygml": "CityGML", "cityjson": "CityJSON"
  };
  var OFF_AXIS = ["cityjsonseq-gz", "duckdb-parquet"];
  var SCEN_ORDER = ["full-read", "count", "bbox-1pct", "bbox-5pct", "bbox-25pct",
                    "attr-filter", "attr-stats", "id-lookup", "project"];

  /* Which formats this run actually measured. A run carries the formats it was
     asked for — the 2026-08-17 corpus run carried three of the six — and the
     page's own sentences count them rather than assuming all six. */
  var MEASURED = {};
  DATA.read.forEach(function (r) {
    if (r.time_ratio != null || r.format === META.baseline) { MEASURED[r.format] = true; }
  });
  var MEASURED_SIZES = {};
  DATA.sizes.forEach(function (r) {
    if (r.frac_of_baseline != null) { MEASURED_SIZES[r.format] = true; }
  });

  /* ---------------- small helpers ---------------- */

  function esc(s) {
    return String(s).replace(/&/g, "&amp;").replace(/</g, "&lt;")
      .replace(/>/g, "&gt;").replace(/"/g, "&quot;");
  }
  function num(n) { return n == null ? "n/a" : Number(n).toLocaleString("en-US"); }
  function ratio(v) {
    if (v == null || !isFinite(v)) { return "n/a"; }
    var a = Math.abs(v), s;
    if (a >= 1000) { s = Math.round(v).toLocaleString("en-US"); }
    else if (a >= 10) { s = String(Math.round(v)); }
    else if (a >= 1) { s = v.toFixed(1); }
    else if (a >= 0.1) { s = v.toFixed(2); }
    else if (a >= 0.01) { s = v.toFixed(3); }
    else { s = Number(v.toPrecision(2)).toString(); }
    return s + "×";
  }
  function secs(v) {
    if (v == null) { return "n/a"; }
    if (v < 1) { return (v * 1000).toFixed(v < 0.01 ? 2 : 1) + " ms"; }
    return v.toFixed(3) + " s";
  }
  function bytes(b) {
    if (b == null) { return "n/a"; }
    var u = ["B", "kB", "MB", "GB"], i = 0, v = b;
    while (v >= 1024 && i < 3) { v /= 1024; i += 1; }
    return (i === 0 ? String(v) : v.toFixed(1)) + " " + u[i];
  }
  function el(id) { return document.getElementById(id); }

  /* ---------------- indices ---------------- */

  var READ = {};          // dataset -> scenario -> format -> record
  var GRAIN = {};         // scenario -> grain_comparable
  DATA.read.forEach(function (r) {
    if (!READ[r.dataset]) { READ[r.dataset] = {}; }
    if (!READ[r.dataset][r.scenario_key]) { READ[r.dataset][r.scenario_key] = {}; }
    READ[r.dataset][r.scenario_key][r.format] = r;
    GRAIN[r.scenario_key] = r.grain_comparable;
  });
  function rec(ds, sc, f) {
    return (READ[ds] && READ[ds][sc] && READ[ds][sc][f]) || null;
  }
  var SIZES = {};         // dataset -> format -> record
  DATA.sizes.forEach(function (s) {
    if (!SIZES[s.dataset]) { SIZES[s.dataset] = {}; }
    SIZES[s.dataset][s.format] = s;
  });
  var COMP = {};          // dataset -> [records]
  DATA.compression.forEach(function (c) {
    if (!COMP[c.dataset]) { COMP[c.dataset] = []; }
    COMP[c.dataset].push(c);
  });
  var COMP_DATASETS = DATASETS.map(function (d) { return d.id; })
    .filter(function (id) { return COMP[id]; });

  function dagger(sc) {
    if (GRAIN[sc] === false) {
      return '<a class="dag" href="#caveat-1" title="counting grain differs across formats' +
        ' in this scenario — see caveat 1">†</a>';
    }
    return "";
  }

  /* ---------------- scales ---------------- */

  function logScale(dmin, dmax, r0, r1) {
    var a = Math.log10(dmin), b = Math.log10(dmax);
    if (!isFinite(a) || !isFinite(b) || b - a < 1e-9) { a -= 0.5; b += 0.5; }
    return function (v) { return r0 + (Math.log10(v) - a) / (b - a) * (r1 - r0); };
  }
  function linScale(dmin, dmax, r0, r1) {
    var span = dmax - dmin || 1;
    return function (v) { return r0 + (v - dmin) / span * (r1 - r0); };
  }
  function logTicks(dmin, dmax) {
    var t = [], e, v;
    if (!(dmin > 0) || !(dmax > 0) || !isFinite(dmin) || !isFinite(dmax)) { return t; }
    for (e = Math.floor(Math.log10(dmin)); e <= Math.ceil(Math.log10(dmax)); e += 1) {
      v = Math.pow(10, e);
      if (v >= dmin && v <= dmax) { t.push(v); }
    }
    if (t.length < 3) {
      for (e = Math.floor(Math.log10(dmin)); e <= Math.ceil(Math.log10(dmax)); e += 1) {
        v = 3 * Math.pow(10, e);
        if (v >= dmin && v <= dmax) { t.push(v); }
      }
      t.sort(function (a, b) { return a - b; });
    }
    return t;
  }
  function tickText(v) {
    if (v >= 1) { return num(v) + "×"; }
    return Number(v.toPrecision(2)).toString() + "×";
  }

  /* ---------------- markers ---------------- */

  function marker(fmt, x, y, s) {
    var g = "";
    x = Math.round(Number(x) * 10) / 10;
    y = Math.round(Number(y) * 10) / 10;
    if (fmt === "cityparquet") {
      g = '<circle cx="' + x + '" cy="' + y + '" r="' + s + '" class="mk-accent"/>';
    } else if (fmt === "cityparquet-hilbert") {
      g = '<circle cx="' + x + '" cy="' + y + '" r="' + s + '" class="mk-open"/>';
    } else if (fmt === "cityjsonseq") {
      g = '<path class="mk-line mk-gray" d="M' + (x - s) + ' ' + (y - s) + 'L' + (x + s) +
        ' ' + (y + s) + 'M' + (x - s) + ' ' + (y + s) + 'L' + (x + s) + ' ' + (y - s) + '"/>';
    } else if (fmt === "cityjsonseq-gz") {
      g = '<rect x="' + (x - s) + '" y="' + (y - s) + '" width="' + (2 * s) +
        '" height="' + (2 * s) + '" class="mk-gray"/>';
    } else if (fmt === "citygml") {
      /* star: distinct from the triangle at panel scale, and from every filled
         round or square mark. Same shape as the print figures use. */
      var pts = "";
      for (var k = 0; k < 10; k++) {
        var rr = k % 2 ? s * 0.5 : s * 1.35;
        var aa = -Math.PI / 2 + (k * Math.PI) / 5;
        pts += (k ? "L" : "M") + (x + rr * Math.cos(aa)).toFixed(1) + " " +
          (y + rr * Math.sin(aa)).toFixed(1);
      }
      g = '<path class="mk-gray" d="' + pts + 'Z"/>';
    } else if (fmt === "cityjson") {
      var t = s * 0.45;
      g = '<path class="mk-gray" d="M' + (x - t) + ' ' + (y - s) + 'h' + (2 * t) + 'v' +
        (s - t) + 'h' + (s - t) + 'v' + (2 * t) + 'h' + (-(s - t)) + 'v' + (s - t) +
        'h' + (-2 * t) + 'v' + (-(s - t)) + 'h' + (-(s - t)) + 'v' + (-2 * t) + 'h' +
        (s - t) + 'Z"/>';
    } else if (fmt === "flatcitybuf") {
      g = '<path class="mk-gray" d="M' + x + ' ' + (y - s * 1.2) + 'L' + (x + s * 1.15) +
        ' ' + (y + s * 0.9) + 'L' + (x - s * 1.15) + ' ' + (y + s * 0.9) + 'Z"/>';
    } else {
      g = '<path class="mk-gray" d="M' + x + ' ' + (y - s * 1.25) + 'L' + (x + s * 1.15) +
        ' ' + y + 'L' + x + ' ' + (y + s * 1.25) + 'L' + (x - s * 1.15) + ' ' + y + 'Z"/>';
    }
    return g;
  }
  function point(fmt, x, y, s, tip) {
    x = Number(x);
    y = Number(y);
    return '<g class="pt" tabindex="0" role="img" aria-label="' + esc(tip.replace(/\n/g, ". ")) +
      '" data-tip="' + esc(tip) + '">' +
      '<circle class="halo" cx="' + x + '" cy="' + y + '" r="' + (s + 3) +
      '" fill="none" stroke="none"/>' +
      '<circle cx="' + x + '" cy="' + y + '" r="' + (s + 3) + '" fill="transparent"/>' +
      marker(fmt, x, y, s) + "</g>";
  }

  /* ---------------- direct-label placement ----------------
     Greedy placement against a set of occupied boxes: try the point's right,
     left, above and below in turn, then progressively further out. A label
     that cannot be placed without colliding is dropped — the tooltip still
     carries it — because an overlapping label is worse than no label. */

  function placeLabels(items, bounds, cls, extra) {
    var CH = cls === "dlabel" ? 5.2 : 4.8;   /* per-char width at 9 px / 8 px */
    var LH = cls === "dlabel" ? 9 : 8;
    var occupied = items.map(function (it) {
      return { x0: it.px - 4.5, y0: it.py - 4.5, x1: it.px + 4.5, y1: it.py + 4.5 };
    }).concat(extra || []);
    /* offsets clear the point's own marker box (±4.5) before anything else */
    var cands = [
      { dx: 7, dy: 2.5, a: "start" }, { dx: -7, dy: 2.5, a: "end" },
      { dx: 0, dy: -7, a: "middle" }, { dx: 0, dy: 12, a: "middle" },
      { dx: 7, dy: -6, a: "start" }, { dx: -7, dy: -6, a: "end" },
      { dx: 7, dy: 11, a: "start" }, { dx: -7, dy: 11, a: "end" },
      { dx: 0, dy: -16, a: "middle" }, { dx: 0, dy: 21, a: "middle" },
      { dx: 0, dy: -25, a: "middle" }, { dx: 0, dy: 30, a: "middle" },
      { dx: 0, dy: -34, a: "middle" }, { dx: 0, dy: 39, a: "middle" }
    ];
    var svg = "";
    items.slice().sort(function (a, b) { return a.py - b.py; }).forEach(function (it) {
      var w = it.text.length * CH, i, c, x, y, bx0, box, hit;
      for (i = 0; i < cands.length; i += 1) {
        c = cands[i];
        x = it.px + c.dx;
        y = it.py + c.dy;
        bx0 = c.a === "start" ? x : (c.a === "end" ? x - w : x - w / 2);
        box = { x0: bx0 - 1.6, y0: y - LH + 1, x1: bx0 + w + 1.6, y1: y + 2.5 };
        if (box.x0 < bounds.x0 || box.x1 > bounds.x1 ||
            box.y0 < bounds.y0 || box.y1 > bounds.y1) { continue; }
        hit = occupied.some(function (o) {
          return !(box.x1 < o.x0 || box.x0 > o.x1 || box.y1 < o.y0 || box.y0 > o.y1);
        });
        if (hit) { continue; }
        occupied.push(box);
        if (Math.abs(c.dy) > 12) {            /* hairline leader for far offsets */
          svg += '<line class="leader" x1="' + it.px.toFixed(1) + '" y1="' +
            it.py.toFixed(1) + '" x2="' + x.toFixed(1) + '" y2="' +
            (c.dy < 0 ? y + 2 : y - 6).toFixed(1) + '"/>';
        }
        svg += '<text class="' + cls + '" x="' + x.toFixed(1) + '" y="' + y.toFixed(1) +
          '" text-anchor="' + c.a + '">' + esc(it.text) + "</text>";
        return;
      }
      /* nowhere to put it without an overlap — leave it to the tooltip */
    });
    return svg;
  }

  /* ---------------- tooltip ---------------- */

  var tip = el("tip");
  function showTip(text, x, y) {
    tip.textContent = text;
    tip.hidden = false;
    var r = tip.getBoundingClientRect();
    var left = Math.min(x + 12, window.innerWidth - r.width - 8);
    var top = y - r.height - 12;
    if (top < 4) { top = y + 18; }
    tip.style.left = Math.max(4, left) + "px";
    tip.style.top = top + "px";
  }
  function hideTip() { tip.hidden = true; }
  document.addEventListener("mouseover", function (ev) {
    var t = ev.target.closest ? ev.target.closest(".pt") : null;
    if (t) { showTip(t.getAttribute("data-tip"), ev.clientX, ev.clientY); }
  });
  document.addEventListener("mouseout", function (ev) {
    if (ev.target.closest && ev.target.closest(".pt")) { hideTip(); }
  });
  document.addEventListener("focusin", function (ev) {
    var t = ev.target.closest ? ev.target.closest(".pt") : null;
    if (t) {
      var b = t.getBoundingClientRect();
      showTip(t.getAttribute("data-tip"), b.left + b.width / 2, b.top);
    }
  });
  document.addEventListener("focusout", hideTip);
  document.addEventListener("keydown", function (ev) {
    if (ev.key === "Escape") { hideTip(); }
  });

  /* ---------------- header / how to read ---------------- */

  /* The title and the lede are claims about the corpus, so they are computed
     from it. Typed by hand they survive exactly until the next benchmark run,
     which is how the first edition of this page came to open with "Eleven
     CityJSON corpora, six formats" over a 21-dataset, three-format run. */
  var CP_SERIES = FORMATS.filter(function (f) {
    return f.indexOf("cityparquet") === 0 && MEASURED[f];
  })[0];

  function medianOf(list) {
    var v = list.filter(function (x) { return x != null; }).sort(function (a, b) {
      return a - b;
    });
    if (!v.length) { return null; }
    var m = v.length >> 1;
    return v.length % 2 ? v[m] : (v[m - 1] + v[m]) / 2;
  }

  function medianRatio(scenario, fmt, key) {
    return medianOf(DATA.read.filter(function (r) {
      return r.scenario_key === scenario && r.format === fmt;
    }).map(function (r) { return r[key]; }));
  }

  function renderHeadline() {
    var scenarios = {};
    DATA.read.forEach(function (r) { scenarios[r.scenario_key] = true; });
    var scenList = Object.keys(scenarios);
    var nFmt = Object.keys(MEASURED).length;
    var full = CP_SERIES ? medianRatio("full-read", CP_SERIES, "time_ratio") : null;
    var selective = CP_SERIES ? medianOf(scenList.filter(function (sc) {
      return sc !== "full-read";
    }).map(function (sc) { return medianRatio(sc, CP_SERIES, "time_ratio"); })) : null;
    var sizeMed = medianOf(DATA.sizes.filter(function (r) {
      return r.format === CP_SERIES || r.format === "cityparquet";
    }).map(function (r) { return r.frac_of_baseline; }));

    var title = "CityParquet benchmarks — read speed, memory and size against CityJSONSeq";
    if (full != null && selective != null) {
      title = selective < 1 && full >= 0.9
        ? "CityParquet dominates the selective read and pays for the full read"
        : (selective < 1 && full < 0.9
            ? "CityParquet reads faster than CityJSONSeq, selectively and in full"
            : "CityParquet trades read time for selectivity");
    }
    el("headline").textContent = title;
    document.title = title;

    var parts = [
      "<b>" + DATASETS.length + " CityJSON corpora, " + nFmt + " formats, " +
        scenList.length + " read scenarios</b> — every number on this page is a ratio " +
        "against the same CityJSONSeq baseline."
    ];
    if (CP_SERIES && selective != null && full != null) {
      parts.push("Column pruning and predicate push-down put " + SHORT[CP_SERIES] +
        " at a median " + ratio(selective) + " of the baseline's time across the " +
        "selective scenarios, against " + ratio(full) + " for materialising every " +
        "CityObject" + (sizeMed != null ? ", at a median " + ratio(sizeMed) +
        " of the bytes on disk" : "") + ".");
    }
    el("lede").innerHTML = parts.join(" ");
  }

  el("provenance").innerHTML =
    "Baseline " + esc(META.baseline) + " = 1× &middot; read data " +
    '<span class="num">' + esc(META.sources.read) + "</span> &middot; sizes " +
    '<span class="num">' + esc(META.sources.sizes) + "</span> &middot; compression " +
    '<span class="num">' + esc(META.sources.compression) + "</span> &middot; " +
    num(DATA.read.length) + " read records over " + DATASETS.length + " datasets.";
  el("caveat-src").textContent =
    "cityparquet-rs/bench/READ_BENCHMARK.md and cityparquet-rs/bench/README.md";

  el("howto").innerHTML = [
    "<p><b>Section 0 is the summary; sections 1 to 4 are the evidence.</b> " +
      'The <a href="#view-overview">format profiles</a> collapse the corpus to one median ' +
      "position per format — the slopegraph for the shape of the trade-off, the dot-strips " +
      "for whether that median is a consistent win or an average over disagreement. " +
      "Everything after it is dataset by dataset.</p>",
    "<p><b>Everything is a ratio.</b> Each value divides that format's measurement by the " +
      "CityJSONSeq measurement for the same dataset and scenario. CityJSONSeq is therefore " +
      "exactly 1× everywhere, drawn as a cross or a dashed reference line.</p>",
    "<p><b>Lower and to the left is better.</b> On the Pareto panels the horizontal axis is " +
      "time and the vertical axis is memory, both logarithmic; a point below and left of the " +
      "baseline cross is both faster and leaner than CityJSONSeq.</p>",
    "<p><b>Differences under " + (FLOOR * 1000).toFixed(0) + "&nbsp;ms are noise.</b> " +
      'The shaded vertical band in each Pareto panel is ±' + (FLOOR * 1000).toFixed(0) +
      "&nbsp;ms expressed as a ratio, so it is wide for small datasets and a sliver for " +
      'Zurich. Heatmap cells inside the floor are muted and prefixed "≈" ' +
      '(<a href="#caveat-8">caveat 8</a>).</p>',
    "<p><b>† marks a grain mismatch.</b> In <i>full-read</i>, <i>count</i> and the " +
      "<i>bbox</i> scenarios CityParquet counts one row per CityObject while CityJSONSeq and " +
      "FlatCityBuf count top-level features. Those ratios compare different units of work " +
      '(<a href="#caveat-1">caveat 1</a>). The four unmarked scenarios are grain-comparable.</p>',
    "<p><b>Two formats carry asterisks of their own.</b> " +
      "<i>DuckDB on CityParquet</i> runs out of process: it reports no heap figure and carries " +
      "roughly 0.06&nbsp;s of un-subtracted start-up in every timing " +
      '(<a href="#caveat-6">caveat 6</a>). <i>id-lookup</i> samples a table-order-first ' +
      'identifier, which favours linear-scan formats (<a href="#caveat-9">caveat 9</a>).</p>',
    "<p><b>Shapes, not colours, carry identity.</b> CityParquet is the accented filled circle " +
      "and CityParquet&nbsp;(Hilbert) the open one; CityJSONSeq is a cross, CityJSONSeq+gz a " +
      "square, FlatCityBuf a triangle, DuckDB a diamond. Panels are labelled directly in the " +
      "first cell of each grid; hover or tab to any point for its absolute numbers.</p>"
  ].join("");

  /* =========================================================
     VIEW 0 — overview: format profiles
     Two questions the per-dataset grids cannot answer: what shape is the
     trade-off, and is the win the same everywhere.  Both charts read the same
     ratios as the views below; the medians are computed here, over the
     non-null values only, and never over an imputed one.
     ========================================================= */

  var NONBASE = FORMATS.filter(function (f) { return f !== META.baseline; });

  function median(vals) {
    var v = vals.filter(function (x) { return x != null && isFinite(x); })
      .sort(function (a, b) { return a - b; });
    if (!v.length) { return null; }
    var m = Math.floor(v.length / 2);
    return v.length % 2 ? v[m] : (v[m - 1] + v[m]) / 2;
  }
  function isAccent(f) {
    return f === "cityparquet" || f === "cityparquet-hilbert";
  }
  function readVals(sc, f, key) {
    var out = [];
    DATASETS.forEach(function (d) {
      var r = rec(d.id, sc, f);
      if (r && r[key] != null && isFinite(r[key]) && r[key] > 0) {
        out.push({ ds: d.id, v: r[key], floor: r.below_floor === true });
      }
    });
    return out;
  }
  function sizeVals(f) {
    var out = [];
    DATASETS.forEach(function (d) {
      var s = SIZES[d.id] && SIZES[d.id][f];
      if (s && s.frac_of_baseline != null && s.frac_of_baseline > 0) {
        out.push({ ds: d.id, v: s.frac_of_baseline, floor: false });
      }
    });
    return out;
  }
  function extent(vals) {
    var v = vals.map(function (o) { return o.v; });
    return v.length
      ? { lo: Math.min.apply(null, v), hi: Math.max.apply(null, v) }
      : null;
  }
  function floorCount(vals) {
    return vals.filter(function (o) { return o.floor; }).length;
  }
  /* share of a scenario's non-baseline time deltas that sit inside the floor */
  function floorShare(sc) {
    var tot = 0, below = 0;
    DATASETS.forEach(function (d) {
      NONBASE.forEach(function (f) {
        var r = rec(d.id, sc, f);
        if (r && r.time_ratio != null) {
          tot += 1;
          if (r.below_floor) { below += 1; }
        }
      });
    });
    return { n: tot, below: below, mostly: tot > 0 && below * 2 >= tot };
  }

  /* End-of-line labels for the slopegraph.  placeLabels() is the right tool
     when points are scattered in two dimensions; here every label belongs to
     one x column, so a monotone vertical dodge is both simpler and stronger —
     it can never drop a label, never place one over the plot, and never let
     two overlap.  Labels moved off their point get the same hairline leader. */
  function placeEndLabels(items, bounds) {
    var GAP = 11, byX = {}, svg = "";
    items.forEach(function (it) {
      var k = String(it.px);
      if (!byX[k]) { byX[k] = []; }
      byX[k].push(it);
    });
    Object.keys(byX).forEach(function (k) {
      var col = byX[k].slice().sort(function (a, b) { return a.py - b.py; });
      var prev = -Infinity, over;
      col.forEach(function (it) {
        it.ly = Math.max(it.py, prev + GAP);
        prev = it.ly;
      });
      over = col.length ? col[col.length - 1].ly - bounds.y1 : 0;
      if (over > 0) {
        col.slice().reverse().forEach(function (it) {
          it.ly -= over;
          if (it.ly < bounds.y0) { it.ly = bounds.y0; }
        });
      }
      col.forEach(function (it) {
        var x = it.px + 8;
        if (Math.abs(it.ly - it.py) > 2.5) {
          svg += '<line class="leader" x1="' + (it.px + 4.5).toFixed(1) + '" y1="' +
            it.py.toFixed(1) + '" x2="' + (x - 1.5).toFixed(1) + '" y2="' +
            (it.ly - 3).toFixed(1) + '"/>';
        }
        svg += '<text class="' + (it.cls || "dlabel") + '" x="' + x + '" y="' +
          it.ly.toFixed(1) + '" text-anchor="start">' + esc(it.text) + "</text>";
      });
    });
    return svg;
  }

  /* ---------------- 0a — trade-off slopegraph ---------------- */

  var slopeState = { scenario: "full-read" };

  (function buildSlopeSelect() {
    var sel = el("slope-scen"), html = "";
    SCEN_ORDER.forEach(function (sc) {
      var n = scenarioCoverage(sc);
      html += '<option value="' + esc(sc) + '"' + (n === 0 ? " disabled" : "") +
        (sc === slopeState.scenario ? " selected" : "") + ">" + esc(sc) +
        (GRAIN[sc] === false ? " †" : "") +
        " (" + n + "/" + DATASETS.length + " datasets)</option>";
    });
    sel.innerHTML = html;
    sel.addEventListener("change", function () {
      slopeState.scenario = sel.value;
      renderSlope();
    });
  }());

  function renderSlope() {
    var sc = slopeState.scenario;
    var W = 740, H = 392, ML = 94, MR = 178, MT = 66, MB = 66;
    var x1 = ML, x3 = W - MR, x2 = Math.round((x1 + x3) / 2);
    var AX = [
      { x: x1, key: "time_ratio", title: "read time",
        sub: "median ratio (log)", sub2: sc },
      { x: x2, key: "rss_ratio", title: "peak RSS",
        sub: "median ratio (log)", sub2: sc },
      { x: x3, key: "size", title: "bytes on disk",
        sub: "fraction of baseline", sub2: "scenario-independent" }
    ];

    var all = [1], series = {};
    NONBASE.forEach(function (f) {
      series[f] = AX.map(function (a) {
        var vals = a.key === "size" ? sizeVals(f) : readVals(sc, f, a.key);
        var med = median(vals.map(function (o) { return o.v; }));
        vals.forEach(function (o) { all.push(o.v); });
        if (med != null) { all.push(med); }
        return { vals: vals, med: med, ext: extent(vals) };
      });
    });

    var lo = Math.min.apply(null, all) * 0.55, hi = Math.max.apply(null, all) * 1.8;
    var sy = logScale(lo, hi, H - MB, MT);
    var svg = "", labels = [];

    /* the 1× baseline, spanning all three axes */
    svg += '<line class="refline" x1="' + (x1 - 30) + '" y1="' + sy(1).toFixed(1) +
      '" x2="' + (x3 + 30) + '" y2="' + sy(1).toFixed(1) + '"/>';

    /* value ticks, in the left margin of the first axis only */
    logTicks(lo, hi).forEach(function (t) {
      svg += '<text class="tick" x="' + (x1 - 34) + '" y="' + (sy(t) + 3).toFixed(1) +
        '" text-anchor="end">' + esc(tickText(t)) + "</text>";
    });
    svg += '<text class="tick" x="' + (x1 - 34) + '" y="' + (sy(1) + 13).toFixed(1) +
      '" text-anchor="end">CityJSONSeq</text>';

    var fl = floorShare(sc);
    AX.forEach(function (a, ai) {
      /* range-frame: the axis line covers the data it carries, nothing more */
      var lows = [], highs = [], nds = {};
      NONBASE.forEach(function (f) {
        var s = series[f][ai];
        if (s.ext) { lows.push(s.ext.lo); highs.push(s.ext.hi); }
        s.vals.forEach(function (o) { nds[o.ds] = 1; });
      });
      if (lows.length) {
        svg += '<line class="axis" x1="' + a.x + '" y1="' +
          sy(Math.min.apply(null, lows)).toFixed(1) + '" x2="' + a.x + '" y2="' +
          sy(Math.max.apply(null, highs)).toFixed(1) + '"/>';
      }
      svg += '<text class="axtitle" x="' + a.x + '" y="' + (MT - 36) +
        '" text-anchor="middle">' + esc(a.title) + "</text>";
      svg += '<text class="tick" x="' + a.x + '" y="' + (MT - 25) +
        '" text-anchor="middle">' + esc(a.sub) + "</text>";
      svg += '<text class="tick" x="' + a.x + '" y="' + (MT - 15) +
        '" text-anchor="middle">' + esc(a.sub2) + (GRAIN[sc] === false &&
          a.key !== "size" ? " †" : "") + "</text>";
      var n = Object.keys(nds).length;
      svg += '<text class="tick" x="' + a.x + '" y="' + (H - MB + 20) +
        '" text-anchor="middle">median of ' + n + " dataset" +
        (n === 1 ? "" : "s") + "</text>";
      if (a.key === "time_ratio" && fl.mostly) {
        svg += '<text class="tick warn" x="' + a.x + '" y="' + (H - MB + 33) +
          '" text-anchor="middle">mostly &lt; ' + (FLOOR * 1000).toFixed(0) +
          " ms — not citable</text>";
      }
    });

    /* individual datasets: faint ticks straddling the axis */
    NONBASE.forEach(function (f) {
      AX.forEach(function (a, ai) {
        series[f][ai].vals.forEach(function (o) {
          svg += '<line class="dtick' + (isAccent(f) ? " acc" : "") + '" x1="' +
            (a.x - 6) + '" y1="' + sy(o.v).toFixed(1) + '" x2="' + (a.x + 6) +
            '" y2="' + sy(o.v).toFixed(1) + '"><title>' +
            esc(o.ds + " · " + SHORT[f] + " · " + a.title + " " + ratio(o.v)) +
            "</title></line>";
        });
      });
    });

    /* one polyline per format through its medians */
    NONBASE.forEach(function (f) {
      var pts = [], d;
      AX.forEach(function (a, ai) {
        if (series[f][ai].med != null) { pts.push({ x: a.x, y: sy(series[f][ai].med) }); }
      });
      if (pts.length > 1) {
        d = "M" + pts.map(function (p) {
          return p.x + " " + p.y.toFixed(1);
        }).join("L");
        svg += '<path class="slope' + (isAccent(f) ? " accent" : "") +
          (f === "cityparquet-hilbert" ? " dash" : "") + '" d="' + d + '"/>';
      }
      AX.forEach(function (a, ai) {
        var s = series[f][ai];
        if (s.med == null) { return; }
        var kf = a.key === "time_ratio" ? floorCount(s.vals) : 0;
        var t = SHORT[f] + "\n" + a.title + " · median " + ratio(s.med) +
          " over " + s.vals.length + " dataset" + (s.vals.length === 1 ? "" : "s") +
          (a.key === "size" ? "" : " · " + sc) +
          "\nrange " + ratio(s.ext.lo) + " – " + ratio(s.ext.hi) +
          (a.key === "time_ratio"
            ? "\n" + kf + " of " + s.vals.length + " time deltas are inside the " +
              (FLOOR * 1000).toFixed(0) + " ms citation floor"
            : "") +
          (s.vals.length < DATASETS.length
            ? "\nn = " + s.vals.length + " of " + DATASETS.length +
              " datasets — the rest carry no value here"
            : "") +
          (f === "duckdb-parquet" && a.key === "time_ratio"
            ? "\nincludes ~0.06 s un-subtracted start-up" : "");
        svg += point(f, a.x, sy(s.med), 3.4, t);
      });
      /* the direct label goes at the last axis the format reaches */
      var last = null;
      AX.forEach(function (a, ai) {
        if (series[f][ai].med != null) { last = { x: a.x, y: sy(series[f][ai].med) }; }
      });
      if (last) {
        labels.push({ px: last.x, py: last.y, text: SHORT[f] });
        if (f === "duckdb-parquet") {
          labels.push({ px: last.x, py: last.y + 0.1, cls: "tick",
                        text: "no artefact to size" });
        }
      }
    });

    svg += placeEndLabels(labels, { y0: MT - 8, y1: H - MB + 6 });

    var notes = [
      "Each polyline joins one format's median across the datasets that carry the " +
        "selected scenario; the faint ticks are those datasets individually, so the " +
        "spread the median summarises stays on the page."
    ];
    if (series["duckdb-parquet"]) {
      notes.push(series["duckdb-parquet"][0].med == null
        ? "DuckDB on CityParquet has no line here: it was not benchmarked on this " +
          "scenario, and it writes no artefact of its own to size."
        : "DuckDB on CityParquet reads the CityParquet package rather than writing an " +
          "artefact of its own, so its line stops at the peak-RSS axis.");
    }
    if (fl.mostly) {
      notes.push(fl.below + " of this scenario's " + fl.n + " time deltas are inside the " +
        (FLOOR * 1000).toFixed(0) + " ms citation floor — the time axis is measuring " +
        "noise more often than not here, so read it as a tie.");
    }
    if (GRAIN[sc] === false) {
      notes.push("† " + sc + " compares different counting grains across formats — " +
        "one row per CityObject against one top-level feature " +
        '(<a href="#caveat-1">caveat 1</a>).');
    }
    if (sc === "id-lookup") {
      notes.push("The sampled identifier is table-order-first, which favours " +
        'linear-scan formats (<a href="#caveat-9">caveat 9</a>).');
    }

    var aria = "Trade-off slopegraph for " + sc + ". Medians against CityJSONSeq: " +
      NONBASE.map(function (f) {
        return SHORT[f] + " time " + ratio(series[f][0].med) + ", peak RSS " +
          ratio(series[f][1].med) + ", size " +
          (series[f][2].med == null ? "no artefact" : ratio(series[f][2].med));
      }).join("; ") + ".";

    el("slope-fig").innerHTML =
      '<svg viewBox="0 0 ' + W + " " + H + '" role="img" aria-label="' + esc(aria) +
      '">' + svg + "</svg>" +
      notes.map(function (t) { return '<p class="note">' + t + "</p>"; }).join("");

    el("slope-scen-note").textContent =
      scenarioCoverage(sc) + " of " + DATASETS.length + " datasets carry this scenario" +
      (GRAIN[sc] === false ? " — grain mismatch applies (caveat 1)" : "") +
      (sc === "id-lookup" ? " — table-order-first sampling bias (caveat 9)" : "");

    el("slope-lede").innerHTML =
      "Three criteria, five formats, one line each. Every axis is a ratio against " +
      "CityJSONSeq on the same logarithmic scale, so the dashed 1× line runs straight " +
      "across all three and <b>lower is better everywhere</b>: a line that stays low is " +
      "a format that wins on all three counts, a line that climbs is a trade. The first " +
      "two axes depend on the scenario you pick; on-disk size does not.";
  }

  /* ---------------- 0b — consistency dot-strips ---------------- */

  var stripState = { metric: "time" };
  var STRIP_KEY = { time: "time_ratio", rss: "rss_ratio", heap: "heap_ratio" };
  var STRIP_NAME = { time: "read time", rss: "peak RSS", heap: "peak heap" };

  Array.prototype.forEach.call(
    document.querySelectorAll('input[name="stripmetric"]'),
    function (r) {
      r.addEventListener("change", function () {
        if (r.checked) { stripState.metric = r.value; renderStrips(); }
      });
    }
  );

  function renderStrips() {
    var key = STRIP_KEY[stripState.metric];
    var isTime = stripState.metric === "time";
    var lo = Infinity, hi = 0;
    DATA.read.forEach(function (r) {
      if (NONBASE.indexOf(r.format) < 0) { return; }
      var v = r[key];
      if (v != null && isFinite(v) && v > 0) {
        if (v < lo) { lo = v; }
        if (v > hi) { hi = v; }
      }
    });
    if (!(lo < hi)) { lo = 0.1; hi = 10; }
    lo *= 0.55;
    hi *= 1.9;

    var W = 330, ML = 50, MR = 14, MT = 8, rowH = 17, MB = 24;
    var H = MT + NONBASE.length * rowH + MB;
    var sx = logScale(lo, hi, ML, W - MR);
    var ticks = logTicks(lo, hi);
    var step = Math.max(1, Math.ceil(ticks.length / 5));

    var out = "";
    SCEN_ORDER.forEach(function (sc) {
      var svg = "", finding = [], thin = [], nds = {};

      svg += '<line class="refline" x1="' + sx(1).toFixed(1) + '" y1="' + MT +
        '" x2="' + sx(1).toFixed(1) + '" y2="' + (MT + NONBASE.length * rowH) + '"/>';

      NONBASE.forEach(function (f, i) {
        var y = MT + i * rowH + rowH / 2;
        var vals = readVals(sc, f, key);
        svg += '<text class="tick" x="' + (ML - 6) + '" y="' + (y + 3) +
          '" text-anchor="end">' + esc(ABBR[f]) + "</text>";
        if (!vals.length) {
          svg += '<text class="tick" x="' + (ML + 4) + '" y="' + (y + 3) + '">' +
            (f === "duckdb-parquet" && stripState.metric === "heap"
              ? "n/a — out-of-process"
              : "n/a — not benchmarked here") + "</text>";
          return;
        }
        vals.forEach(function (o) {
          nds[o.ds] = 1;
          var hollow = isTime && o.floor;
          svg += '<circle class="' + (hollow ? "dot-open" : "dot") + '" cx="' +
            sx(o.v).toFixed(1) + '" cy="' + y + '" r="' + (hollow ? 2 : 1.8) +
            '"><title>' + esc(o.ds + " · " + SHORT[f] + " " + ratio(o.v) +
              (hollow ? " — inside the citation floor, a tie" : "")) +
            "</title></circle>";
        });
        var med = median(vals.map(function (o) { return o.v; }));
        var ex = extent(vals);
        var kf = floorCount(vals);
        var t = SHORT[f] + " · " + sc + "\nmedian " + STRIP_NAME[stripState.metric] +
          " ratio " + ratio(med) + " over " + vals.length + " dataset" +
          (vals.length === 1 ? "" : "s") +
          "\nrange " + ratio(ex.lo) + " – " + ratio(ex.hi) +
          (isTime ? "\n" + kf + " of " + vals.length +
            " inside the " + (FLOOR * 1000).toFixed(0) +
            " ms citation floor (drawn hollow)" : "") +
          (vals.length < DATASETS.length ? "\nn = " + vals.length + " of " +
            DATASETS.length + " datasets" : "");
        svg += point(f, sx(med), y, 3, t);
        finding.push(SHORT[f] + " " + ratio(med));
        thin.push({ f: f, n: vals.length });
      });

      var ay = MT + NONBASE.length * rowH;
      svg += '<line class="axis" x1="' + ML + '" y1="' + ay + '" x2="' + (W - MR) +
        '" y2="' + ay + '"/>';
      ticks.forEach(function (t, ti) {
        svg += '<line class="axis" x1="' + sx(t).toFixed(1) + '" y1="' + ay +
          '" x2="' + sx(t).toFixed(1) + '" y2="' + (ay + 2.5) + '"/>';
        if (ti % step === 0 || t === 1) {
          svg += '<text class="tick" x="' + sx(t).toFixed(1) + '" y="' + (ay + 12) +
            '" text-anchor="middle">' + esc(tickText(t)) + "</text>";
        }
      });

      var n = Object.keys(nds).length;
      /* only the rows whose n differs from the panel's coverage are worth naming */
      var odd = thin.filter(function (o) { return o.n !== n; }).map(function (o) {
        return ABBR[o.f] + " n=" + o.n;
      });
      var aria = sc + ": median " + STRIP_NAME[stripState.metric] +
        " ratio against CityJSONSeq over " + n + " datasets — " +
        finding.join(", ") + ".";
      out += '<figure class="panel"><figcaption><span class="name">' + esc(sc) +
        "</span>" + dagger(sc) +
        (sc === "id-lookup" ? '<a class="dag" href="#caveat-9" title="table-order-first ' +
          'sampling bias — see caveat 9">‡</a>' : "") +
        '<span class="sub">' + n + " of " + DATASETS.length + " datasets" +
        (odd.length ? " · " + esc(odd.join(", ")) : "") + "</span></figcaption>" +
        '<svg viewBox="0 0 ' + W + " " + H + '" role="img" aria-label="' + esc(aria) +
        '">' + svg + "</svg></figure>";
    });
    el("strip-grid").innerHTML = out;

    el("strip-lede").innerHTML =
      "One panel per scenario, one row per format, all panels on one shared logarithmic " +
      "scale. Each small grey dot is a single dataset and the larger marker is the " +
      "median, in that format's own shape: a tight cluster left of the dashed 1× line " +
      "is a win that holds everywhere, a smear across it is an average over disagreement. " +
      "CityJSONSeq is the reference line itself rather than a row. Rows: " +
      NONBASE.map(function (f) {
        return "<b>" + esc(ABBR[f]) + "</b> " + esc(SHORT[f]);
      }).join(" &middot; ") + ".";

    el("strip-metric-note").textContent = stripState.metric === "heap"
      ? "heap is the allocator's view — DuckDB runs out of process and reports none"
      : (stripState.metric === "rss"
        ? "peak RSS is the memory metric present for every format"
        : "hollow dots are inside the " + (FLOOR * 1000).toFixed(0) +
          " ms citation floor — measured, but a tie");

    el("view-overview").setAttribute("aria-label",
      "Format profiles across the corpus. The slopegraph shows CityParquet trading a " +
      "little full-read time and a lot of memory for roughly a third of the bytes and " +
      "one to two orders of magnitude on selective reads; the dot-strips show that " +
      "selective win holding on every dataset, while the full-read penalty is the " +
      "scenario where the formats cluster around 1 times.");
  }

  function renderOverview() {
    el("overview-rationale").textContent =
      "Two chart types, chosen by the shape of the data. A handful of formats judged on " +
      "three ordered criteria is a slopegraph — it makes the trade a line the eye can " +
      "follow. " + DATASETS.length + " datasets per format per scenario is a " +
      "distribution, so it gets a dot strip — a median alone would hide whether the " +
      "corpus agrees with itself.";
    renderSlope();
    renderStrips();
  }

  /* =========================================================
     VIEW 1 — Pareto grid
     ========================================================= */

  var paretoState = { scenario: "full-read", metric: "rss" };

  function scenarioCoverage(sc) {
    var n = 0;
    DATASETS.forEach(function (d) {
      var byF = READ[d.id] && READ[d.id][sc];
      if (!byF) { return; }
      var ok = FORMATS.some(function (f) {
        return byF[f] && byF[f].time_ratio != null;
      });
      if (ok) { n += 1; }
    });
    return n;
  }

  (function buildScenSelect() {
    var sel = el("scen-select"), html = "";
    SCEN_ORDER.forEach(function (sc) {
      var n = scenarioCoverage(sc);
      html += '<option value="' + esc(sc) + '"' + (n === 0 ? " disabled" : "") +
        (sc === paretoState.scenario ? " selected" : "") + ">" + esc(sc) +
        (GRAIN[sc] === false ? " †" : "") +
        " (" + n + "/" + DATASETS.length + " datasets)</option>";
    });
    sel.innerHTML = html;
    sel.addEventListener("change", function () {
      paretoState.scenario = sel.value;
      renderPareto();
    });
    Array.prototype.forEach.call(
      document.querySelectorAll('input[name="mem"]'),
      function (r) {
        r.addEventListener("change", function () {
          if (r.checked) { paretoState.metric = r.value; renderPareto(); }
        });
      }
    );
  }());

  function paretoFrontier(pts) {
    var s = pts.slice().sort(function (a, b) { return a.x - b.x || a.y - b.y; });
    var out = [], best = Infinity;
    s.forEach(function (p) {
      if (p.y < best - 1e-12) { out.push(p); best = p.y; }
    });
    return out;
  }

  function renderPareto() {
    var sc = paretoState.scenario;
    var key = paretoState.metric === "rss" ? "rss_ratio" : "heap_ratio";
    var memName = paretoState.metric === "rss" ? "peak RSS" : "peak heap";
    var xs = [1], ys = [1], i;

    DATASETS.forEach(function (d) {
      FORMATS.forEach(function (f) {
        var r = rec(d.id, sc, f);
        if (r && r.time_ratio != null && r[key] != null && r.time_ratio > 0 && r[key] > 0) {
          xs.push(r.time_ratio); ys.push(r[key]);
        }
      });
    });
    var xmin = Math.min.apply(null, xs) * 0.55, xmax = Math.max.apply(null, xs) * 1.8;
    var ymin = Math.min.apply(null, ys) * 0.7, ymax = Math.max.apply(null, ys) * 1.4;

    var W = 280, H = 205, ML = 34, MR = 12, MT = 10, MB = 30;
    var pw = W - ML - MR, ph = H - MT - MB;
    var sx = logScale(xmin, xmax, ML, ML + pw);
    var sy = logScale(ymin, ymax, MT + ph, MT);

    var out = "";
    DATASETS.forEach(function (d, di) {
      var byF = READ[d.id] && READ[d.id][sc];
      var svg = "", notes = [], pts = [], missing = [], nullRatio = [];

      if (byF) {
        FORMATS.forEach(function (f) {
          var r = byF[f];
          if (!r) { missing.push(f); return; }
          if (r.time_ratio == null || r[key] == null) { nullRatio.push(f); return; }
          pts.push({ x: r.time_ratio, y: r[key], f: f, r: r });
        });
      }

      /* citation-floor band */
      var base = byF && byF.cityjsonseq;
      if (base && base.time_s) {
        var w = FLOOR / base.time_s;
        var lo = Math.max(1 - w, xmin), hi = Math.min(1 + w, xmax);
        if (hi > lo) {
          svg += '<rect class="band" x="' + sx(lo).toFixed(1) + '" y="' + MT +
            '" width="' + Math.max(0.8, sx(hi) - sx(lo)).toFixed(1) + '" height="' + ph +
            '"><title>±' + (FLOOR * 1000).toFixed(0) + " ms citation floor: ±" +
            ratio(w) + " around 1×, because the CityJSONSeq baseline here takes " +
            secs(base.time_s) + ". Differences inside the band are noise.</title></rect>";
        }
      }

      /* reference lines at 1x */
      svg += '<line class="refline" x1="' + sx(1).toFixed(1) + '" y1="' + MT +
        '" x2="' + sx(1).toFixed(1) + '" y2="' + (MT + ph) + '"/>';
      svg += '<line class="refline" x1="' + ML + '" y1="' + sy(1).toFixed(1) +
        '" x2="' + (ML + pw) + '" y2="' + sy(1).toFixed(1) + '"/>';

      /* range-frame axes over the data extent */
      if (pts.length) {
        var exX = pts.map(function (p) { return p.x; }).concat([1]);
        var exY = pts.map(function (p) { return p.y; }).concat([1]);
        svg += '<line class="axis" x1="' + sx(Math.min.apply(null, exX)).toFixed(1) +
          '" y1="' + (MT + ph) + '" x2="' + sx(Math.max.apply(null, exX)).toFixed(1) +
          '" y2="' + (MT + ph) + '"/>';
        svg += '<line class="axis" x1="' + ML + '" y1="' +
          sy(Math.min.apply(null, exY)).toFixed(1) + '" x2="' + ML + '" y2="' +
          sy(Math.max.apply(null, exY)).toFixed(1) + '"/>';
      }

      /* ticks — their boxes are kept as no-go areas for the direct labels */
      var tickBoxes = [];
      logTicks(xmin, xmax).forEach(function (t) {
        var tw = tickText(t).length * 4.8;
        svg += '<text class="tick" x="' + sx(t).toFixed(1) + '" y="' + (MT + ph + 10) +
          '" text-anchor="middle">' + esc(tickText(t)) + "</text>";
        tickBoxes.push({ x0: sx(t) - tw / 2 - 2, y0: MT + ph + 2,
                         x1: sx(t) + tw / 2 + 2, y1: MT + ph + 12 });
      });
      logTicks(ymin, ymax).forEach(function (t) {
        var tw = tickText(t).length * 4.8;
        svg += '<text class="tick" x="' + (ML - 4) + '" y="' + (sy(t) + 3).toFixed(1) +
          '" text-anchor="end">' + esc(tickText(t)) + "</text>";
        tickBoxes.push({ x0: ML - 4 - tw - 2, y0: sy(t) - 5,
                         x1: ML - 2, y1: sy(t) + 5 });
      });

      /* pareto frontier */
      var fr = paretoFrontier(pts);
      if (fr.length > 1) {
        var dpath = "M" + sx(fr[0].x).toFixed(1) + " " + sy(fr[0].y).toFixed(1);
        for (i = 1; i < fr.length; i += 1) {
          dpath += "L" + sx(fr[i].x).toFixed(1) + " " + sy(fr[i - 1].y).toFixed(1) +
            "L" + sx(fr[i].x).toFixed(1) + " " + sy(fr[i].y).toFixed(1);
        }
        svg += '<path class="frontier" d="' + dpath + '"/>';
      }

      /* points */
      pts.forEach(function (p) {
        var absMem = paretoState.metric === "rss" ? p.r.rss_b : p.r.heap_b;
        var t = SHORT[p.f] + "\n" + d.id + " · " + sc +
          "\ntime " + ratio(p.x) + " (" + secs(p.r.time_s) + " ± " +
          secs(p.r.time_mad_s) + " MAD)" +
          "\n" + memName + " " + ratio(p.y) + " (" + bytes(absMem) + ")" +
          "\nresults " + num(p.r.result_count) +
          (p.r.below_floor ? "\ninside the " + (FLOOR * 1000).toFixed(0) +
            " ms citation floor — treat as a tie" : "") +
          (p.f === "duckdb-parquet" ? "\nincludes ~0.06 s un-subtracted start-up" : "");
        svg += point(p.f, sx(p.x).toFixed(1), sy(p.y).toFixed(1), 3, t);
      });

      /* direct labels — first panel only, pushed apart so they never collide */
      if (di === 0 && pts.length) {
        svg += placeLabels(pts.map(function (p) {
          return { px: sx(p.x), py: sy(p.y), text: ABBR[p.f] };
        }), { x0: 2, x1: W - 2, y0: MT + 6, y1: MT + ph + 1 }, "dlabel", tickBoxes);
      }

      if (di === 0) {
        svg += '<text class="axlabel" x="' + (ML + pw) + '" y="' + (MT + ph + 22) +
          '" text-anchor="end">time ratio (log)</text>';
        svg += '<text class="axlabel" transform="translate(' + (ML - 22) + ',' + (MT + ph) +
          ') rotate(-90)">' + esc(memName) + " ratio (log)</text>";
      }

      if (!byF) {
        notes.push("no data for this scenario");
      }
      if (missing.length) {
        notes.push("not benchmarked: " + missing.map(function (f) { return ABBR[f]; }).join(", "));
      }
      if (nullRatio.length) {
        var why = (byF && byF.cityjsonseq)
          ? "no " + (paretoState.metric === "heap" ? "heap figure" : "value")
          : "no CityJSONSeq baseline row, so no ratio exists";
        notes.push(nullRatio.map(function (f) { return ABBR[f]; }).join(", ") +
          ": " + why + (paretoState.metric === "heap" &&
            nullRatio.indexOf("duckdb-parquet") >= 0 ? " — out-of-process" : ""));
      }

      out += '<figure class="panel"><figcaption><span class="name">' + esc(d.id) +
        '</span><span class="sub">' + esc(d.subtitle) + "</span></figcaption>" +
        '<svg viewBox="0 0 ' + W + " " + H + '" role="img" aria-label="' +
        esc(d.id + ": " + sc + " — time and " + memName +
            " ratios against CityJSONSeq for " + pts.length + " formats") + '">' +
        svg + "</svg>" +
        (notes.length ? '<p class="note">' + esc(notes.join(" · ")) + "</p>" : "") +
        "</figure>";
    });

    el("pareto-grid").innerHTML = out;

    el("pareto-lede").innerHTML =
      "Each panel is one dataset, all panels share one scale. Horizontal: read time " +
      "against CityJSONSeq (log). Vertical: " + esc(memName) +
      " against CityJSONSeq (log). The cross at 1×, 1× is the baseline itself; " +
      "the shaded band is the " + (FLOOR * 1000).toFixed(0) +
      " ms citation floor; the dashed staircase is the Pareto frontier through the " +
      "non-dominated formats.";

    var covered = scenarioCoverage(sc);
    el("pareto-scen-note").textContent =
      covered + " of " + DATASETS.length + " datasets carry this scenario" +
      (GRAIN[sc] === false ? " — grain mismatch applies (caveat 1)" : "") +
      (sc === "id-lookup" ? " — table-order-first sampling bias (caveat 9)" : "");

    el("pareto-metric-note").innerHTML = paretoState.metric === "heap"
      ? "Heap is the allocator's view, not the operating system's: FlatCityBuf streams by " +
        "design and so shows a tiny heap, and DuckDB on CityParquet reports no heap at all " +
        'because it runs out of process (marked "n/a" in the panel notes). Peak RSS is the ' +
        "metric present for every format measured here and is the one to cite " +
        '(<a href="#caveat-6">caveat 6</a>).'
      : "Peak RSS is the primary memory metric: present for every format measured here, " +
        "and platform units cancel in the ratio.";

    var sect = el("view-pareto");
    sect.setAttribute("aria-label",
      "Speed against memory, " + DATASETS.length + " datasets. On selective scenarios " +
      "CityParquet sits below " +
      "and to the left of the CityJSONSeq baseline — faster and leaner; on full-read it " +
      "sits near 1 times in time and above it in memory.");
  }

  /* =========================================================
     VIEW 2 — speedup heatmap
     ========================================================= */

  function heatColor(speedup) {
    var t = Math.log2(speedup) / 6;                 /* +-64x saturates */
    if (t > 1) { t = 1; } else if (t < -1) { t = -1; }
    var pct = (Math.abs(t) * 55).toFixed(0);
    var base = t >= 0 ? "var(--heat-pos)" : "var(--heat-neg)";
    return "color-mix(in srgb, " + base + " " + pct + "%, transparent)";
  }

  function renderHeat() {
    var out = "";
    DATASETS.forEach(function (d) {
      var head = '<tr><th class="scen">' + esc(d.id) + "</th>";
      FORMATS.forEach(function (f) {
        head += '<th><abbr title="' + esc(SHORT[f]) + '">' + esc(ABBR[f]) + "</abbr></th>";
      });
      head += "</tr>";

      var body = "";
      SCEN_ORDER.forEach(function (sc) {
        body += '<tr><td class="scen">' + esc(sc) + dagger(sc) +
          (sc === "id-lookup" ? '<a class="dag" href="#caveat-9" title="table-order-first ' +
            'sampling bias — see caveat 9">‡</a>' : "") + "</td>";
        FORMATS.forEach(function (f) {
          var r = rec(d.id, sc, f);
          if (!r) {
            body += '<td class="na" title="' + esc(SHORT[f] + " — " + sc +
              " not benchmarked on " + d.id) + '">–</td>';
            return;
          }
          if (r.time_ratio == null || !(r.time_ratio > 0)) {
            body += '<td class="na" title="' + esc(SHORT[f] + " — " + secs(r.time_s) +
              " measured, but no CityJSONSeq baseline row for " + d.id + " / " + sc +
              ", so no ratio exists") + '">–</td>';
            return;
          }
          var sp = 1 / r.time_ratio;
          var label = (r.below_floor ? "≈" : "") + ratio(sp);
          var tipTxt = SHORT[f] + " — " + d.id + " / " + sc + ": " + ratio(sp) +
            " the speed of CityJSONSeq (" + secs(r.time_s) + " ± " +
            secs(r.time_mad_s) + " vs " + secs(rec(d.id, sc, "cityjsonseq") &&
              rec(d.id, sc, "cityjsonseq").time_s) + ")" +
            (r.below_floor ? " — inside the citation floor, treat as a tie" : "");
          body += '<td class="' + (r.below_floor ? "floor" : "") + '" style="background:' +
            heatColor(sp) + '" title="' + esc(tipTxt) + '">' + esc(label) + "</td>";
        });
        body += "</tr>";
      });

      out += '<figure class="panel"><div class="scroll"><table class="heat">' +
        '<caption class="sr-only">' +
        esc(d.id + " read speedup against CityJSONSeq") + "</caption><thead>" + head +
        "</thead><tbody>" + body + "</tbody></table></div>" +
        '<p class="note">' + esc(d.subtitle) + "</p></figure>";
    });
    el("heat-grid").innerHTML = out;

    el("heat-lede").innerHTML =
      "Cell = how many times faster than CityJSONSeq that format is on that scenario " +
      "(1 / time ratio). Green is faster, red is slower, in log₂ steps centred " +
      "on 1× — but the number is printed in every cell, so colour is never the only " +
      'channel. "≈" marks a difference inside the ' + (FLOOR * 1000).toFixed(0) +
      " ms citation floor; – is a scenario that was not benchmarked or has no " +
      "baseline. Columns: " + FORMATS.map(function (f) {
        return "<b>" + esc(ABBR[f]) + "</b> " + esc(SHORT[f]);
      }).join(" &middot; ") + ".";

    /* exact-value companion tables */
    var det = "";
    DATASETS.forEach(function (d) {
      var rows = "", n = 0;
      SCEN_ORDER.forEach(function (sc) {
        FORMATS.forEach(function (f) {
          var r = rec(d.id, sc, f);
          if (!r) { return; }
          n += 1;
          rows += '<tr' + (n % 6 === 1 ? ' class="sep"' : "") + "><td>" + esc(sc) +
            (GRAIN[sc] === false ? " †" : "") + "</td><td>" + esc(ABBR[f]) +
            '</td><td class="n">' + esc(secs(r.time_s)) +
            '</td><td class="n">' + esc(secs(r.time_mad_s)) +
            '</td><td class="n">' + esc(ratio(r.time_ratio)) +
            '</td><td class="n">' + esc(bytes(r.heap_b)) +
            '</td><td class="n">' + esc(bytes(r.rss_b)) +
            '</td><td class="n">' + esc(ratio(r.rss_ratio)) +
            '</td><td class="n">' + esc(num(r.result_count)) +
            "</td><td>" + (r.below_floor ? "inside floor" : "") + "</td></tr>";
        });
      });
      det += "<details><summary>Exact values — " + esc(d.id) + " (" + n +
        ' measurements)</summary><div class="scroll"><table class="data"><thead><tr>' +
        "<th>scenario</th><th>format</th><th>time</th><th>MAD</th><th>time ratio</th>" +
        "<th>peak heap</th><th>peak RSS</th><th>RSS ratio</th><th>results</th><th></th>" +
        "</tr></thead><tbody>" + rows + "</tbody></table></div></details>";
    });
    el("heat-details").innerHTML = det;

    el("view-heat").setAttribute("aria-label",
      "Read speedup heatmap. CityParquet is one to two orders of magnitude faster than " +
      "CityJSONSeq on count, bounding-box, attribute and projection scenarios, and around " +
      "parity on full-read; the table cells carry the numbers as text.");
  }

  /* =========================================================
     VIEW 3 — on-disk size
     ========================================================= */

  function renderSizes() {
    /* A LOG axis, with the bars growing out of the 1x baseline rather than out
       of zero. The format axis spans CityParquet at ~0.3x and CityGML at up to
       ~25x of the same bytes, and on a linear 0-to-max scale the CityParquet
       series — the subject of the view — collapses against the left edge. */
    var lo = Infinity, hi = 0;
    DATA.sizes.forEach(function (s) {
      if (s.frac_of_baseline == null) { return; }
      if (s.frac_of_baseline < lo) { lo = s.frac_of_baseline; }
      if (s.frac_of_baseline > hi) { hi = s.frac_of_baseline; }
    });
    lo = Math.min(lo / 1.6, 0.5);
    hi = Math.max(hi * 1.6, 2);
    var W = 280, ML = 52, MR = 40, rowH = 20, MT = 6;
    var pw = W - ML - MR;
    var sx = logScale(lo, hi, ML, ML + pw);

    var out = "";
    DATASETS.forEach(function (d, di) {
      var rows = SIZE_FORMATS.map(function (f) {
        return SIZES[d.id] && SIZES[d.id][f] ? SIZES[d.id][f] : null;
      }).filter(function (r) { return r && r.frac_of_baseline != null; });
      rows.sort(function (a, b) { return a.frac_of_baseline - b.frac_of_baseline; });

      var H = MT + rows.length * rowH + 16;
      var svg = "";
      svg += '<line class="refline" x1="' + sx(1).toFixed(1) + '" y1="' + MT +
        '" x2="' + sx(1).toFixed(1) + '" y2="' + (MT + rows.length * rowH) + '"/>';
      rows.forEach(function (r, i) {
        var y = MT + i * rowH + 4, bh = 10;
        var cls = r.format === "cityparquet" ? "bar accent"
          : (r.format === "cityparquet-hilbert" ? "bar accent-open" : "bar");
        var x2 = sx(r.frac_of_baseline), x1 = sx(1);
        var bx = Math.min(x1, x2), bw = Math.max(0.8, Math.abs(x2 - x1));
        var smaller = r.frac_of_baseline < 1;
        var tipTxt = SHORT[r.format] + "\n" + d.id + "\n" + bytes(r.bytes) + " = " +
          ratio(r.frac_of_baseline) + " of the CityJSONSeq baseline (" +
          bytes(SIZES[d.id].cityjsonseq && SIZES[d.id].cityjsonseq.bytes) + ")";
        svg += '<g class="pt" tabindex="0" role="img" aria-label="' +
          esc(SHORT[r.format] + " " + ratio(r.frac_of_baseline) + " of baseline, " +
              bytes(r.bytes)) + '" data-tip="' + esc(tipTxt) + '">' +
          '<rect class="' + cls + '" x="' + bx.toFixed(1) + '" y="' + y + '" width="' +
          bw.toFixed(1) + '" height="' + bh + '"/>' +
          '<text class="tick" x="' + (ML - 4) + '" y="' + (y + bh - 1.5) +
          '" text-anchor="end">' + esc(ABBR[r.format]) + "</text>" +
          '<text class="tick" x="' + (smaller ? bx - 4 : x2 + 4).toFixed(1) + '" y="' +
          (y + bh - 1.5) + '"' + (smaller ? ' text-anchor="end"' : "") + ">" +
          esc(ratio(r.frac_of_baseline)) + "</text></g>";
      });
      svg += '<line class="axis" x1="' + ML + '" y1="' + (MT + rows.length * rowH) +
        '" x2="' + (ML + pw) + '" y2="' + (MT + rows.length * rowH) + '"/>';
      logTicks(lo, hi).forEach(function (t) {
        svg += '<text class="tick" x="' + sx(t).toFixed(1) + '" y="' +
          (MT + rows.length * rowH + 11) + '" text-anchor="middle">' +
          esc(t === 1 && di === 0 ? "1× CityJSONSeq" : tickText(t)) + "</text>";
      });

      out += '<figure class="panel"><figcaption><span class="name">' + esc(d.id) +
        '</span><span class="sub">' + esc(d.subtitle) + "</span></figcaption>" +
        '<svg viewBox="0 0 ' + W + " " + H + '" role="img" aria-label="' +
        esc(d.id + " on-disk size as a fraction of CityJSONSeq: " +
            rows.map(function (r) {
              return SHORT[r.format] + " " + ratio(r.frac_of_baseline);
            }).join(", ")) + '">' + svg + "</svg></figure>";
    });
    el("size-grid").innerHTML = out;

    el("size-lede").innerHTML =
      "Bars are file size divided by the CityJSONSeq file size for the same dataset, sorted " +
      "smallest first, all panels on one shared scale. " +
      SIZE_FORMATS.filter(function (f) { return MEASURED_SIZES[f]; }).length +
      " formats — DuckDB is absent because it reads the CityParquet artefact rather than " +
      "writing one of its own. " +
      "CityParquet is accented, Hilbert-ordered CityParquet is the outlined bar.";

    el("view-size").setAttribute("aria-label",
      "On-disk size grid. CityParquet stores every dataset in roughly 0.2 to 0.4 times the " +
      "bytes of CityJSONSeq, while FlatCityBuf stays close to 1 times.");
  }

  /* =========================================================
     VIEW 4 — compression variants (de-emphasized)
     ========================================================= */

  /* short codes, as used by the static figure — long names cannot be placed
     inside the dense 1×,1× corner without colliding */
  var VARIANT_CODE = {
    "cityparquet": "def", "cityparquet+gzip": "gzip", "cityparquet+brotli": "brot",
    "cityparquet+lz4": "lz4", "cityparquet+snappy": "snap",
    "cityparquet+uncompressed": "none", "cityparquet+rg512": "rg512",
    "cityparquet+rg4096": "rg4k"
  };
  function variantLabel(v) {
    return v.replace("cityparquet+", "").replace("cityparquet", "default");
  }
  function variantCode(v) { return VARIANT_CODE[v] || variantLabel(v); }

  function renderCompression() {
    el("codec-note").innerHTML = '<p class="small"><b>Read this before the panels.</b> ' +
      esc(META.codec_level_note) + "</p>";

    /* A corpus measured for reads but never for compression is normal: the
       compression benchmark is a separate, much slower pass. Say so where the
       panels would have been — an empty section reads like a rendering bug. */
    if (!DATA.compression.length) {
      el("comp-grid").innerHTML = "";
      el("comp-lede").innerHTML =
        "<p><b>No compression run for this corpus.</b> The codec and row-group " +
        "variants are measured by a separate pass over the same inputs " +
        "(<span class='num'>just compression-bench</span>), and " +
        "<span class='num'>bench/compression_results</span> is empty for the run " +
        "reported here. Nothing on this page is derived from an earlier corpus's " +
        "compression numbers.</p>";
      el("comp-notes").innerHTML = "";
      el("roundtrip-strip").innerHTML = "";
      el("view-comp").setAttribute("aria-label",
        "Compression variants: not measured for this corpus.");
      return;
    }

    var xs = [1], ys = [1];
    DATA.compression.forEach(function (c) {
      if (c.write_ratio > 0) { xs.push(c.write_ratio); }
      if (c.size_ratio > 0) { ys.push(c.size_ratio); }
    });
    var xmin = Math.min.apply(null, xs) * 0.85, xmax = Math.max.apply(null, xs) * 1.2;
    var ymin = Math.min.apply(null, ys) * 0.92, ymax = Math.max.apply(null, ys) * 1.3;

    var W = 230, H = 175, ML = 32, MR = 26, MT = 8, MB = 26;
    var pw = W - ML - MR, ph = H - MT - MB;
    var sx = logScale(xmin, xmax, ML, ML + pw);
    var sy = logScale(ymin, ymax, MT + ph, MT);

    var gapsBy = {};
    DATA.compression_gaps.forEach(function (g) { gapsBy[g.dataset] = g.issue; });

    var out = "";
    COMP_DATASETS.forEach(function (id, di) {
      var recs = COMP[id];
      var failed = recs.every(function (c) { return c.roundtrip === false; });
      var svg = "";

      svg += '<line class="refline" x1="' + sx(1).toFixed(1) + '" y1="' + MT + '" x2="' +
        sx(1).toFixed(1) + '" y2="' + (MT + ph) + '"/>';
      svg += '<line class="refline" x1="' + ML + '" y1="' + sy(1).toFixed(1) + '" x2="' +
        (ML + pw) + '" y2="' + sy(1).toFixed(1) + '"/>';
      var tickBoxes = [];
      logTicks(xmin, xmax).forEach(function (t) {
        var tw = tickText(t).length * 4.8;
        svg += '<text class="tick" x="' + sx(t).toFixed(1) + '" y="' + (MT + ph + 10) +
          '" text-anchor="middle">' + esc(tickText(t)) + "</text>";
        tickBoxes.push({ x0: sx(t) - tw / 2 - 2, y0: MT + ph + 2,
                         x1: sx(t) + tw / 2 + 2, y1: MT + ph + 12 });
      });
      logTicks(ymin, ymax).forEach(function (t) {
        var tw = tickText(t).length * 4.8;
        svg += '<text class="tick" x="' + (ML - 4) + '" y="' + (sy(t) + 3).toFixed(1) +
          '" text-anchor="end">' + esc(tickText(t)) + "</text>";
        tickBoxes.push({ x0: ML - 4 - tw - 2, y0: sy(t) - 5,
                         x1: ML - 2, y1: sy(t) + 5 });
      });
      svg += '<line class="axis" x1="' + ML + '" y1="' + (MT + ph) + '" x2="' + (ML + pw) +
        '" y2="' + (MT + ph) + '"/>';
      svg += '<line class="axis" x1="' + ML + '" y1="' + MT + '" x2="' + ML + '" y2="' +
        (MT + ph) + '"/>';

      var labels = [];
      recs.forEach(function (c) {
        if (c.write_ratio == null || c.size_ratio == null) { return; }
        var x = sx(c.write_ratio), y = sy(c.size_ratio);
        var lbl = variantLabel(c.variant);
        labels.push({ px: x, py: y, text: variantCode(c.variant) });
        var tipTxt = lbl + " · " + id + "\nwrite " + ratio(c.write_ratio) + " (" +
          secs(c.write_s) + ")\nsize " + ratio(c.size_ratio) + " (" + bytes(c.total_bytes) +
          ")\nfull scan " + secs(c.full_scan_s) + " · window " + secs(c.window_query_s) +
          "\nround-trip " + (c.roundtrip ? "equal" : "FAILED") +
          "\nkind: " + c.kind + (c.kind === "codec"
            ? " (mismatched levels — not a codec ranking)"
            : (c.kind === "rowgroup" ? " (row-group size, not a codec)" : ""));
        var shape;
        if (c.kind === "default") {
          shape = '<path class="mk-line mk-gray" d="M' + (x - 3.5).toFixed(1) + " " +
            (y - 3.5).toFixed(1) + "L" + (x + 3.5).toFixed(1) + " " + (y + 3.5).toFixed(1) +
            "M" + (x - 3.5).toFixed(1) + " " + (y + 3.5).toFixed(1) + "L" +
            (x + 3.5).toFixed(1) + " " + (y - 3.5).toFixed(1) + '"/>';
        } else if (c.kind === "codec") {
          shape = '<circle cx="' + x.toFixed(1) + '" cy="' + y.toFixed(1) +
            '" r="3" class="mk-gray"/>';
        } else {
          shape = '<circle cx="' + x.toFixed(1) + '" cy="' + y.toFixed(1) +
            '" r="3" class="mk-gray-open"/>';
        }
        svg += '<g class="pt" tabindex="0" role="img" aria-label="' +
          esc(tipTxt.replace(/\n/g, ". ")) + '" data-tip="' + esc(tipTxt) + '">' +
          '<circle class="halo" cx="' + x.toFixed(1) + '" cy="' + y.toFixed(1) +
          '" r="6" fill="none" stroke="none"/>' +
          '<circle cx="' + x.toFixed(1) + '" cy="' + y.toFixed(1) +
          '" r="6" fill="transparent"/>' + shape + "</g>";
      });

      /* direct labels: dodged around the markers, kept clear of the tick row */
      svg += placeLabels(labels,
        { x0: 2, x1: W - 2, y0: MT + 6, y1: MT + ph + 1 }, "tick", tickBoxes);

      if (di === 0) {
        svg += '<text class="axlabel" x="' + (ML + pw) + '" y="' + (MT + ph + 21) +
          '" text-anchor="end">write time ratio (log)</text>';
        svg += '<text class="axlabel" transform="translate(' + (ML - 21) + ',' + (MT + ph) +
          ') rotate(-90)">size ratio (log)</text>';
      }

      out += '<figure class="panel' + (failed ? " dim" : "") +
        '"><figcaption><span class="name">' + esc(id) + "</span>" +
        (failed ? ' <span class="badge">roundtrip FAILED — not citable</span>' : "") +
        '<span class="sub">' + esc(recs.length + " variants vs the default CityParquet recipe") +
        "</span></figcaption>" +
        '<svg viewBox="0 0 ' + W + " " + H + '" role="img" aria-label="' +
        esc(id + " compression variants: write time and size against the default recipe" +
            (failed ? " — every variant failed its round-trip check" : "")) + '">' +
        svg + "</svg>" +
        (gapsBy[id] ? '<p class="note">' + esc(gapsBy[id]) + "</p>" : "") + "</figure>";
    });
    el("comp-grid").innerHTML = out;

    el("comp-lede").innerHTML =
      COMP_DATASETS.length + " of the " + DATASETS.length + " datasets have a " +
      "compression run. Each point is one writer variant " +
      "measured against that dataset's own default CityParquet recipe at 1×, 1× " +
      "(the cross): horizontal is write time, vertical is total bytes, both logarithmic. " +
      "Filled circles are codec variants, open circles are row-group-size variants — a " +
      "different axis of variation shown on the same plot; the whole view is drawn in the " +
      "neutral grey series, because nothing in it is a citable ranking. Points are " +
      "labelled directly with short codes — <b>def</b> the default recipe, <b>none</b> " +
      "uncompressed, <b>brot</b> brotli, <b>snap</b> snappy, <b>gzip</b>, <b>lz4</b>, and " +
      "<b>rg512</b>/<b>rg4k</b> the two row-group sizes. Where the 1×, 1× cluster leaves no " +
      "room for a label without overlapping another, the label is omitted rather than " +
      "overprinted; hover or tab to the point for its name and numbers.";

    var missing = DATASETS.map(function (d) { return d.id; })
      .filter(function (id) { return !COMP[id]; });
    el("comp-notes").innerHTML =
      '<div class="callout"><p class="small"><b>The three datasets without a panel.</b> ' +
      missing.map(function (id) {
        return "<b>" + esc(id) + "</b> — " +
          esc(gapsBy[id] || "no compression run in this corpus");
      }).join("; ") +
      ". They are named here rather than dropped silently.</p></div>";

    var strip = COMP_DATASETS.map(function (id) {
      var recs = COMP[id];
      var ok = recs.filter(function (c) { return c.roundtrip; }).length;
      var mark = ok === recs.length ? "✓" : "✗";
      return esc(id) + "&nbsp;" + mark + "&nbsp;<span class='sub'>(" + ok + "/" +
        recs.length + ")</span>";
    }).join(" &middot; ");
    el("roundtrip-strip").innerHTML =
      "Every variant is re-read and compared against the source; ✓ means all variants of " +
      "that dataset round-tripped equal. " + strip + ". " +
      "Where a dataset shows ✗ for every variant, nothing in its panel is citable — the " +
      "written bytes did not read back equal to the source.";

    el("view-comp").setAttribute("aria-label",
      "Compression and row-group variants, de-emphasised. Codec choice moves size by at most " +
      "a factor of two at mismatched compression levels, so no codec ranking is citable from " +
      "this benchmark.");
  }

  /* =========================================================
     Caveats + coverage
     ========================================================= */

  /* ---- 1 - the corpus table --------------------------------------------
     A table, not a chart: the question is "which datasets, and how big",
     which is a lookup rather than a pattern, and a reader wants the exact
     bytes. Sizes are per format so the encoding comparison is visible in
     the same row as the shape of the dataset it applies to. */
  var TABLE_FORMATS = ["citygml", "cityjson", "cityjsonseq", "flatcitybuf",
                       "cityparquet-hilbert"];

  function renderCorpus() {
    var present = TABLE_FORMATS.filter(function (f) {
      return DATASETS.some(function (d) { return SIZES[d.id] && SIZES[d.id][f]; });
    });
    var head = '<tr><th scope="col">Dataset</th><th scope="col">CityObjects</th>';
    present.forEach(function (f) {
      head += '<th scope="col">' + esc(SHORT[f]) + "</th>";
    });
    head += '<th scope="col">Factor</th></tr>';

    var body = "";
    DATASETS.forEach(function (d) {
      var row = '<tr><th scope="row">' + esc(d.id) + "</th><td>" + num(d.objects) + "</td>";
      var seq = SIZES[d.id] && SIZES[d.id].cityjsonseq;
      var cpq = SIZES[d.id] && (SIZES[d.id]["cityparquet-hilbert"] || SIZES[d.id].cityparquet);
      present.forEach(function (f) {
        var s = SIZES[d.id] && SIZES[d.id][f];
        row += "<td>" + (s ? esc(bytes(s.bytes)) : "&mdash;") + "</td>";
      });
      var factor = (seq && cpq && cpq.bytes) ? seq.bytes / cpq.bytes : null;
      row += "<td>" + (factor ? esc(ratio(factor)) : "&mdash;") + "</td></tr>";
      body += row;
    });

    el("corpus-table").innerHTML =
      '<div class="tablewrap"><table class="corpus"><caption>' +
      esc(DATASETS.length + " datasets, ordered by CityObject count. Factor is " +
          "CityJSONSeq bytes divided by CityParquet bytes; a dash is a format " +
          "this dataset has no conversion for.") +
      "</caption><thead>" + head + "</thead><tbody>" + body + "</tbody></table></div>";
    el("corpus-lede").textContent =
      "Every dataset the read benchmark measured, with its size in each format. " +
      "This is the lookup the rest of the page refers back to.";
  }

  /* ---- 2 - read time and peak memory, per dataset -----------------------
     Bars grow out of the 1x rule on a LOG axis, not out of zero on a linear
     one: within one scenario the formats span several orders of magnitude,
     and a zero-anchored bar renders everything but the fastest as a sliver. */
  function pairedPanel(d, sc, tlo, thi, mlo, mhi) {
    var fmts = FORMATS.filter(function (f) { return f !== META.baseline; });
    var W = 300, ML = 56, GAP = 12, rowH = 15, MT = 4;
    var half = (W - ML - GAP) / 2;
    var sx = logScale(tlo, thi, ML, ML + half);
    var mx = logScale(mlo, mhi, ML + half + GAP, W);
    var H = MT + fmts.length * rowH + 18;
    var svg = "";
    [[sx, tlo, thi], [mx, mlo, mhi]].forEach(function (a) {
      svg += '<line class="refline" x1="' + a[0](1).toFixed(1) + '" y1="' + MT +
        '" x2="' + a[0](1).toFixed(1) + '" y2="' + (MT + fmts.length * rowH) + '"/>';
    });
    var any = false;
    fmts.forEach(function (f, i) {
      var r = rec(d.id, sc, f);
      var y = MT + i * rowH + 3, bh = 9;
      svg += '<text class="tick" x="' + (ML - 4) + '" y="' + (y + bh - 1.5) +
        '" text-anchor="end">' + esc(ABBR[f]) + "</text>";
      if (!r) { return; }
      any = true;
      [[sx, r.time_ratio, "time", r.below_floor], [mx, r.rss_ratio, "rss", false]]
        .forEach(function (m) {
          if (m[1] == null || m[1] <= 0) { return; }
          var v = 1 / m[1];
          var x2 = m[0](v), x1 = m[0](1);
          var bx = Math.min(x1, x2), bw = Math.max(0.8, Math.abs(x2 - x1));
          var cls = "bar" + (isAccent(f) ? " accent" : "") + (m[3] ? " muted" : "");
          var tipTxt = SHORT[f] + "\n" + d.id + " - " + sc + "\n" +
            (m[2] === "time"
              ? secs(r.time_s) + " = " + ratio(v) + " the baseline's speed"
              : bytes(r.rss_b) + " peak RSS = " + ratio(v) + " leaner") +
            (m[3] ? "\nwithin the 10 ms citation floor" : "");
          svg += '<g class="pt" tabindex="0" role="img" aria-label="' +
            esc(SHORT[f] + " " + ratio(v)) + '" data-tip="' + esc(tipTxt) + '">' +
            '<rect class="' + cls + '" x="' + bx.toFixed(1) + '" y="' + y +
            '" width="' + bw.toFixed(1) + '" height="' + bh + '"/></g>';
        });
    });
    if (!any) {
      return '<figure class="panel"><figcaption><span class="name">' + esc(d.id) +
        '</span><span class="sub">' + esc(d.subtitle) + "</span></figcaption>" +
        '<p class="small">not measured in this run</p></figure>';
    }
    var ay = MT + fmts.length * rowH;
    [[sx, tlo, thi, ML, ML + half], [mx, mlo, mhi, ML + half + GAP, W]]
      .forEach(function (a) {
        svg += '<line class="axis" x1="' + a[3] + '" y1="' + ay + '" x2="' + a[4] +
          '" y2="' + ay + '"/>';
        logTicks(a[1], a[2]).forEach(function (t) {
          svg += '<text class="tick" x="' + a[0](t).toFixed(1) + '" y="' + (ay + 10) +
            '" text-anchor="middle">' + esc(tickText(t)) + "</text>";
        });
      });
    svg += '<text class="tick" x="' + (ML + half / 2) + '" y="' + (ay + 18) +
      '" text-anchor="middle">time (x faster)</text>';
    svg += '<text class="tick" x="' + (ML + half + GAP + half / 2) + '" y="' + (ay + 18) +
      '" text-anchor="middle">memory (x leaner)</text>';
    return '<figure class="panel"><figcaption><span class="name">' + esc(d.id) +
      '</span><span class="sub">' + esc(d.subtitle) + "</span></figcaption>" +
      '<svg viewBox="0 0 ' + W + " " + H + '" role="img" aria-label="' +
      esc(d.id + " read time and peak memory by format") + '">' + svg + "</svg></figure>";
  }

  function renderFormats() {
    var sel = el("formats-scen");
    if (!sel.options.length) {
      var present = [];
      DATA.read.forEach(function (r) {
        if (present.indexOf(r.scenario_key) < 0) { present.push(r.scenario_key); }
      });
      present.sort(function (a, b) { return SCEN_ORDER.indexOf(a) - SCEN_ORDER.indexOf(b); });
      present.forEach(function (sc) {
        var o = document.createElement("option");
        o.value = sc; o.textContent = sc + (dagger(sc) ? " (grain-incomparable)" : "");
        sel.appendChild(o);
      });
      sel.value = present.indexOf("full-read") >= 0 ? "full-read" : present[0];
      sel.addEventListener("change", renderFormats);
    }
    var sc = sel.value;
    var tlo = Infinity, thi = 0, mlo = Infinity, mhi = 0;
    DATA.read.forEach(function (r) {
      if (r.scenario_key !== sc || r.format === META.baseline) { return; }
      if (r.time_ratio) { var t = 1 / r.time_ratio; tlo = Math.min(tlo, t); thi = Math.max(thi, t); }
      if (r.rss_ratio) { var m = 1 / r.rss_ratio; mlo = Math.min(mlo, m); mhi = Math.max(mhi, m); }
    });
    if (!isFinite(tlo)) { tlo = 0.5; thi = 2; }
    if (!isFinite(mlo)) { mlo = 0.5; mhi = 2; }
    tlo = Math.min(tlo / 1.6, 0.5); thi = Math.max(thi * 1.6, 2);
    mlo = Math.min(mlo / 1.6, 0.5); mhi = Math.max(mhi * 1.6, 2);

    el("formats-grid").innerHTML = DATASETS.map(function (d) {
      return pairedPanel(d, sc, tlo, thi, mlo, mhi);
    }).join("");
    el("formats-scen-note").textContent = dagger(sc)
      ? "grain-incomparable - see fairness caveat 1" : "";
    el("formats-lede").textContent =
      "One panel per dataset: read time on the left, peak memory on the right, " +
      "both against the CityJSONSeq artefact for the same dataset and scenario. " +
      "Bars grow out of the 1x rule on a shared log scale; right is better in both. " +
      "A muted bar is inside the 10 ms citation floor.";
  }

  /* ---- 3 - configuration axes ------------------------------------------ */
  function renderConfigOrdering() {
    var rows = DATA.ordering || [];
    if (!rows.length) {
      el("config-order-fig").innerHTML =
        '<p class="small">No ordering run in this corpus - `just ordering-bench` produces it.</p>';
      return;
    }
    var sel = el("config-order-scen");
    var keys = [];
    rows.forEach(function (r) { if (keys.indexOf(r.scenario_key) < 0) { keys.push(r.scenario_key); } });
    keys.sort(function (a, b) { return SCEN_ORDER.indexOf(a) - SCEN_ORDER.indexOf(b); });
    if (!sel.options.length) {
      keys.forEach(function (k) {
        var o = document.createElement("option");
        o.value = k; o.textContent = k; sel.appendChild(o);
      });
      sel.value = keys.indexOf("bbox-5pct") >= 0 ? "bbox-5pct" : keys[0];
      sel.addEventListener("change", renderConfigOrdering);
    }
    var sc = sel.value;
    var mine = rows.filter(function (r) { return r.scenario_key === sc && r.time_ratio; });
    mine.sort(function (a, b) { return b.time_ratio - a.time_ratio; });
    var lo = Infinity, hi = 0;
    mine.forEach(function (r) { lo = Math.min(lo, r.time_ratio); hi = Math.max(hi, r.time_ratio); });
    lo = Math.min(lo / 1.6, 0.5); hi = Math.max(hi * 1.6, 2);

    var W = 620, ML = 210, rowH = 14, MT = 6;
    var sx = logScale(lo, hi, ML, W - 46);
    var H = MT + mine.length * rowH + 20;
    var svg = '<line class="refline" x1="' + sx(1).toFixed(1) + '" y1="' + MT +
      '" x2="' + sx(1).toFixed(1) + '" y2="' + (MT + mine.length * rowH) + '"/>';
    mine.forEach(function (r, i) {
      var y = MT + i * rowH + 3, bh = 8;
      var x2 = sx(r.time_ratio), x1 = sx(1);
      var bx = Math.min(x1, x2), bw = Math.max(0.8, Math.abs(x2 - x1));
      svg += '<g class="pt" tabindex="0" role="img" aria-label="' +
        esc(r.dataset + " " + ratio(r.time_ratio)) + '" data-tip="' +
        esc(r.dataset + "\n" + sc + "\nsource order " + secs(r.base_time_s) +
            " -> Hilbert " + secs(r.variant_time_s) + " = " + ratio(r.time_ratio) +
            (r.below_floor ? "\nwithin the 10 ms citation floor" : "")) + '">' +
        '<rect class="bar accent' + (r.below_floor ? " muted" : "") + '" x="' +
        bx.toFixed(1) + '" y="' + y + '" width="' + bw.toFixed(1) + '" height="' + bh + '"/>' +
        '<text class="tick" x="' + (ML - 4) + '" y="' + (y + bh - 1) +
        '" text-anchor="end">' + esc(r.dataset) + "</text></g>";
    });
    var ay = MT + mine.length * rowH;
    svg += '<line class="axis" x1="' + ML + '" y1="' + ay + '" x2="' + (W - 46) +
      '" y2="' + ay + '"/>';
    logTicks(lo, hi).forEach(function (t) {
      svg += '<text class="tick" x="' + sx(t).toFixed(1) + '" y="' + (ay + 10) +
        '" text-anchor="middle">' + esc(tickText(t)) + "</text>";
    });
    svg += '<text class="tick" x="' + ((ML + W - 46) / 2) + '" y="' + (ay + 18) +
      '" text-anchor="middle">Hilbert order vs source order (x faster)</text>';
    var cleared = mine.filter(function (r) { return !r.below_floor; }).length;
    el("config-order-fig").innerHTML =
      '<svg viewBox="0 0 ' + W + " " + H + '" role="img" aria-label="' +
      esc("Hilbert ordering speed-up by dataset for " + sc) + '">' + svg + "</svg>";
    el("config-order-lede").textContent =
      "The Hilbert-ordered package against the same package written in source order - " +
      "same writer, same reader, same scenarios. " + cleared + " of " + mine.length +
      " datasets clear the 10 ms citation floor on this scenario; the muted bars do not, " +
      "and their position is noise.";
  }

  function renderConfigVariants() {
    var recs = (DATA.scaling && DATA.scaling.compression) || [];
    if (!recs.length) {
      el("config-var-table").innerHTML =
        '<p class="small">No codec or row-group run in this corpus - ' +
        '`just compression-bench` over the scaling corpus produces it.</p>';
      return;
    }
    var slices = [];
    recs.forEach(function (r) {
      if (!slices.some(function (s) { return s.id === r.dataset; })) {
        slices.push({ id: r.dataset, objects: r.objects });
      }
    });
    slices.sort(function (a, b) { return a.objects - b.objects; });
    var variants = [];
    recs.forEach(function (r) { if (variants.indexOf(r.variant) < 0) { variants.push(r.variant); } });

    var head = '<tr><th scope="col">Variant</th><th scope="col">Axis</th>';
    slices.forEach(function (s) {
      head += '<th scope="col">' + num(s.objects) + " obj</th>";
    });
    head += '<th scope="col">Row groups touched</th></tr>';
    var body = "";
    variants.forEach(function (v) {
      var kind = "";
      var row = '<tr><th scope="row">' + esc(v.replace("cityparquet+", "").replace("cityparquet", "default")) + "</th>";
      var cells = "", touched = "";
      slices.forEach(function (s) {
        var r = recs.filter(function (x) { return x.variant === v && x.dataset === s.id; })[0];
        kind = r ? r.kind : kind;
        cells += "<td>" + (r ? esc(bytes(r.total_bytes)) : "&mdash;") + "</td>";
        if (r && r.row_groups_total) {
          touched = r.row_groups_touched + " / " + r.row_groups_total;
        }
      });
      body += row + "<td>" + esc(kind) + "</td>" + cells + "<td>" + esc(touched || "&mdash;") + "</td></tr>";
    });
    el("config-var-table").innerHTML =
      '<div class="tablewrap"><table class="corpus"><caption>' +
      esc("Bytes written by each writer configuration, at four cardinalities of the " +
          "same city model. Row groups touched is for the widest spatial window at the " +
          "largest slice - it is what row-group size actually buys.") +
      "</caption><thead>" + head + "</thead><tbody>" + body + "</tbody></table></div>";
    el("config-var-lede").textContent =
      "Codec and row-group size are write-side axes: the harness reports bytes, write " +
      "time and row-group counts for them, but no peak RSS and only two query types, so " +
      "they cannot be drawn in the shape used above.";
  }

  /* ---- 4 - scaling trend ------------------------------------------------
     The one view on this page that plots ABSOLUTE seconds and bytes rather
     than a ratio to CityJSONSeq. That is deliberate: the quantity being read
     is the SLOPE of the line on log-log axes - flat means the cost does not
     grow with the model, and a ratio to a baseline that is itself growing
     hides exactly that. */
  function scalingPanel(metric, sc, slices) {
    var recs = (DATA.scaling && DATA.scaling.read) || [];
    var fmts = [];
    recs.forEach(function (r) { if (fmts.indexOf(r.format) < 0) { fmts.push(r.format); } });
    fmts.sort(function (a, b) { return FORMATS.indexOf(a) - FORMATS.indexOf(b); });

    var key = metric === "time" ? "time_s" : "rss_b";
    var xlo = Infinity, xhi = 0, ylo = Infinity, yhi = 0;
    recs.forEach(function (r) {
      if (r.scenario_key !== sc || r[key] == null || r[key] <= 0) { return; }
      xlo = Math.min(xlo, r.objects); xhi = Math.max(xhi, r.objects);
      ylo = Math.min(ylo, r[key]); yhi = Math.max(yhi, r[key]);
    });
    if (!isFinite(xlo) || !isFinite(ylo)) {
      return '<figure class="panel"><figcaption><span class="name">' +
        esc(metric === "time" ? "read time" : "peak memory") +
        '</span></figcaption><p class="small">not measured in this run</p></figure>';
    }
    var W = 300, H = 210, ML = 48, MR = 62, MT = 8, MB = 30;
    var sx = logScale(xlo / 1.3, xhi * 1.3, ML, W - MR);
    var sy = logScale(ylo / 1.6, yhi * 1.6, H - MB, MT);
    var svg = "";

    if (metric === "time") {
      /* Honesty rule 1, carried into an absolute-value view: anything under
         the 10 ms floor is a timing this benchmark will not cite. */
      var floor = DATA.meta.citation_floor_s;
      if (floor > ylo / 1.6) {
        var fy = Math.min(H - MB, sy(floor));
        svg += '<rect class="floorband" x="' + ML + '" y="' + fy.toFixed(1) +
          '" width="' + (W - MR - ML) + '" height="' + (H - MB - fy).toFixed(1) + '"/>';
        svg += '<text class="tick" x="' + (ML + 3) + '" y="' + (fy - 2).toFixed(1) +
          '">10 ms citation floor</text>';
      }
    }
    var ends = [];
    fmts.forEach(function (f) {
      var pts = recs.filter(function (r) {
        return r.format === f && r.scenario_key === sc && r[key] != null && r[key] > 0;
      }).sort(function (a, b) { return a.objects - b.objects; });
      if (!pts.length) { return; }
      var dstr = pts.map(function (p, i) {
        return (i ? "L" : "M") + sx(p.objects).toFixed(1) + " " + sy(p[key]).toFixed(1);
      }).join(" ");
      svg += '<path class="line' + (isAccent(f) ? " accent" : "") + '" d="' + dstr + '"/>';
      pts.forEach(function (p) {
        svg += point(f, sx(p.objects), sy(p[key]), 3,
          SHORT[f] + "\n" + num(p.objects) + " CityObjects, " + sc + "\n" +
          (metric === "time" ? secs(p[key]) : bytes(p[key])));
      });
      var last = pts[pts.length - 1];
      ends.push({ x: sx(last.objects) + 4, y: sy(last[key]), t: ABBR[f], f: f });
    });
    placeEndLabels(ends, { top: MT, bottom: H - MB });
    ends.forEach(function (e) {
      svg += '<text class="endlab' + (isAccent(e.f) ? " accent" : "") + '" x="' +
        e.x.toFixed(1) + '" y="' + e.y.toFixed(1) + '">' + esc(e.t) + "</text>";
    });
    svg += '<line class="axis" x1="' + ML + '" y1="' + (H - MB) + '" x2="' + (W - MR) +
      '" y2="' + (H - MB) + '"/>';
    slices.forEach(function (s) {
      svg += '<text class="tick" x="' + sx(s.objects).toFixed(1) + '" y="' + (H - MB + 10) +
        '" text-anchor="middle">' + esc(num(s.objects)) + "</text>";
    });
    logTicks(ylo / 1.6, yhi * 1.6).forEach(function (t) {
      svg += '<text class="tick" x="' + (ML - 4) + '" y="' + (sy(t) + 3).toFixed(1) +
        '" text-anchor="end">' +
        esc(metric === "time" ? secs(t) : bytes(t)) + "</text>";
    });
    svg += '<text class="tick" x="' + ((ML + W - MR) / 2) + '" y="' + (H - MB + 22) +
      '" text-anchor="middle">CityObjects (log)</text>';
    return '<figure class="panel"><figcaption><span class="name">' +
      esc(metric === "time" ? "read time" : "peak memory") +
      '</span><span class="sub">' + esc("absolute " + (metric === "time" ? "seconds" : "bytes") +
      ", log-log - a flat line is a cost that does not grow") + "</span></figcaption>" +
      '<svg viewBox="0 0 ' + W + " " + H + '" role="img" aria-label="' +
      esc((metric === "time" ? "Read time" : "Peak memory") +
          " against CityObject count by format for " + sc) + '">' + svg + "</svg></figure>";
  }

  function renderScaling() {
    var recs = (DATA.scaling && DATA.scaling.read) || [];
    if (!recs.length) {
      el("scaling-grid").innerHTML =
        '<p class="small">No scaling corpus in this run - `just fetch-scaling-data` ' +
        'builds it and the bench recipes measure it into bench/scaling_*_results.</p>';
      el("scaling-lede").textContent = "";
      return;
    }
    var slices = [];
    recs.forEach(function (r) {
      if (!slices.some(function (s) { return s.id === r.dataset; })) {
        slices.push({ id: r.dataset, objects: r.objects });
      }
    });
    slices.sort(function (a, b) { return a.objects - b.objects; });

    var sel = el("scaling-scen");
    if (!sel.options.length) {
      var keys = [];
      recs.forEach(function (r) { if (keys.indexOf(r.scenario_key) < 0) { keys.push(r.scenario_key); } });
      keys.sort(function (a, b) { return SCEN_ORDER.indexOf(a) - SCEN_ORDER.indexOf(b); });
      keys.forEach(function (k) {
        var o = document.createElement("option");
        o.value = k; o.textContent = k + (dagger(k) ? " (grain-incomparable)" : "");
        sel.appendChild(o);
      });
      sel.value = keys.indexOf("full-read") >= 0 ? "full-read" : keys[0];
      sel.addEventListener("change", renderScaling);
    }
    var sc = sel.value;
    el("scaling-grid").innerHTML =
      scalingPanel("time", sc, slices) + scalingPanel("rss", sc, slices);
    el("scaling-scen-note").textContent = dagger(sc)
      ? "grain-incomparable - see fairness caveat 1" : "";
    el("scaling-lede").textContent =
      "One city model cut to " + slices.length + " cardinalities (" +
      slices.map(function (s) { return num(s.objects); }).join(", ") +
      " CityObjects), so the only thing changing along the x axis is how much data " +
      "there is. Both panels are absolute values on log-log axes rather than ratios: " +
      "the slope IS the answer, and dividing by a baseline that grows too would hide it.";
  }

  function renderCaveats() {
    el("caveats").innerHTML = META.caveats_read.map(function (c, i) {
      return '<li id="caveat-' + (i + 1) + '"><div class="verbatim">' + esc(c) + "</div></li>";
    }).join("");
    el("caveats-comp").innerHTML = META.caveats_compression.map(function (c) {
      return '<div class="verbatim">' + esc(c) + "</div>";
    }).join("");
    el("codec-note-verbatim").textContent = META.codec_level_note;
  }

  function renderCoverage() {
    var items = [];
    DATA.compression_gaps.forEach(function (g) {
      items.push("<b>" + esc(g.dataset) + "</b> — compression: " + esc(g.issue) + ".");
    });
    var noStats = DATASETS.filter(function (d) {
      return !(READ[d.id] && READ[d.id]["attr-stats"]);
    }).map(function (d) { return d.id; });
    if (noStats.length) {
      items.push("<b>attr-stats</b> is absent for " + esc(noStats.join(", ")) +
        " — a source fact of the benchmark corpus, rendered as “n/a” rather " +
        "than imputed.");
    }
    var noBaseline = [];
    DATASETS.forEach(function (d) {
      SCEN_ORDER.forEach(function (sc) {
        var byF = READ[d.id] && READ[d.id][sc];
        if (byF && !byF.cityjsonseq) {
          noBaseline.push(d.id + " / " + sc + " (only " + Object.keys(byF).map(function (f) {
            return ABBR[f];
          }).join(", ") + ")");
        }
      });
    });
    if (noBaseline.length) {
      items.push("<b>No CityJSONSeq baseline row</b>, so no ratio can be formed, for: " +
        esc(noBaseline.join("; ")) + ". The absolute times are still in the exact-value " +
        "tables above; only the ratios are undefined.");
    }
    /* A format the run never measured is one sentence, not one per (dataset,
       scenario) pair: the previous edition listed 180 pairs for a format that
       was simply absent, which buries the gaps that are actually specific. */
    FORMATS.forEach(function (f) {
      if (f === META.baseline || MEASURED[f]) { return; }
      items.push("<b>" + esc(SHORT[f]) + "</b> was not read-benchmarked in this run — no " +
        "marker, column or row in sections 0 to 2" +
        (MEASURED_SIZES[f] ? "; its on-disk size WAS measured and is in section 3" : "") +
        ".");
    });
    /* Gaps are grouped by format and by dataset. A format missing from every
       scenario of six datasets is six facts, not fifty-four rows. */
    FORMATS.forEach(function (f) {
      if (!MEASURED[f] || f === META.baseline) { return; }
      var whole = [], partial = [];
      DATASETS.forEach(function (d) {
        var covered = 0, offered = 0;
        SCEN_ORDER.forEach(function (sc) {
          var byF = READ[d.id] && READ[d.id][sc];
          if (!byF || !byF[META.baseline]) { return; }
          offered++;
          if (byF[f]) { covered++; }
        });
        if (!offered) { return; }
        if (!covered) { whole.push(d.id); }
        else if (covered < offered) { partial.push(d.id + " (" + covered + "/" + offered + ")"); }
      });
      if (whole.length) {
        items.push("<b>" + esc(SHORT[f]) + "</b> was not measured for " + whole.length +
          " of " + DATASETS.length + " datasets: " + esc(whole.join(", ")) +
          ". Their cells are “–”, never zero.");
      }
      if (partial.length) {
        items.push("<b>" + esc(SHORT[f]) + "</b> covers only some scenarios of " +
          esc(partial.join(", ")) + ".");
      }
    });
    /* Rows a run opted into that are not on the format axis. They are measured
       data and they stay in the JSON; what they are not is a format, so no view
       plots them — said once here rather than left to be noticed. */
    var offAxis = OFF_AXIS.concat(["cityparquet"]).filter(function (f) {
      return FORMATS.indexOf(f) < 0 && (MEASURED[f] || MEASURED_SIZES[f]);
    });
    if (offAxis.length) {
      items.push("<b>Measured but off the format axis</b>: " +
        esc(offAxis.map(function (f) { return SHORT[f]; }).join(", ")) +
        ". This page compares FORMATS a city model can ship as; a compression " +
        "variant of a format already here, an SQL engine reading one of them, and " +
        "the source-ordered CityParquet package (an ordering question, asked by the " +
        "separate ordering benchmark) are different questions. Their rows are in " +
        "the CSVs and in this page's data block, plotted nowhere.");
    }
    var noHeap = DATA.read.filter(function (r) { return r.heap_b == null; }).length;
    if (noHeap) {
      items.push("<b>Peak heap</b> is missing for " + num(noHeap) + " record(s), all of " +
        "them out-of-process measurements: the harness cannot see another process's " +
        "allocator. Use the peak-RSS view for any memory claim.");
    }
    items.push("<b>Sizes</b> cover " +
      SIZE_FORMATS.filter(function (f) { return MEASURED_SIZES[f]; }).length +
      " formats; DuckDB produces no artefact of its own, it reads the CityParquet package.");
    (DATA.meta.excluded_formats || []).forEach(function (e) {
      items.push("<b>" + esc(e.format) + "</b> was measured but is <b>not plotted " +
        "anywhere on this page</b> (" + num(e.rows) + " " + esc(e.where.join(" + ")) +
        " row(s)): this page has no colour, marker or caption for it yet. Its numbers " +
        "are in the result CSVs; nothing here averages them in.");
    });
    el("coverage").innerHTML = items.map(function (t) { return "<li>" + t + "</li>"; }).join("");
  }

  /* =========================================================
     Theme toggle
     ========================================================= */

  (function theme() {
    var btn = el("theme-btn");
    var modes = ["auto", "light", "dark"];
    var cur = 0;
    try {
      var saved = window.localStorage.getItem("benchviz-theme");
      if (saved && modes.indexOf(saved) >= 0) { cur = modes.indexOf(saved); }
    } catch (e) { /* file:// may deny storage */ }
    function apply() {
      var m = modes[cur];
      if (m === "auto") { document.documentElement.removeAttribute("data-theme"); }
      else { document.documentElement.setAttribute("data-theme", m); }
      btn.textContent = "Theme: " + m;
      try { window.localStorage.setItem("benchviz-theme", m); } catch (e) { /* ignore */ }
    }
    btn.addEventListener("click", function () {
      cur = (cur + 1) % modes.length;
      apply();
    });
    apply();
  }());

  renderHeadline();
  renderCorpus();
  renderFormats();
  renderConfigOrdering();
  renderConfigVariants();
  renderScaling();
  renderOverview();
  renderPareto();
  renderHeat();
  renderSizes();
  renderCompression();
  renderCaveats();
  renderCoverage();
}());
"""

TEMPLATE = """<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>CityParquet benchmarks — selective reads win, full reads pay</title>
<meta name="description" content="Self-contained benchmark summary: CityParquet against \
CityJSONSeq, FlatCityBuf and DuckDB across a corpus of CityJSON datasets — read speed, memory, \
on-disk size and compression variants, with the fairness caveats quoted verbatim.">
<style>
__CSS__
</style>
</head>
<body>
__BODY__
<script type="application/json" id="bench-data">__DATA__</script>
<script>
__JS__
</script>
</body>
</html>
"""


def embed_json(data: dict) -> str:
    """Serialise ``data`` so it can never terminate the host ``<script>``.

    ``<``, ``>`` and ``&`` cannot appear outside JSON strings, so escaping them
    as ``\\uXXXX`` keeps the document valid JSON with byte-identical string
    values.
    """
    return (
        json.dumps(data, ensure_ascii=False)
        .replace("&", "\\u0026")
        .replace("<", "\\u003c")
        .replace(">", "\\u003e")
        .replace(" ", "\\u2028")
        .replace(" ", "\\u2029")
    )


def render(data: dict) -> str:
    return (
        TEMPLATE.replace("__CSS__", CSS.strip())
        .replace("__BODY__", BODY.strip())
        .replace("__DATA__", embed_json(data))
        .replace("__JS__", JS.strip())
    )


def main(data_path: Path | None = None, out_path: Path | None = None) -> Path:
    data_path = data_path or DEFAULT_DATA_PATH
    out_path = out_path or DEFAULT_HTML_PATH
    if not data_path.exists():
        raise SystemExit(
            f"benchviz: {data_path} is missing — run `python -m benchviz prep` first."
        )
    data = json.loads(data_path.read_text(encoding="utf-8"))
    for key in (
        "meta",
        "datasets",
        "read",
        "sizes",
        "compression",
        "compression_gaps",
        "ordering",
        "scaling",
    ):
        if key not in data:
            raise SystemExit(
                f"benchviz: bench_data.json has no '{key}' key — it does not match "
                "the DESIGN.md contract."
            )
    html = render(data)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(html, encoding="utf-8")
    print(
        "benchviz: wrote {} ({:,} bytes, {} datasets, {} read records)".format(
            out_path, len(html.encode("utf-8")), len(data["datasets"]), len(data["read"])
        )
    )
    return out_path


if __name__ == "__main__":  # pragma: no cover
    main()
