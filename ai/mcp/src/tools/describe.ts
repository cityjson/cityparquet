// Answering "what is in this dataset" in one call.

import type { Engine } from "../duckdb.js";

/** Normative, from the specification's dataset-package chapter. */
export const MODULE_TABLES = [
  "building", "bridge", "tunnel", "construction", "transportation",
  "vegetation", "relief", "water_body", "land_use", "city_furniture", "generics",
] as const;

export const SIDECAR_TABLES = ["materials", "textures", "geometry_templates"] as const;

export interface TableSummary {
  readonly name: string;
  readonly file: string;
  readonly row_count: number | null;
  readonly geometry_columns: string[];
  readonly lods: string[];
}

export interface DescribeResult {
  readonly url: string;
  readonly kind: "file" | "package";
  /** Where the file list came from — the STAC Item, or a probe of the normative basenames. */
  readonly inventory: "stac" | "probe";
  readonly crs: string | null;
  readonly stac: Record<string, unknown> | null;
  readonly tables: TableSummary[];
  readonly notes: string[];
}

export function geometryColumnsOf(columns: readonly string[]): string[] {
  return columns.filter((c) => /^geometry_lod\d+_\d+$/.test(c));
}

export function lodsOf(geometryColumns: readonly string[]): string[] {
  const lods = geometryColumns
    .map((c) => /^geometry_lod(\d+)_(\d+)$/.exec(c))
    .filter((m): m is RegExpExecArray => m !== null)
    .map((m) => `${m[1]}.${m[2]}`);
  return [...new Set(lods)].sort();
}

function sqlLiteral(value: string): string {
  return `'${value.replace(/'/g, "''")}'`;
}

async function summariseFile(engine: Engine, url: string, name: string): Promise<TableSummary | null> {
  try {
    const schema = await engine.connection.runAndReadAll(
      `SELECT name FROM parquet_schema(${sqlLiteral(url)})`,
    );
    const columns = schema.getRowsJson().map((row) => String(row[0]));
    const geometryColumns = geometryColumnsOf(columns);

    let rowCount: number | null = null;
    try {
      const meta = await engine.connection.runAndReadAll(
        `SELECT sum(num_rows)::BIGINT FROM parquet_file_metadata(${sqlLiteral(url)})`,
      );
      const value = meta.getRowsJson()[0]?.[0];
      rowCount = value === null || value === undefined ? null : Number(value);
    } catch {
      rowCount = null;
    }

    return {
      name,
      file: url,
      row_count: rowCount,
      geometry_columns: geometryColumns,
      lods: lodsOf(geometryColumns),
    };
  } catch {
    return null;
  }
}

interface ProjJsonId {
  readonly authority?: string;
  readonly code?: string | number;
}

interface ProjJsonCrs {
  readonly name?: string;
  readonly id?: ProjJsonId;
}

interface CityFooter {
  readonly crs?: unknown;
}

interface GeoFooter {
  readonly primary_column?: string;
  readonly columns?: Record<string, { readonly crs?: unknown }>;
}

/**
 * Renders a PROJJSON CRS object to something an agent can act on: enough to
 * tell whether coordinates are metres or degrees, not the full definition.
 */
function renderCrs(crs: unknown): string | null {
  if (crs === null || typeof crs !== "object") return null;
  const candidate = crs as ProjJsonCrs;
  const id = candidate.id;
  if (id && id.authority !== undefined && id.code !== undefined) {
    return candidate.name ? `${candidate.name} (${id.authority}:${id.code})` : `${id.authority}:${id.code}`;
  }
  return typeof candidate.name === "string" && candidate.name.length > 0 ? candidate.name : null;
}

/**
 * The `city` footer key is authoritative for decoding; `geo`'s primary
 * column carries the same information in GeoParquet's own vocabulary and is
 * the fallback when `city` is absent or states no CRS. Both are PROJJSON.
 *
 * Reads the raw footer key-value pairs rather than a decoding function: the
 * `cityjson`/`three_d` extensions this server loads do not expose one (the
 * function the specification's own source documents describe is not in the
 * published community build) — `parquet_kv_metadata`, and the `decode()` a
 * BLOB value needs before it parses as JSON, are both DuckDB core.
 */
async function footerCrs(engine: Engine, url: string): Promise<string | null> {
  try {
    const reader = await engine.connection.runAndReadAll(
      `SELECT key, decode(value) FROM parquet_kv_metadata(${sqlLiteral(url)}) WHERE key IN ('city', 'geo')`,
    );
    const byKey = new Map<string, string>();
    for (const row of reader.getRowsJson()) {
      byKey.set(String(row[0]), String(row[1]));
    }

    const cityRaw = byKey.get("city");
    if (cityRaw) {
      const city = JSON.parse(cityRaw) as CityFooter;
      const rendered = renderCrs(city.crs);
      if (rendered) return rendered;
    }

    const geoRaw = byKey.get("geo");
    if (geoRaw) {
      const geo = JSON.parse(geoRaw) as GeoFooter;
      const primary = geo.primary_column ? geo.columns?.[geo.primary_column] : undefined;
      const rendered = primary ? renderCrs(primary.crs) : null;
      if (rendered) return rendered;
    }

    return null;
  } catch {
    return null;
  }
}

export async function describe(engine: Engine, url: string): Promise<DescribeResult> {
  const notes: string[] = [];
  const trimmed = url.replace(/\/+$/, "");

  if (/\.parquet$/i.test(trimmed)) {
    // One critical section for both queries this path issues, not one each —
    // see the package path below for why.
    return engine.exclusive(async () => {
      const table = await summariseFile(engine, trimmed, trimmed.split("/").pop() ?? trimmed);
      if (!table) throw new Error(`could not read a Parquet footer at ${trimmed}`);
      return {
        url: trimmed,
        kind: "file",
        inventory: "probe",
        crs: await footerCrs(engine, trimmed),
        stac: null,
        tables: [table],
        notes,
      };
    });
  }

  // A package. The STAC Item's assets map is the file inventory — but the
  // specification makes that a SHOULD, and the Item may be absent entirely, so
  // the normative basenames are the fallback.
  let stac: Record<string, unknown> | null = null;
  let files: { name: string; url: string }[] = [];
  let inventory: "stac" | "probe" = "probe";

  try {
    // A hung host must not stall the tool for undici's multi-minute default.
    // Ten seconds is generous for a `metadata.json` fetch and short enough
    // that a caller notices; the probe fallback below absorbs the failure.
    const response = await fetch(`${trimmed}/metadata.json`, { signal: AbortSignal.timeout(10_000) });
    if (response.ok) {
      stac = (await response.json()) as Record<string, unknown>;
      const assets = stac.assets as Record<string, { href?: string }> | undefined;
      if (assets) {
        // No package legitimately contains the same file twice — but an
        // Item's assets map can list one file under more than one role (a
        // generic "data" role alongside a module-named one), so dedupe by
        // the resolved URL and keep the first occurrence.
        const seen = new Set<string>();
        for (const [name, asset] of Object.entries(assets)) {
          if (!asset.href?.endsWith(".parquet")) continue;
          const resolved = new URL(asset.href, `${trimmed}/`).toString();
          if (seen.has(resolved)) continue;
          seen.add(resolved);
          files.push({ name, url: resolved });
        }
        if (files.length > 0) inventory = "stac";
      }
      if (files.length === 0) {
        notes.push("metadata.json carries no Parquet assets; probing the normative basenames instead.");
      }
    } else {
      notes.push(`no metadata.json (HTTP ${response.status}); probing the normative basenames instead.`);
    }
  } catch (error) {
    notes.push(
      `metadata.json unreachable (${error instanceof Error ? error.message : String(error)}); probing the normative basenames instead.`,
    );
  }

  if (files.length === 0) {
    files = [...MODULE_TABLES, ...SIDECAR_TABLES].map((name) => ({
      name,
      url: `${trimmed}/${name}.parquet`,
    }));
  }

  // One critical section for the whole batch — up to fourteen statements
  // (eleven module tables, three sidecars) plus the footer read — not one
  // per statement. All five tools share this connection; contending for it
  // statement by statement would let another tool's query interleave between
  // this batch's own statements just as readily as between two different
  // tools' calls, and it would also make this describe() far slower under
  // concurrent load for no benefit, since the batch has no use for partial
  // interleaving with itself.
  return engine.exclusive(async () => {
    const summaries = await Promise.all(files.map((f) => summariseFile(engine, f.url, f.name)));
    const tables = summaries.filter((t): t is TableSummary => t !== null);
    if (tables.length === 0) throw new Error(`no readable Parquet files under ${trimmed}`);

    const crs = await footerCrs(engine, tables[0]!.file);
    if (crs === null) {
      notes.push("no CRS in the footer — the package states nothing about its coordinate system.");
    }

    return { url: trimmed, kind: "package", inventory, crs, stac, tables, notes };
  });
}
