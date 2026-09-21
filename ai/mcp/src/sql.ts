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

    // An E-string (`E'…'`) takes backslash escapes, so `E'it\'s'` is one
    // string; an ordinary one does not. The prefix counts only when it
    // starts a token — `name'…'` is an identifier followed by a string.
    if ((ch === "e" || ch === "E") && sql[i + 1] === "'" && !/[A-Za-z0-9_$]/.test(sql[i - 1] ?? "")) {
      i += 2;
      while (i < sql.length) {
        if (sql[i] === "\\") i += 2;
        else if (sql[i] === "'") {
          if (sql[i + 1] === "'") i += 2;
          else {
            i += 1;
            break;
          }
        } else i += 1;
      }
      continue;
    }

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

    // A tag may carry digits after its first character (`$t1$`), but may not
    // start with one: `$1` is a positional parameter, not a quote.
    if (ch === "$") {
      const tag = /^\$(?:[A-Za-z_][A-Za-z0-9_]*)?\$/.exec(sql.slice(i));
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

    // Block comments nest, as DuckDB's own lexer has them: the first `*/`
    // inside `/* a /* b */ c */` closes only the inner one.
    if (ch === "/" && sql[i + 1] === "*") {
      let depth = 1;
      i += 2;
      while (i < sql.length && depth > 0) {
        if (sql[i] === "/" && sql[i + 1] === "*") {
          depth += 1;
          i += 2;
        } else if (sql[i] === "*" && sql[i + 1] === "/") {
          depth -= 1;
          i += 2;
        } else i += 1;
      }
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
 *
 * `blobByteLength` is the true byte count of a BLOB cell, from the typed
 * `getRows()` path — never derive it from `value`. `getRowsJson()` renders a
 * BLOB by escaping every non-printable byte to `\xNN` (four characters for
 * one byte), so the escaped string's length is not the blob's length: real
 * WKB is mostly non-printable, and a length read off the string inflates up
 * to 4×. When the caller cannot cheaply obtain the true count, omitting the
 * number entirely (`<BLOB>`) is the honest answer — never a guess dressed up
 * as a measurement.
 */
export function elideCell(
  value: unknown,
  type: string,
  maxBytes: number,
  blobByteLength?: number,
): unknown {
  if (value === null || value === undefined) return value;

  if (type.toUpperCase().startsWith("BLOB")) {
    return blobByteLength === undefined ? "<BLOB>" : `<BLOB ${blobByteLength} bytes>`;
  }

  if (typeof value === "number" || typeof value === "boolean") return value;

  const text = typeof value === "string" ? value : JSON.stringify(value);
  const length = Buffer.byteLength(text, "utf8");
  return length > maxBytes ? `<${type} ${length} bytes>` : value;
}
