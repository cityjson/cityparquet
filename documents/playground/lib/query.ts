// Running a query and turning Arrow into something a table can render.

import { QUERY_TIMEOUT_MS, ROW_DISPLAY_CAP } from "../config";
import { readByteStats } from "./bytes";
import { describe, type Session } from "./duckdb";

export type Cell = string | number | boolean | null;

export interface QueryResult {
  readonly columns: readonly string[];
  readonly rows: readonly (readonly Cell[])[];
  /** Rows returned, which may exceed the number kept in `rows`. */
  readonly rowCount: number;
  readonly truncated: boolean;
  readonly elapsedMs: number;
  /** Null when the byte counter is unavailable — never a guess. */
  readonly bytesRead: number | null;
}

export interface QueryFailure {
  readonly kind: "extension" | "network" | "timeout" | "sql";
  readonly message: string;
  readonly elapsedMs: number;
}

export class QueryError extends Error {
  readonly failure: QueryFailure;
  constructor(failure: QueryFailure) {
    super(failure.message);
    this.name = "QueryError";
    this.failure = failure;
  }
}

/**
 * Arrow values are not all JSON-friendly: integers arrive as BigInt, and nested
 * types as objects. Render them in a way that is honest about what they are
 * without pretending a struct is a scalar.
 */
function toCell(value: unknown): Cell {
  if (value === null || value === undefined) return null;
  if (typeof value === "bigint") return Number(value);
  if (typeof value === "number" || typeof value === "boolean") return value;
  if (typeof value === "string") return value;
  if (value instanceof Uint8Array) return `<${value.byteLength} bytes>`;
  if (value instanceof Date) return value.toISOString();
  try {
    return JSON.stringify(value, (_key, inner) =>
      typeof inner === "bigint" ? Number(inner) : inner,
    );
  } catch {
    return String(value);
  }
}

/** The deadline as the message should read it: minutes once it is minutes long. */
export function formatDeadline(ms: number): string {
  const seconds = Math.round(ms / 1_000);
  if (seconds < 120) return `${seconds}s`;
  return `${Math.round(seconds / 60)} min`;
}

/**
 * Classify a failure so the interface can say something useful.
 *
 * The network case matters most. A host that answers range requests but does
 * not expose `Accept-Ranges` to the browser produces no error — the read simply
 * never finishes — so a timeout here usually means CORS, not a slow network,
 * and saying so saves the reader a long detour.
 */
function classify(message: string): QueryFailure["kind"] {
  const text = message.toLowerCase();
  if (text.includes("timeout")) return "timeout";
  // Extension failures are checked first: their messages carry the repository
  // URL, so a plain "does it mention http" test misfiles them as network faults
  // and offers the reader a CORS explanation for a missing extension.
  if (text.includes("extension")) return "extension";
  if (
    text.includes("http") ||
    text.includes("failed to load") ||
    text.includes("xmlhttprequest") ||
    text.includes("network")
  ) {
    return "network";
  }
  return "sql";
}

/** Run one statement, with a deadline, and collect what the UI needs. */
export async function runQuery(session: Session, sql: string): Promise<QueryResult> {
  // The statement and the two byte readings are one exclusive block: the
  // counter is a running total, so a schema lookup landing between the readings
  // would be billed to the reader, and one landing *on* them would time them
  // out and take the readout away altogether.
  const measured = session.exclusive(async () => {
    const before = await readByteStats(session.worker);
    // Timed from here, so a queue this waited behind is not reported as the
    // statement's own time.
    const started = performance.now();
    const table = await session.connection.query(sql);
    const elapsedMs = performance.now() - started;
    const after = await readByteStats(session.worker);
    return {
      table,
      elapsedMs,
      /** Null when the counter is unavailable — never a guess. */
      bytesRead: before && after ? Math.max(0, after.bytes - before.bytes) : null,
    };
  });

  const waitStarted = performance.now();
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    // The deadline races the *wait*, not the statement, which keeps its place
    // in the queue and runs to completion either way — so nothing starts on top
    // of a query this gave up on, and nothing else runs until it finishes.
    // DuckDB-Wasm's `cancelSent()` cannot help: the worker is inside the
    // statement, so it will not read the cancellation until the statement ends.
    const { table, elapsedMs, bytesRead } = await Promise.race([
      measured,
      new Promise<never>((_resolve, reject) => {
        timer = setTimeout(
          () =>
            reject(
              new Error(
                `Timeout after ${formatDeadline(QUERY_TIMEOUT_MS)}. ` +
                  "If the query reads a remote file, the host may not be exposing " +
                  "the CORS range headers a reader needs (Accept-Ranges, Content-Range, ETag).",
              ),
            ),
          QUERY_TIMEOUT_MS,
        );
      }),
    ]);

    const columns = table.schema.fields.map((field) => field.name);
    const all = table.toArray();
    const kept = all.slice(0, ROW_DISPLAY_CAP);
    const rows = kept.map((row) => {
      const record = row.toJSON() as Record<string, unknown>;
      return columns.map((column) => toCell(record[column]));
    });

    return {
      columns,
      rows,
      rowCount: all.length,
      truncated: all.length > kept.length,
      elapsedMs,
      bytesRead,
    };
  } catch (error) {
    const message = describe(error);
    throw new QueryError({
      kind: classify(message),
      message,
      elapsedMs: performance.now() - waitStarted,
    });
  } finally {
    if (timer !== undefined) clearTimeout(timer);
  }
}

/**
 * Run a statement and get plain rows back, with no deadline and no byte
 * accounting.
 *
 * This is for the queries the interface asks on the reader's behalf — schema
 * lookups, the function list behind completion — not for the reader's own,
 * which go through `runQuery` so that they are measured and bounded.
 */
export async function rowsOf(session: Session, sql: string): Promise<Record<string, unknown>[]> {
  const table = await session.query(sql);
  return table.toArray().map((row) => row.toJSON() as Record<string, unknown>);
}

/** Column names and types for whatever the query selects, without reading data. */
export async function describeQuery(
  session: Session,
  sql: string,
): Promise<{ name: string; type: string }[]> {
  const trimmed = sql.trim().replace(/;\s*$/, "");
  if (!trimmed) return [];
  const rows = await rowsOf(session, `DESCRIBE ${trimmed}`);
  return rows.map((row) => ({
    name: String(row.column_name ?? ""),
    type: String(row.column_type ?? ""),
  }));
}
