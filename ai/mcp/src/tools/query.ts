// Running a caller's SQL, one statement at a time, and shaping what comes back.

import { DuckDBBlobValue } from "@duckdb/node-api";

import type { Engine } from "../duckdb.js";
import { elideCell, splitStatements } from "../sql.js";

export interface QueryOptions {
  readonly maxRows: number;
  readonly maxCellBytes: number;
  readonly timeoutMs: number;
}

export const QUERY_DEFAULTS: QueryOptions = {
  maxRows: 100,
  maxCellBytes: 256,
  timeoutMs: 120_000,
};

export interface StatementResult {
  readonly statement: string;
  readonly columns?: { name: string; type: string }[];
  readonly rows?: unknown[][];
  /** Rows returned, not rows matched — see `truncated`. */
  readonly row_count?: number;
  readonly truncated?: boolean;
  readonly elapsed_ms?: number;
  readonly error?: string;
}

export async function runQuery(
  engine: Engine,
  sql: string,
  options: Partial<QueryOptions> = {},
): Promise<StatementResult[]> {
  const { maxRows, maxCellBytes, timeoutMs } = { ...QUERY_DEFAULTS, ...options };
  const results: StatementResult[] = [];

  for (const statement of splitStatements(sql)) {
    // Reassigned once execution actually starts, inside the critical section
    // below — not here. A statement can queue behind another tool call's
    // turn on the shared connection first, and that wait is not this
    // statement's execution time: counting it would let `elapsed_ms` report
    // tens of seconds on a query that in fact ran, and finished, in
    // milliseconds once it got the connection.
    let started = performance.now();
    // Set inside the timer callback: true if and only if the timer fired,
    // which is exactly the question "did this statement time out?" — and,
    // unlike comparing elapsed wall-clock time against `timeoutMs`, immune to
    // clock behaviour (a wall-clock step, or a sub-millisecond-early fire at
    // a tight threshold) that could otherwise make a genuine timeout look
    // like an ordinary error.
    let timedOut = false;
    try {
      // The whole statement — including the timer that may call
      // `connection.interrupt()` — runs inside one critical section. All
      // five tools share this connection, and an MCP client pipelines tool
      // calls: without `exclusive`, a timeout here could interrupt a
      // concurrently running `describe` or another `query` instead of this
      // statement. Serialising means whichever statement is running when the
      // timer fires is, guaranteed, this one.
      const { names, types, fetched, typed } = await engine.exclusive(async () => {
        started = performance.now();
        // `interrupt` is what makes the deadline recoverable: the statement is
        // cancelled inside the engine rather than abandoned, so the connection is
        // still usable afterwards.
        const timer = setTimeout(() => {
          timedOut = true;
          engine.connection.interrupt();
        }, timeoutMs);
        try {
          // One row past the cap, never `runAndReadAll`. Reading everything and
          // slicing afterwards would materialise the whole result in Node's heap
          // first — and DuckDB's memory_limit does not govern the JS side, so
          // `SELECT * FROM read_cityjsonseq(<big>)` would exhaust the process
          // despite a 100-row cap. Reading cap+1 is what makes the cap real.
          const reader = await engine.connection.runAndReadUntil(statement, maxRows + 1);
          return {
            names: reader.columnNames(),
            types: reader.columnTypes().map((t) => String(t)),
            fetched: reader.getRowsJson(),
            // The typed path, read alongside the display path above rather
            // than instead of it: `getRows()` is what exposes a BLOB's real
            // `byteLength` (see `elideCell`'s doc comment), but its BIGINT
            // values come back as JS `bigint`, which `elideCell`'s
            // `JSON.stringify` fallback throws on for any other type — the
            // very reason `getRowsJson()` was chosen for display in the
            // first place. Reading both, and using the typed rows for
            // nothing but a BLOB's byte length, gets the true count without
            // that hazard.
            typed: reader.getRows(),
          };
        } finally {
          clearTimeout(timer);
        }
      });

      const truncated = fetched.length > maxRows;
      const rows = fetched.slice(0, maxRows).map((row, rowIndex) =>
        row.map((value, index) => {
          const type = types[index] ?? "VARCHAR";
          const cell = typed[rowIndex]?.[index];
          const blobByteLength =
            type.toUpperCase().startsWith("BLOB") && cell instanceof DuckDBBlobValue
              ? cell.bytes.byteLength
              : undefined;
          return elideCell(value, type, maxCellBytes, blobByteLength);
        }),
      );

      results.push({
        statement,
        columns: names.map((name, index) => ({ name, type: types[index] ?? "VARCHAR" })),
        rows,
        // Rows returned, not rows matched. The exact total is unknowable
        // without reading the whole result, which is precisely what this
        // avoids — `truncated` says there are more. Do not "fix" this back to
        // a total; a caller who needs one should SELECT count(*).
        row_count: rows.length,
        truncated,
        elapsed_ms: Math.round(performance.now() - started),
      });
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      results.push({
        statement,
        error: timedOut ? `timed out after ${timeoutMs} ms: ${message}` : message,
        elapsed_ms: Math.round(performance.now() - started),
      });
      break; // a script's later statements almost always depend on its earlier ones
    }
  }

  return results;
}
