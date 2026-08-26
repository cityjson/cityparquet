// What a query is about to read, and what it actually read.
//
// The playground already reports rows, time and bytes. This is the same facts
// laid out spatially: a Parquet file is a grid of column chunks — one per
// (row group, column) — and a query touches a rectangle of it. Saying "6.6 MB
// of 16.4 GB" is the claim; drawing the twelve columns of eighty-eight row
// groups that those megabytes came from is the explanation.
//
// Everything here is derived from the file's own footer or from the statement
// text. Nothing is invented: when a fact cannot be had — the host hides
// `Content-Length`, the statement reads no single Parquet file — the caller is
// given `null` and hides that part of the display, which is the same rule the
// byte counter follows.
//
// What this deliberately does **not** know is which chunks were fetched. That
// needs the byte ranges the worker requested mapped onto `parquet_metadata()`
// offsets, and the worker cannot be asked anything while it is executing (see
// `Session.exclusive`). The projection is real; per-chunk reads are not
// claimed.

import { rowsOf } from "./query";
import type { Session } from "./duckdb";

export interface ScanTarget {
  /** The URL as it appears in the statement. */
  readonly url: string;
  /** The last two path segments — `delft/building.parquet`. */
  readonly label: string;
}

export interface ScanColumn {
  readonly name: string;
  readonly type: string;
}

export interface FileStructure {
  readonly target: ScanTarget;
  readonly columns: readonly ScanColumn[];
  readonly rowGroups: number;
  readonly rows: number;
  /** Null when the host does not let the browser read `Content-Length`. */
  readonly bytes: number | null;
}

/** One row of the drawn grid: the row groups it stands for. */
export interface RowBand {
  readonly from: number;
  /** Exclusive. */
  readonly to: number;
}

/**
 * Strip everything that looks like SQL but should not be read as identifiers:
 * line comments, block comments, and string literals. Keeping the delimiters'
 * whitespace preserves token boundaries, so `from'x'where` cannot fuse.
 */
function stripNoise(sql: string): string {
  return sql
    .replace(/--[^\n]*/g, " ")
    .replace(/\/\*[\s\S]*?\*\//g, " ")
    .replace(/'(?:[^']|'')*'/g, " ");
}

/**
 * The Parquet files a statement reads, in order of first appearance.
 *
 * Any string literal ending in `.parquet` counts, which covers `read_parquet`,
 * a bare `FROM 'https://…'`, and the list form — DuckDB accepts all three, and
 * the distinction does not matter to what gets read.
 */
export function parquetTargets(sql: string): ScanTarget[] {
  const found = new Map<string, ScanTarget>();
  for (const match of sql.matchAll(/'((?:[^']|'')*\.parquet)'/gi)) {
    const url = match[1].replace(/''/g, "'");
    if (!found.has(url)) found.set(url, { url, label: labelFor(url) });
  }
  return [...found.values()];
}

/** `https://host/data/delft/building.parquet` → `delft/building.parquet`. */
export function labelFor(url: string): string {
  const path = url.split(/[?#]/)[0];
  return path.split("/").filter(Boolean).slice(-2).join("/") || path;
}

/**
 * Does the statement select every column?
 *
 * `count(*)` does not: Parquet answers it from the footer's row counts without
 * reading a single column chunk, and treating that star as a projection would
 * paint the whole file as read. A star that stands for columns is followed by
 * a comma, a `FROM`, or one of the modifiers DuckDB allows after it.
 */
export function selectsAllColumns(sql: string): boolean {
  const text = stripNoise(sql).replace(/\(\s*\*\s*\)/g, " ");
  return /(?:^|[\s,])(?:[a-z_]\w*\.)?\*(?=\s*(?:,|from\b|except\b|exclude\b|replace\b|$))/i.test(
    text,
  );
}

/**
 * Which of `columns` the statement names.
 *
 * Every identifier in the statement is considered, not just the select list:
 * a column in a `WHERE` clause is read as surely as one that is returned, and
 * showing only the output columns would understate what the scan costs.
 *
 * The trade is that a column whose name collides with a keyword or an alias
 * used elsewhere is counted in. Over-reporting is the safe direction — this
 * figure exists to make a query's cost visible, and a projection drawn smaller
 * than the truth would flatter it.
 */
export function projectedColumns(sql: string, columns: readonly ScanColumn[]): ScanColumn[] {
  if (selectsAllColumns(sql)) return [...columns];
  const named = new Set(
    (
      stripNoise(sql)
        .toLowerCase()
        .match(/[a-z_]\w*/g) ?? []
    ).map((token) => token),
  );
  return columns.filter((column) => named.has(column.name.toLowerCase()));
}

/**
 * Fold `count` row groups into at most `max` drawn bands.
 *
 * A national package has more row groups than a grid has pixels, so past the
 * limit each drawn row stands for several. The caller says so in words; a grid
 * that silently showed the first 48 of 88 would be a lie of omission.
 */
export function rowBands(count: number, max: number): RowBand[] {
  if (count <= 0 || max <= 0) return [];
  if (count <= max) return Array.from({ length: count }, (_v, i) => ({ from: i, to: i + 1 }));
  const size = count / max;
  return Array.from({ length: max }, (_v, i) => ({
    from: Math.floor(i * size),
    to: i === max - 1 ? count : Math.floor((i + 1) * size),
  }));
}

/** Single-quote a URL for interpolation into SQL. */
function quote(url: string): string {
  return `'${url.replace(/'/g, "''")}'`;
}

/**
 * The file's real shape: its columns, its row groups, and its size.
 *
 * All three come from the footer, which `DESCRIBE` has usually pulled already,
 * so this costs a round trip rather than a download. The size is the exception
 * — Parquet's footer does not record it — and comes from a `HEAD`, whose
 * `Content-Length` is CORS-safelisted and therefore readable cross-origin even
 * from a host that exposes nothing else.
 */
export async function readStructure(session: Session, target: ScanTarget): Promise<FileStructure> {
  const url = quote(target.url);

  const [meta] = await rowsOf(
    session,
    `SELECT num_rows, num_row_groups FROM parquet_file_metadata(${url})`,
  );
  const described = await rowsOf(session, `DESCRIBE SELECT * FROM read_parquet(${url})`);

  return {
    target,
    columns: described.map((row) => ({
      name: String(row.column_name ?? ""),
      type: String(row.column_type ?? ""),
    })),
    rowGroups: Number(meta?.num_row_groups ?? 0),
    rows: Number(meta?.num_rows ?? 0),
    bytes: await headLength(target.url),
  };
}

/** The file's length, or null if the host will not say. Never a guess. */
async function headLength(url: string): Promise<number | null> {
  try {
    const response = await fetch(url, { method: "HEAD" });
    const length = response.headers.get("Content-Length");
    if (!length) return null;
    const parsed = Number.parseInt(length, 10);
    return Number.isFinite(parsed) ? parsed : null;
  } catch {
    return null;
  }
}
