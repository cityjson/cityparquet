// Answering "what is in this dataset" in one call.

import { lookup } from "node:dns/promises";
import { readFile } from "node:fs/promises";
import { join as joinPath, resolve as resolvePath } from "node:path";
import { fileURLToPath } from "node:url";

import type { Engine } from "../duckdb.js";
import { isInternalAddress } from "../egress-proxy.js";

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
  /** This table's own footer CRS. Tables in one package can disagree. */
  readonly crs: string | null;
}

export interface DescribeResult {
  readonly url: string;
  readonly kind: "file" | "package";
  /** Where the file list came from — the STAC Item, or a probe of the normative basenames. */
  readonly inventory: "stac" | "probe";
  /** The package's CRS when every table that states one agrees; null otherwise — see `notes`. */
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

function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/** A table is named after its file: `building.parquet` is `building`. */
function tableName(file: string): string {
  const last = file.split(/[\\/]/).pop() ?? file;
  return last.replace(/\.parquet$/i, "");
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

function crsKey(crs: unknown): string | null {
  if (crs === null || typeof crs !== "object") return null;
  const id = (crs as ProjJsonCrs).id;
  if (id && id.authority !== undefined && id.code !== undefined) {
    return `${String(id.authority).toUpperCase()}:${String(id.code)}`;
  }
  return JSON.stringify(crs);
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

interface FooterCrs {
  readonly crs: string | null;
  /**
   * What makes two CRSs the same one: `authority:code` when the PROJJSON
   * carries an id, else the whole definition. Never the rendered string —
   * an id with and without a `name` is one CRS, and two unidentified
   * definitions sharing a name are not.
   */
  readonly key: string | null;
  /** What was wrong with the footer, if anything — for `notes`. */
  readonly problems: string[];
}

/**
 * The `city` footer key is authoritative for decoding; `geo`'s primary
 * column carries the same information in GeoParquet's own vocabulary and is
 * the fallback when `city` is absent, states no CRS, or does not parse. Both
 * are PROJJSON. Each key is parsed on its own, so a malformed `city` cannot
 * take the `geo` fallback down with it.
 *
 * Reads the raw footer key-value pairs with DuckDB core alone —
 * `parquet_kv_metadata`, and the `decode()` a BLOB value needs before it
 * parses as JSON — rather than `cityjson`'s own footer helpers, so describe
 * works on any engine, whichever extensions it was brought up with.
 */
async function footerCrs(engine: Engine, file: string): Promise<FooterCrs> {
  const problems: string[] = [];
  const byKey = new Map<string, string>();
  try {
    const reader = await engine.connection.runAndReadAll(
      `SELECT key, decode(value) FROM parquet_kv_metadata(${sqlLiteral(file)}) WHERE key IN ('city', 'geo')`,
    );
    for (const row of reader.getRowsJson()) byKey.set(String(row[0]), String(row[1]));
  } catch (error) {
    return { crs: null, key: null, problems: [`footer key-value metadata unreadable (${messageOf(error)})`] };
  }

  const cityRaw = byKey.get("city");
  if (cityRaw !== undefined) {
    try {
      const crs = (JSON.parse(cityRaw) as CityFooter | null)?.crs;
      const rendered = renderCrs(crs);
      if (rendered) return { crs: rendered, key: crsKey(crs), problems };
    } catch {
      problems.push("the city footer is not valid JSON");
    }
  }

  const geoRaw = byKey.get("geo");
  if (geoRaw !== undefined) {
    try {
      const geo = JSON.parse(geoRaw) as GeoFooter | null;
      const primary = geo?.primary_column ? geo.columns?.[geo.primary_column] : undefined;
      const rendered = primary ? renderCrs(primary.crs) : null;
      if (rendered) {
        if (problems.length > 0) problems.push("the CRS was read from the geo footer instead");
        return { crs: rendered, key: crsKey(primary!.crs), problems };
      }
    } catch {
      problems.push("the geo footer is not valid JSON");
    }
  }

  return { crs: null, key: null, problems };
}

async function summariseFile(
  engine: Engine,
  file: string,
  name: string,
): Promise<{ table: TableSummary; crsKey: string | null; problems: string[] } | null> {
  let columns: string[];
  try {
    const schema = await engine.connection.runAndReadAll(`SELECT name FROM parquet_schema(${sqlLiteral(file)})`);
    columns = schema.getRowsJson().map((row) => String(row[0]));
  } catch {
    return null;
  }
  const geometryColumns = geometryColumnsOf(columns);

  let rowCount: number | null = null;
  try {
    const meta = await engine.connection.runAndReadAll(
      `SELECT sum(num_rows)::BIGINT FROM parquet_file_metadata(${sqlLiteral(file)})`,
    );
    const value = meta.getRowsJson()[0]?.[0];
    rowCount = value === null || value === undefined ? null : Number(value);
  } catch {
    rowCount = null;
  }

  const { crs, key, problems } = await footerCrs(engine, file);
  return {
    table: { name, file, row_count: rowCount, geometry_columns: geometryColumns, lods: lodsOf(geometryColumns), crs },
    crsKey: key,
    problems,
  };
}

/** A scheme followed by `//`: `https://`, `s3://`, `file://`. A bare path has none. */
const REMOTE = /^[A-Za-z][A-Za-z0-9+.-]*:\/\//;

type Location = { readonly local: true; readonly path: string } | { readonly local: false; readonly url: string };

function locate(input: string): Location {
  const trimmed = input.length > 1 ? input.replace(/\/+$/, "") : input;
  if (/^file:/i.test(trimmed)) return { local: true, path: fileURLToPath(trimmed) };
  if (REMOTE.test(trimmed)) return { local: false, url: trimmed };
  return { local: true, path: resolvePath(trimmed) };
}

const PROBING = "probing the normative basenames instead.";

/**
 * The package's STAC Item, if there is one, and a note saying precisely why
 * not if there is not. "Unreachable" is reserved for a request that failed —
 * a missing file, an HTTP error and a malformed body are each named as what
 * they are.
 */
/**
 * Why Node may not fetch `url` itself, or null if it may. DuckDB's reads go
 * through the egress proxy; this fetch does not, so on an engine with an
 * allowlist it applies the proxy's rules: HTTPS on the default port, to an
 * allowlisted host, which does not resolve to an internal address.
 */
async function fetchRefusal(url: string, allowedHosts: readonly string[] | undefined): Promise<string | null> {
  if (!allowedHosts) return null;
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return "not a URL";
  }
  const { protocol, hostname, port } = parsed;
  if (protocol !== "https:" || port !== "") return `only HTTPS on the default port is allowed`;
  if (!allowedHosts.some((host) => host.toLowerCase() === hostname.toLowerCase())) {
    return `${hostname} is not on this server's allowlist (HTTPS to ${allowedHosts.join(", ")})`;
  }
  try {
    const addresses = await lookup(hostname, { all: true });
    if (addresses.some((address) => isInternalAddress(address.address))) return `${hostname} resolves to an internal address`;
  } catch {
    return `${hostname} does not resolve`;
  }
  return null;
}

async function readItem(
  location: Location,
  allowedHosts?: readonly string[],
): Promise<{ item: unknown; note?: string }> {
  let text: string;
  if (location.local) {
    const path = joinPath(location.path, "metadata.json");
    try {
      text = await readFile(path, "utf8");
    } catch (error) {
      const code = (error as NodeJS.ErrnoException).code;
      return code === "ENOENT"
        ? { item: null, note: `no metadata.json in ${location.path}; ${PROBING}` }
        : { item: null, note: `metadata.json unreadable (${messageOf(error)}); ${PROBING}` };
    }
  } else {
    const itemUrl = `${location.url}/metadata.json`;
    const refusal = await fetchRefusal(itemUrl, allowedHosts);
    if (refusal) return { item: null, note: `metadata.json not fetched: ${refusal}; ${PROBING}` };
    let response: Response;
    try {
      // A hung host must not stall the tool for undici's multi-minute
      // default. Ten seconds is generous for a `metadata.json` fetch and
      // short enough that a caller notices; the probe fallback absorbs it.
      // With an allowlist, a redirect is not followed: it could lead off it.
      // It arrives as a 3xx and is reported like any other non-OK status.
      response = await fetch(itemUrl, {
        signal: AbortSignal.timeout(10_000),
        ...(allowedHosts ? { redirect: "manual" as const } : {}),
      });
    } catch (error) {
      return { item: null, note: `metadata.json unreachable (${messageOf(error)}); ${PROBING}` };
    }
    if (!response.ok) {
      return { item: null, note: `no metadata.json (HTTP ${response.status}); ${PROBING}` };
    }
    // Reading the body and parsing it are separate steps, because they fail
    // for different reasons: a body cut off mid-read (a reset, the timeout)
    // is a network failure, and only a body that arrived whole can be
    // malformed.
    try {
      text = await response.text();
    } catch (error) {
      return { item: null, note: `metadata.json unreachable (${messageOf(error)}); ${PROBING}` };
    }
  }

  try {
    return { item: JSON.parse(text) };
  } catch (error) {
    return { item: null, note: `metadata.json is not valid JSON (${messageOf(error)}); ${PROBING}` };
  }
}

/** An asset href, resolved against the package: a URL for a remote package, a path for a local one. */
function resolveHref(location: Location, href: string): string {
  if (location.local) {
    if (/^file:/i.test(href)) return fileURLToPath(href);
    if (REMOTE.test(href)) return href;
    return resolvePath(location.path, href);
  }
  return new URL(href, `${location.url}/`).toString();
}

export async function describe(engine: Engine, url: string): Promise<DescribeResult> {
  const notes: string[] = [];
  const location = locate(url);
  const shown = location.local ? location.path : location.url;

  // Node's `fs` is not governed by DuckDB's `disabled_filesystems`, so a
  // sandboxed engine must refuse a local path here, before `readFile` — or
  // this tool becomes the local-file read the sandbox exists to prevent.
  if (location.local && engine.sandbox) {
    throw new Error(
      `${shown} is a local path, and this server's engine is sandboxed with no local filesystem — pass an http(s) URL`,
    );
  }

  if (/\.parquet$/i.test(shown)) {
    // One critical section for every query this path issues, not one each —
    // see the package path below for why.
    return engine.exclusive(async () => {
      const summary = await summariseFile(engine, shown, tableName(shown));
      if (!summary) throw new Error(`could not read a Parquet footer at ${shown}`);
      notes.push(...summary.problems.map((p) => `${summary.table.name}: ${p}.`));
      return {
        url: shown,
        kind: "file",
        inventory: "probe",
        crs: summary.table.crs,
        stac: null,
        tables: [summary.table],
        notes,
      };
    });
  }

  // A package. The STAC Item's assets map is the file inventory — but the
  // specification makes that a SHOULD, and the Item may be absent entirely, so
  // the normative basenames are the fallback.
  let stac: Record<string, unknown> | null = null;
  let files: { name: string; file: string }[] = [];
  let inventory: "stac" | "probe" = "probe";

  const { item, note } = await readItem(location, engine.allowedHosts);
  if (note) notes.push(note);
  if (item !== null && typeof item === "object" && !Array.isArray(item)) {
    stac = item as Record<string, unknown>;
    const assets = stac.assets;
    if (assets !== null && typeof assets === "object") {
      // An Item's assets map can list one file under more than one key — a
      // generic "data" role alongside a module-named one — so dedupe by the
      // resolved location and name each table after its file, not its key.
      const seen = new Set<string>();
      for (const [key, asset] of Object.entries(assets as Record<string, { href?: unknown } | null>)) {
        const href = asset?.href;
        if (typeof href !== "string" || !/\.parquet$/i.test(href)) continue;
        let file: string;
        try {
          file = resolveHref(location, href);
        } catch {
          notes.push(`asset '${key}' has an href that does not resolve (${href}); skipped.`);
          continue;
        }
        if (seen.has(file)) continue;
        seen.add(file);
        files.push({ name: tableName(file), file });
      }
      if (files.length > 0) inventory = "stac";
    }
    if (files.length === 0) notes.push(`metadata.json carries no Parquet assets; ${PROBING}`);
  } else if (item !== null) {
    notes.push(`metadata.json is not a STAC Item (not a JSON object); ${PROBING}`);
  }

  if (files.length === 0) {
    files = [...MODULE_TABLES, ...SIDECAR_TABLES].map((name) => ({
      name,
      file: location.local ? joinPath(location.path, `${name}.parquet`) : `${location.url}/${name}.parquet`,
    }));
  }

  // One critical section for the whole batch — up to fourteen files, three
  // statements each — not one per statement. All five tools share this
  // connection; contending for it statement by statement would let another
  // tool's query interleave between this batch's own statements, and would
  // make describe() far slower under concurrent load for no benefit.
  return engine.exclusive(async () => {
    const summaries = await Promise.all(files.map((f) => summariseFile(engine, f.file, f.name)));
    const tables: TableSummary[] = [];
    const keys = new Map<TableSummary, string | null>();
    summaries.forEach((summary, index) => {
      if (summary) {
        tables.push(summary.table);
        keys.set(summary.table, summary.crsKey);
        notes.push(...summary.problems.map((p) => `${summary.table.name}: ${p}.`));
      } else if (inventory === "stac") {
        // A probed basename that is absent is expected; a file the Item
        // lists and that cannot be read is not.
        notes.push(`${files[index]!.name}: listed in metadata.json but not readable as Parquet; omitted.`);
      }
    });
    if (tables.length === 0) throw new Error(`no readable Parquet files under ${shown}`);

    return { url: shown, kind: "package", inventory, crs: packageCrs(tables, keys, notes), stac, tables, notes };
  });
}

/**
 * One CRS for the package when the tables that state one agree. When they
 * do not, there is no honest single answer: report null and let each
 * table's own `crs` speak, rather than whichever table happened to be first.
 */
function packageCrs(
  tables: readonly TableSummary[],
  keys: ReadonlyMap<TableSummary, string | null>,
  notes: string[],
): string | null {
  const stated = tables.filter((t) => t.crs !== null);
  const distinct = new Set(stated.map((t) => keys.get(t) ?? t.crs));
  if (distinct.size === 0) {
    notes.push("no CRS in any table's footer — the package states nothing about its coordinate system.");
    return null;
  }
  if (distinct.size > 1) {
    const listed = tables.map((t) => `${t.name}: ${t.crs ?? "none"}`).join("; ");
    notes.push(`the tables disagree on CRS (${listed}), so no package CRS is reported — see each table's crs.`);
    return null;
  }
  const silent = tables.filter((t) => t.crs === null).map((t) => t.name);
  // The most informative rendering of the one CRS: a table that names it
  // over one that gives only the id.
  const shown = stated.map((t) => t.crs!).sort((a, b) => b.length - a.length)[0]!;
  if (silent.length > 0) {
    notes.push(`${silent.join(", ")} ${silent.length === 1 ? "states" : "state"} no CRS in the footer; the other tables state ${shown}.`);
  }
  return shown;
}
