// Running a caller's SQL, one statement at a time, and shaping what comes back.

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
  readonly rowCount?: number;
  readonly truncated?: boolean;
  readonly elapsedMs?: number;
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
    const started = Date.now();
    try {
      // `interrupt` is what makes the deadline recoverable: the statement is
      // cancelled inside the engine rather than abandoned, so the connection is
      // still usable afterwards.
      const timer = setTimeout(() => engine.connection.interrupt(), timeoutMs);
      let reader;
      try {
        // One row past the cap, never `runAndReadAll`. Reading everything and
        // slicing afterwards would materialise the whole result in Node's heap
        // first — and DuckDB's memory_limit does not govern the JS side, so
        // `SELECT * FROM read_cityjsonseq(<big>)` would exhaust the process
        // despite a 100-row cap. Reading cap+1 is what makes the cap real.
        reader = await engine.connection.runAndReadUntil(statement, maxRows + 1);
      } finally {
        clearTimeout(timer);
      }

      const names = reader.columnNames();
      const types = reader.columnTypes().map((t) => String(t));
      const fetched = reader.getRowsJson();
      const truncated = fetched.length > maxRows;
      const rows = fetched.slice(0, maxRows).map((row) =>
        row.map((value, index) => elideCell(value, types[index] ?? "VARCHAR", maxCellBytes)),
      );

      results.push({
        statement,
        columns: names.map((name, index) => ({ name, type: types[index] ?? "VARCHAR" })),
        rows,
        // Rows returned, not rows matched. The exact total is unknowable
        // without reading the whole result, which is precisely what this
        // avoids — `truncated` says there are more. Do not "fix" this back to
        // a total; a caller who needs one should SELECT count(*).
        rowCount: rows.length,
        truncated,
        elapsedMs: Date.now() - started,
      });
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      const elapsed = Date.now() - started;
      results.push({
        statement,
        error:
          elapsed >= timeoutMs
            ? `timed out after ${timeoutMs} ms: ${message}`
            : message,
        elapsedMs: elapsed,
      });
      break; // a script's later statements almost always depend on its earlier ones
    }
  }

  return results;
}
