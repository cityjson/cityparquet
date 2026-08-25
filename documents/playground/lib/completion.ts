// What the editor offers while you type: the columns of the files the statement
// reads, the functions DuckDB and the loaded extensions expose, and the fields
// inside a STRUCT.
//
// `@codemirror/lang-sql` can complete a schema, but only one declared up front
// as `{table: [columns]}`. Here there are no table names to declare: a source is
// `read_parquet('https://…/building.parquet')`, it changes with the text, and
// its columns are only knowable by asking DuckDB. So the schema is discovered
// from whatever the document currently reads, and cached per expression.
//
// Nothing in this file imports DuckDB. It needs one thing from a session — a
// way to run a statement and get rows back — which keeps it testable without a
// WebAssembly engine in the room.

import { syntaxTree } from "@codemirror/language";
import type {
  Completion,
  CompletionContext,
  CompletionResult,
  CompletionSource,
} from "@codemirror/autocomplete";

export interface ColumnInfo {
  readonly name: string;
  readonly type: string;
}

export interface FunctionInfo {
  readonly name: string;
  /** `scalar`, `aggregate`, `table`, `macro`, `pragma`. */
  readonly kind: string;
  readonly returns: string | null;
  readonly description: string | null;
}

/** Run a statement and get its rows — all this module needs from a session. */
export type RunSql = (sql: string) => Promise<Record<string, unknown>[]>;

/**
 * How long the completion source will wait for a `DESCRIBE` before offering
 * what it already has.
 *
 * The first one on the 16.4 GB file transfers a 4.7 MB footer, and it shares a
 * connection with the query the reader may have running, so it can be slow for
 * reasons that have nothing to do with the popup. Columns missing from one
 * keystroke is a far smaller cost than a popup that hangs.
 */
const DESCRIBE_WAIT_MS = 1_500;

/** Syntax nodes where a suggestion would only be in the way. */
const NO_COMPLETION = new Set(["String", "LineComment", "BlockComment", "QuotedIdentifier"]);

// ── Reading the document ────────────────────────────────────────────────────

/**
 * The table-function calls a statement reads from — `read_parquet('…')`,
 * `read_cityjsonseq('…')`, `cityjsonseq_metadata('…')`.
 *
 * Anchored to `FROM` and `JOIN` rather than matching any call with a string
 * argument, so a `strftime(…)` in a projection is not mistaken for a source.
 * A bare name after `FROM` is skipped deliberately: it is a CTE or an alias,
 * and describing it out of context would fail.
 */
export function tableExpressions(sql: string): string[] {
  const found: string[] = [];
  for (const [, call] of sql.matchAll(/\b(?:from|join)\s+([a-z_][a-z0-9_]*\s*\([^()]*\))/gi)) {
    const normalised = call.replace(/\s+/g, " ").trim();
    if (!found.includes(normalised)) found.push(normalised);
  }
  return found;
}

/**
 * The fields of a DuckDB `STRUCT` type, or none when the type is not a struct.
 *
 * The types this meets are real ones from CityParquet, so the parser has to
 * cope with what they contain: quoted field names (`STRUCT("type" VARCHAR, …)`,
 * because `type` is a keyword), nested lists (`INTEGER[][]`), and structs inside
 * structs. Splitting on commas would break on all three.
 */
export function structFields(type: string): ColumnInfo[] {
  const body = type.trim();
  const open = body.indexOf("(");
  if (open < 0 || !/^STRUCT\s*$/i.test(body.slice(0, open))) return [];
  const close = closingParen(body, open);
  // Anything after the closing paren makes this a list of structs, not a
  // struct: `address` cannot be reached into, only `address[1]` can.
  if (close < 0 || body.slice(close + 1).trim() !== "") return [];

  const fields: ColumnInfo[] = [];
  for (const part of splitTopLevel(body.slice(open + 1, close))) {
    const field = parseField(part);
    if (field) fields.push(field);
  }
  return fields;
}

/** Strip one level of list from a type: `STRUCT(…)[]` is a list of structs. */
export function elementType(type: string): string {
  const trimmed = type.trim();
  return trimmed.endsWith("[]") ? trimmed.slice(0, -2).trim() : trimmed;
}

function closingParen(text: string, open: number): number {
  let depth = 0;
  let quoted = false;
  for (let i = open; i < text.length; i++) {
    const char = text[i];
    if (quoted) {
      if (char === '"') quoted = false;
      continue;
    }
    if (char === '"') quoted = true;
    else if (char === "(") depth++;
    else if (char === ")" && --depth === 0) return i;
  }
  return -1;
}

/** Split on commas that are not inside parentheses or a quoted identifier. */
function splitTopLevel(text: string): string[] {
  const parts: string[] = [];
  let depth = 0;
  let quoted = false;
  let start = 0;
  for (let i = 0; i < text.length; i++) {
    const char = text[i];
    if (quoted) {
      if (char === '"') quoted = false;
      continue;
    }
    if (char === '"') quoted = true;
    else if (char === "(") depth++;
    else if (char === ")") depth--;
    else if (char === "," && depth === 0) {
      parts.push(text.slice(start, i));
      start = i + 1;
    }
  }
  parts.push(text.slice(start));
  return parts.map((part) => part.trim()).filter(Boolean);
}

/** One `name type` pair, where the name may be a quoted identifier. */
function parseField(text: string): ColumnInfo | null {
  if (text.startsWith('"')) {
    const end = text.indexOf('"', 1);
    if (end < 0) return null;
    // `""` is an escaped quote inside a quoted identifier.
    return { name: text.slice(1, end).replace(/""/g, '"'), type: text.slice(end + 1).trim() };
  }
  const split = text.indexOf(" ");
  if (split < 0) return null;
  return { name: text.slice(0, split), type: text.slice(split + 1).trim() };
}

// ── Asking DuckDB ───────────────────────────────────────────────────────────

/**
 * The columns and functions the editor completes against, fetched once each and
 * kept.
 *
 * Every lookup is memoised by the exact text that produced it, failures
 * included: a half-typed URL produces an expression that will never describe,
 * and retrying it on each keystroke would be a request per character.
 */
export class SchemaCache {
  private readonly run: RunSql;
  private readonly columnsByExpression = new Map<string, Promise<ColumnInfo[]>>();
  private functionList: Promise<FunctionInfo[]> | null = null;

  constructor(run: RunSql) {
    this.run = run;
  }

  /** Start fetching whatever this statement reads, without waiting for it. */
  warm(sql: string): void {
    void this.functions();
    for (const expression of tableExpressions(sql)) void this.describe(expression);
  }

  /** The columns of every source the statement reads, first occurrence winning. */
  async columns(sql: string): Promise<ColumnInfo[]> {
    const lists = await Promise.all(tableExpressions(sql).map((e) => this.describe(e)));
    const seen = new Set<string>();
    const columns: ColumnInfo[] = [];
    for (const column of lists.flat()) {
      if (seen.has(column.name)) continue;
      seen.add(column.name);
      columns.push(column);
    }
    return columns;
  }

  /**
   * Every function the engine knows, including the ones `cityjson` and `three_d`
   * added when they loaded. This is the half of the schema a reader cannot guess
   * and cannot look up without leaving the page.
   */
  functions(): Promise<FunctionInfo[]> {
    // Grouped in SQL rather than here: `duckdb_functions()` lists one row per
    // overload, and `arg_max` alone has ninety-two of them. The name filter
    // drops the operators — `~~`, `||`, `!~~` — which are functions to DuckDB
    // but not something anyone completes.
    this.functionList ??= this.run(
      `SELECT function_name AS name,
              min(function_type) AS kind,
              max(return_type)   AS returns,
              max(description)   AS description
       FROM duckdb_functions()
       WHERE regexp_matches(function_name, '^[a-zA-Z_][a-zA-Z0-9_]*$')
       GROUP BY function_name
       ORDER BY function_name`,
    )
      .then((rows) =>
        rows.map((row) => ({
          name: String(row.name ?? ""),
          kind: String(row.kind ?? ""),
          returns: row.returns == null ? null : String(row.returns),
          description: row.description == null ? null : String(row.description),
        })),
      )
      .catch(() => []);
    return this.functionList;
  }

  private describe(expression: string): Promise<ColumnInfo[]> {
    let pending = this.columnsByExpression.get(expression);
    if (pending) return pending;
    // `LIMIT 0` is not needed and not used: DESCRIBE plans the statement and
    // reads the Parquet footer, never the row groups.
    pending = this.run(`DESCRIBE SELECT * FROM ${expression}`)
      .then((rows) =>
        rows.map((row) => ({
          name: String(row.column_name ?? ""),
          type: String(row.column_type ?? ""),
        })),
      )
      .catch(() => []);
    this.columnsByExpression.set(expression, pending);
    return pending;
  }
}

// ── The completion source ───────────────────────────────────────────────────

/**
 * Column names, struct fields and function names, in that order of usefulness.
 *
 * Keywords are not here: `sql()` already contributes them through the language's
 * own completion source, and CodeMirror merges the two.
 */
export function cityParquetCompletion(cache: SchemaCache): CompletionSource {
  return async (context: CompletionContext): Promise<CompletionResult | null> => {
    if (inNoCompletionZone(context)) return null;

    const sql = context.state.doc.toString();
    const columns = await withDeadline(cache.columns(sql), [] as ColumnInfo[]);

    const path = structPathBefore(context);
    if (path) {
      const column = columns.find((candidate) => candidate.name === path.name);
      if (!column) return null;
      // `address` is a list of structs, so `address.street` is not a thing but
      // `address[1].street` is. Unwrap only when the text actually indexed it.
      const type = path.indexed ? elementType(column.type) : column.type;
      const fields = structFields(type);
      if (fields.length === 0) return null;
      return {
        from: path.from,
        options: fields.map((field) => ({
          label: field.name,
          type: "property",
          detail: shortType(field.type),
        })),
        validFor: /^\w*$/,
      };
    }

    const word = context.matchBefore(/\w+/);
    // Without this, a space would open a list of nine hundred functions.
    if (!word && !context.explicit) return null;

    const functions = await withDeadline(cache.functions(), [] as FunctionInfo[]);
    const options: Completion[] = [
      ...columns.map((column, index) => ({
        label: column.name,
        type: "property",
        detail: shortType(column.type),
        // Columns before functions when both match, and in file order among
        // themselves — `id`, `object_type`, `bbox` are what a reader wants
        // first, and alphabetical would bury them.
        boost: 1,
        sortText: String(index).padStart(4, "0"),
      })),
      ...functions.map((fn) => ({
        label: fn.name,
        type: fn.kind === "table" ? "class" : "function",
        detail: shortType(fn.returns ?? fn.kind),
        ...(fn.description ? { info: fn.description } : {}),
      })),
    ];

    return {
      from: word ? word.from : context.pos,
      options,
      validFor: /^\w*$/,
    };
  };
}

/** Inside a string, a comment or a quoted identifier, offer nothing. */
function inNoCompletionZone(context: CompletionContext): boolean {
  const node = syntaxTree(context.state).resolveInner(context.pos, -1);
  for (let scan: typeof node | null = node; scan; scan = scan.parent) {
    if (NO_COMPLETION.has(scan.name)) return true;
  }
  return false;
}

/** `bbox.` or `address[1].xm` — the struct being reached into, and from where. */
function structPathBefore(
  context: CompletionContext,
): { name: string; indexed: boolean; from: number } | null {
  const before = context.matchBefore(/[A-Za-z_]\w*(\[\s*\d+\s*\])?\s*\.\s*\w*/);
  if (!before) return null;
  const match = before.text.match(/^([A-Za-z_]\w*)(\[\s*\d+\s*\])?\s*\.\s*(\w*)$/);
  if (!match) return null;
  return { name: match[1], indexed: Boolean(match[2]), from: context.pos - match[3].length };
}

/** Resolve, or give up and offer what is already known. */
function withDeadline<T>(promise: Promise<T>, fallback: T): Promise<T> {
  return Promise.race([
    promise,
    new Promise<T>((resolve) => setTimeout(() => resolve(fallback), DESCRIBE_WAIT_MS)),
  ]);
}

/** The `detail` column is a hint, not the type — the long ones are unreadable. */
export function shortType(type: string): string {
  if (type.length <= 22) return type;
  const head = type.match(/^[A-Z_]+/)?.[0];
  return head ? `${head}…` : `${type.slice(0, 21)}…`;
}
