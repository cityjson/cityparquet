// The example queries.
//
// Every one of these was run against the real dataset before being listed, and
// the figures quoted in the blurbs are measured rather than estimated.
//
// Two facts about the 3DBAG package shape everything here:
//
//   * `Building` rows carry the attributes (all 10,771,547 of them) and no
//     geometry at all; `BuildingPart` rows carry the geometry (10,783,975 with
//     LoD 2.2) and no attributes. Anything using both has to join them through
//     `parents` / `children`.
//   * It is one 16.4 GB file. A query that avoids the geometry columns touches a
//     tiny fraction of it, which is the entire point — but `SELECT *` is an
//     expensive mistake, so nothing here does that without a `LIMIT`.

import { data } from "./config";
import type { ExtensionName } from "./config";

export interface Preset {
  /** Stable: it appears in share links, and documentation points at it. */
  readonly id: string;
  readonly group: string;
  readonly title: string;
  /** One line on what it demonstrates — including any caveat. */
  readonly blurb: string;
  readonly extensions: readonly ExtensionName[];
  readonly sql: string;
}

const BUILDINGS = data("3dbag/building.parquet");

export const PRESETS: readonly Preset[] = [
  // ── Start here ────────────────────────────────────────────────────────────
  {
    id: "schema",
    group: "Start here",
    title: "What is in the file?",
    blurb:
      "88 columns, read from the 4.7 MB Parquet footer alone — none of the 16.4 GB of data is touched.",
    extensions: [],
    sql: `-- DESCRIBE reads the Parquet footer and nothing else.
-- The whole file is 16.4 GB; this reads about 4.7 MB of it.
DESCRIBE
SELECT * FROM read_parquet('${BUILDINGS}');`,
  },
  {
    id: "count",
    group: "Start here",
    title: "Count the whole of the Netherlands",
    blurb:
      "10.7 million buildings and 10.8 million parts. Reads one column of 21.5 million rows, not the geometry.",
    extensions: [],
    sql: `-- Every building in the Netherlands, counted from a browser tab.
-- Only the object_type column is read: the geometry columns, which are almost
-- all of the 16.4 GB, are never fetched.
SELECT
    count(*)                                     AS rows_total,
    count(*) FILTER (object_type = 'Building')     AS buildings,
    count(*) FILTER (object_type = 'BuildingPart') AS building_parts
FROM read_parquet('${BUILDINGS}');`,
  },
  {
    id: "peek",
    group: "Start here",
    title: "Look at a few rows",
    blurb:
      "A handful of attribute columns, with a LIMIT — the safe way to explore a file this size.",
    extensions: [],
    sql: `-- Naming the columns keeps this cheap. SELECT * would pull the geometry
-- BLOBs for every row it touches.
SELECT id, object_type, b3_dak_type, b3_h_dak_max, oorspronkelijkbouwjaar, status
FROM read_parquet('${BUILDINGS}')
WHERE object_type = 'Building'
LIMIT 20;`,
  },

  // ── Attributes ────────────────────────────────────────────────────────────
  {
    id: "roof-types",
    group: "Attributes",
    title: "Roof-type distribution",
    blurb:
      "Groups 10.7 million buildings by reconstructed roof type. The attributes live on Building rows.",
    extensions: [],
    sql: `-- b3_dak_type is 3DBAG's reconstructed roof type. It is present on every
-- Building row and on no BuildingPart row.
SELECT b3_dak_type AS roof_type, count(*) AS buildings
FROM read_parquet('${BUILDINGS}')
WHERE object_type = 'Building' AND b3_dak_type IS NOT NULL
GROUP BY roof_type
ORDER BY buildings DESC;`,
  },
  {
    id: "by-decade",
    group: "Attributes",
    title: "When was the Netherlands built?",
    blurb:
      "Construction year by decade, across every building in the country. Two columns of 21.5 million rows.",
    extensions: [],
    sql: `-- oorspronkelijkbouwjaar is the BAG's original construction year.
SELECT
    (oorspronkelijkbouwjaar // 10) * 10 AS decade,
    count(*)                            AS buildings
FROM read_parquet('${BUILDINGS}')
WHERE object_type = 'Building'
  AND oorspronkelijkbouwjaar BETWEEN 1000 AND 2100
GROUP BY decade
ORDER BY decade;`,
  },
  {
    id: "tallest",
    group: "Attributes",
    title: "The tallest roofs",
    blurb:
      "Ranks by measured roof height. An ORDER BY over 10.7 million rows still reads only two columns.",
    extensions: [],
    sql: `-- b3_h_dak_max is the maximum roof height, in metres above NAP.
SELECT id, b3_dak_type AS roof_type, b3_h_dak_max AS roof_height_m, b3_h_maaiveld AS ground_m
FROM read_parquet('${BUILDINGS}')
WHERE object_type = 'Building' AND b3_h_dak_max IS NOT NULL
ORDER BY b3_h_dak_max DESC
LIMIT 25;`,
  },

  // ── Space ─────────────────────────────────────────────────────────────────
  {
    id: "bbox-search",
    group: "Space",
    title: "Bounding-box search",
    blurb:
      "Finds the parts inside a small area of Delft using the bbox struct every object carries.",
    extensions: [],
    sql: `-- Every CityParquet object carries a 3D bbox STRUCT, so a spatial filter
-- needs no spatial index and no geometry read. Coordinates are EPSG:7415
-- (Amersfoort / RD New + NAP), the CRS 3DBAG publishes.
SELECT id, bbox.xmin, bbox.ymin, bbox.zmin, bbox.zmax
FROM read_parquet('${BUILDINGS}')
WHERE object_type = 'BuildingPart'
  AND bbox.xmin BETWEEN 84000 AND 85000
  AND bbox.ymin BETWEEN 446000 AND 447000
LIMIT 50;`,
  },
  {
    id: "parent-child",
    group: "Space",
    title: "Join attributes to geometry",
    blurb:
      "The CityGML model made visible: attributes sit on the Building, geometry on its BuildingPart.",
    extensions: [],
    sql: `-- Attributes and geometry live on different rows, linked by parents/children.
-- This joins a part back to its parent to get both at once.
WITH parts AS (
    SELECT id, parents[1] AS parent_id, octet_length(geometry_lod2_2) AS geometry_bytes
    FROM read_parquet('${BUILDINGS}')
    WHERE object_type = 'BuildingPart'
      AND bbox.xmin BETWEEN 84000 AND 84500
      AND bbox.ymin BETWEEN 446000 AND 446500
),
buildings AS (
    SELECT id, b3_dak_type, b3_volume_lod22, oorspronkelijkbouwjaar
    FROM read_parquet('${BUILDINGS}')
    WHERE object_type = 'Building'
)
SELECT p.id AS part_id, b.b3_dak_type AS roof_type,
       b.b3_volume_lod22 AS volume_m3, b.oorspronkelijkbouwjaar AS built,
       p.geometry_bytes
FROM parts p
JOIN buildings b ON b.id = p.parent_id
LIMIT 50;`,
  },

  // ── 3D geometry ───────────────────────────────────────────────────────────
  {
    id: "solid-volume",
    group: "3D geometry",
    title: "Measure solids in the browser",
    blurb:
      "Computes volume and surface area from the LoD 2.2 WKB solids themselves, with the three_d extension.",
    extensions: ["three_d"],
    sql: `-- geometry_lod2_2 is a WKB PolyhedralSurface. three_d measures it directly,
-- so this is computed from the geometry rather than read from an attribute.
-- The LIMIT matters: this one does read geometry.
SELECT
    id,
    round(ST_3DVolume(geometry_lod2_2), 1)      AS volume_m3,
    round(ST_3DSurfaceArea(geometry_lod2_2), 1) AS surface_m2,
    ST_3DNumFaces(geometry_lod2_2)              AS faces,
    ST_3DIsClosed(geometry_lod2_2)              AS closed
FROM read_parquet('${BUILDINGS}')
WHERE object_type = 'BuildingPart'
  AND geometry_lod2_2 IS NOT NULL
  AND bbox.xmin BETWEEN 84000 AND 84500
  AND bbox.ymin BETWEEN 446000 AND 446500
LIMIT 25;`,
  },
  {
    id: "volume-check",
    group: "3D geometry",
    title: "Check the published volume",
    blurb:
      "Compares three_d's computed volume against 3DBAG's own b3_volume_lod22 — two routes to one number.",
    extensions: ["three_d"],
    sql: `-- The attribute and the geometry should agree. Where they do not, one of
-- them is wrong -- which is exactly the kind of question a playground is for.
WITH parts AS (
    SELECT parents[1] AS parent_id, geometry_lod2_2
    FROM read_parquet('${BUILDINGS}')
    WHERE object_type = 'BuildingPart'
      AND geometry_lod2_2 IS NOT NULL
      AND bbox.xmin BETWEEN 84000 AND 84300
      AND bbox.ymin BETWEEN 446000 AND 446300
),
buildings AS (
    SELECT id, b3_volume_lod22
    FROM read_parquet('${BUILDINGS}')
    WHERE object_type = 'Building'
)
SELECT
    b.id,
    round(ST_3DVolume(p.geometry_lod2_2), 1) AS computed_m3,
    round(b.b3_volume_lod22, 1)              AS published_m3,
    round(ST_3DVolume(p.geometry_lod2_2) - b.b3_volume_lod22, 2) AS difference
FROM parts p
JOIN buildings b ON b.id = p.parent_id
LIMIT 25;`,
  },
  {
    id: "lod-sizes",
    group: "3D geometry",
    title: "What each level of detail costs",
    blurb:
      "Average encoded geometry size per LoD, over a sample — why multi-LoD storage in one file pays off.",
    extensions: [],
    sql: `-- One row per object holds every LoD it has, so a reader takes only the one
-- it asked for. These are the encoded WKB sizes of each.
SELECT
    round(avg(octet_length(geometry_lod0_0)), 1) AS lod0_0_bytes,
    round(avg(octet_length(geometry_lod1_2)), 1) AS lod1_2_bytes,
    round(avg(octet_length(geometry_lod1_3)), 1) AS lod1_3_bytes,
    round(avg(octet_length(geometry_lod2_2)), 1) AS lod2_2_bytes,
    count(*)                                     AS sampled_parts
FROM read_parquet('${BUILDINGS}')
WHERE object_type = 'BuildingPart'
  AND bbox.xmin BETWEEN 84000 AND 84500
  AND bbox.ymin BETWEEN 446000 AND 446500;`,
  },

  // ── CityJSON ──────────────────────────────────────────────────────────────
  {
    id: "cityjsonseq",
    group: "CityJSON",
    title: "Read CityJSONSeq straight from a URL",
    blurb:
      "The cityjson extension reads the text format directly — no conversion step, no download.",
    extensions: ["cityjson"],
    sql: `-- CityParquet is not the only thing this stack reads. Delft, as published
-- CityJSONSeq, parsed in the browser. The reader gives every city object the
-- same column grammar CityParquet uses, so 'object_type' is the type column.
SELECT object_type, count(*) AS objects
FROM read_cityjsonseq('https://cityjson.open3d.city/cityjsonseq/delft.city.jsonl')
GROUP BY object_type
ORDER BY objects DESC;`,
  },
  {
    id: "cityjson-metadata",
    group: "CityJSON",
    title: "CityJSONSeq metadata",
    blurb: "The header of a CityJSONSeq stream: CRS, extent, and the transform it uses.",
    extensions: ["cityjson"],
    sql: `SELECT *
FROM cityjsonseq_metadata('https://cityjson.open3d.city/cityjsonseq/delft.city.jsonl');`,
  },
];

export const DEFAULT_PRESET_ID = "count";

export function findPreset(id: string | null): Preset | undefined {
  if (!id) return undefined;
  return PRESETS.find((preset) => preset.id === id);
}
