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
  readonly rowCount: number | null;
  readonly geometryColumns: string[];
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

    return { name, file: url, rowCount, geometryColumns, lods: lodsOf(geometryColumns) };
  } catch {
    return null;
  }
}

/** The `city` footer is authoritative for decoding; the STAC Item is a mirror. */
async function footerCrs(engine: Engine, url: string): Promise<string | null> {
  try {
    const reader = await engine.connection.runAndReadAll(
      `SELECT cityparquet_city_field(city, 'referenceSystem')
       FROM (SELECT cityjson_geoparquet_geo(${sqlLiteral(url)}).city AS city)`,
    );
    const value = reader.getRowsJson()[0]?.[0];
    return value === null || value === undefined ? null : String(value);
  } catch {
    return null;
  }
}

export async function describe(engine: Engine, url: string): Promise<DescribeResult> {
  const notes: string[] = [];
  const trimmed = url.replace(/\/+$/, "");

  if (/\.parquet$/i.test(trimmed)) {
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
  }

  // A package. The STAC Item's assets map is the file inventory — but the
  // specification makes that a SHOULD, and the Item may be absent entirely, so
  // the normative basenames are the fallback.
  let stac: Record<string, unknown> | null = null;
  let files: { name: string; url: string }[] = [];
  let inventory: "stac" | "probe" = "probe";

  try {
    const response = await fetch(`${trimmed}/metadata.json`);
    if (response.ok) {
      stac = (await response.json()) as Record<string, unknown>;
      const assets = stac.assets as Record<string, { href?: string }> | undefined;
      if (assets) {
        files = Object.entries(assets)
          .filter(([, asset]) => asset.href?.endsWith(".parquet"))
          .map(([name, asset]) => ({
            name,
            url: new URL(asset.href!, `${trimmed}/`).toString(),
          }));
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

  const summaries = await Promise.all(files.map((f) => summariseFile(engine, f.url, f.name)));
  const tables = summaries.filter((t): t is TableSummary => t !== null);
  if (tables.length === 0) throw new Error(`no readable Parquet files under ${trimmed}`);

  const crs = await footerCrs(engine, tables[0]!.file);
  if (crs === null) notes.push("no CRS in the footer — the package states nothing about its coordinate system.");

  return { url: trimmed, kind: "package", inventory, crs, stac, tables, notes };
}
