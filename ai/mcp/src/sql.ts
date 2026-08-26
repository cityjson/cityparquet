// Splitting a script into statements, and keeping a result from flooding the
// caller's context.

/**
 * DuckDB expands every pragma in a submitted script before running any of it,
 * so the package workflow — `CREATE SCHEMA d;` then `PRAGMA cityparquet_init('d')`
 * — fails outright when submitted as one script. Statements therefore run one
 * at a time, which means splitting them here.
 *
 * `DuckDBConnection.extractStatements` cannot do this: it resolves pragmas at
 * extract time and throws on the very scripts that need splitting.
 */
export function splitStatements(sql: string): string[] {
  const statements: string[] = [];
  let start = 0;
  let i = 0;

  while (i < sql.length) {
    const ch = sql[i]!;

    if (ch === "'" || ch === '"') {
      const quote = ch;
      i += 1;
      while (i < sql.length) {
        if (sql[i] === quote) {
          if (sql[i + 1] === quote) i += 2; // a doubled quote is an escaped one
          else {
            i += 1;
            break;
          }
        } else i += 1;
      }
      continue;
    }

    if (ch === "$") {
      const tag = /^\$[A-Za-z_]*\$/.exec(sql.slice(i));
      if (tag) {
        const marker = tag[0];
        const end = sql.indexOf(marker, i + marker.length);
        i = end === -1 ? sql.length : end + marker.length;
        continue;
      }
    }

    if (ch === "-" && sql[i + 1] === "-") {
      const end = sql.indexOf("\n", i);
      i = end === -1 ? sql.length : end;
      continue;
    }

    if (ch === "/" && sql[i + 1] === "*") {
      const end = sql.indexOf("*/", i + 2);
      i = end === -1 ? sql.length : end + 2;
      continue;
    }

    if (ch === ";") {
      const statement = sql.slice(start, i).trim();
      if (statement) statements.push(statement);
      i += 1;
      start = i;
      continue;
    }

    i += 1;
  }

  const tail = sql.slice(start).trim();
  if (tail) statements.push(tail);
  return statements;
}

/**
 * A single LoD2 solid's WKB runs to kilobytes, and `SELECT *` on an object
 * table returns one per row — so an un-elided result is at its most destructive
 * on the most obvious query anyone will write. The `other` overflow column and
 * long attribute strings are the same bomb by a different route, which is why
 * this is not blob-only.
 *
 * The type is passed in rather than sniffed: `getRowsJson()` renders a BLOB as
 * an ordinary string, so the value alone cannot tell you what it was.
 */
export function elideCell(value: unknown, type: string, maxBytes: number): unknown {
  if (value === null || value === undefined) return value;

  if (type.toUpperCase().startsWith("BLOB")) {
    const length =
      typeof value === "string" ? Buffer.byteLength(value, "utf8") : String(value).length;
    return `<BLOB ${length} bytes>`;
  }

  if (typeof value === "number" || typeof value === "boolean") return value;

  const text = typeof value === "string" ? value : JSON.stringify(value);
  const length = Buffer.byteLength(text, "utf8");
  return length > maxBytes ? `<${type} ${length} bytes>` : value;
}
