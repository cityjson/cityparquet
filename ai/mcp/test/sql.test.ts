import { describe, expect, it } from "vitest";

import { elideCell, splitStatements } from "../src/sql.js";

describe("splitStatements", () => {
  it("splits on semicolons", () => {
    expect(splitStatements("SELECT 1; SELECT 2")).toEqual(["SELECT 1", "SELECT 2"]);
  });

  it("ignores a semicolon inside a single-quoted string", () => {
    expect(splitStatements("SELECT 'a;b'")).toEqual(["SELECT 'a;b'"]);
  });

  it("handles the doubled quotes in a cityparquet_delete predicate", () => {
    const sql = "PRAGMA cityparquet_delete('delft', 'object_type = ''Building''');\nSELECT 1";
    expect(splitStatements(sql)).toEqual([
      "PRAGMA cityparquet_delete('delft', 'object_type = ''Building''')",
      "SELECT 1",
    ]);
  });

  it("ignores a semicolon inside a double-quoted identifier", () => {
    expect(splitStatements('SELECT 1 AS "a;b"')).toEqual(['SELECT 1 AS "a;b"']);
  });

  it("ignores a semicolon inside a dollar-quoted string", () => {
    expect(splitStatements("SELECT $$a;b$$")).toEqual(["SELECT $$a;b$$"]);
  });

  it("ignores a semicolon in a line comment", () => {
    expect(splitStatements("SELECT 1 -- a;b\n; SELECT 2")).toEqual(["SELECT 1 -- a;b", "SELECT 2"]);
  });

  it("ignores a semicolon in a block comment", () => {
    expect(splitStatements("SELECT 1 /* a;b */; SELECT 2")).toEqual([
      "SELECT 1 /* a;b */",
      "SELECT 2",
    ]);
  });

  it("drops empty statements and trailing semicolons", () => {
    expect(splitStatements("SELECT 1;;\n  \n")).toEqual(["SELECT 1"]);
  });
});

describe("elideCell", () => {
  it("always elides a blob, reporting its true length when given one", () => {
    expect(elideCell("abcd", "BLOB", 1024, 4)).toBe("<BLOB 4 bytes>");
  });

  it("reports the byte length passed in, not the length of the rendered string", () => {
    // getRowsJson() would render a 6-byte blob (0x01, 0x02, 0xFF, 'a', 'b',
    // 'c') as this 15-character escaped string — each non-printable byte
    // becomes a 4-character `\xNN` sequence. Deriving the count from the
    // string, as the previous implementation did, would report 15 bytes for
    // a 6-byte blob. The true count must come from the caller, not the value.
    const rendered = "\\x01\\x02\\xFFabc";
    expect(rendered).toHaveLength(15);
    expect(elideCell(rendered, "BLOB", 1024, 6)).toBe("<BLOB 6 bytes>");
  });

  it("reports no number when the true length is not available, rather than a guess", () => {
    expect(elideCell("\\x01\\x02\\xFFabc", "BLOB", 1024)).toBe("<BLOB>");
  });

  it("truncates an oversized string", () => {
    expect(elideCell("x".repeat(50), "VARCHAR", 10)).toBe("<VARCHAR 50 bytes>");
  });

  it("leaves a small value alone", () => {
    expect(elideCell("short", "VARCHAR", 256)).toBe("short");
    expect(elideCell(42, "INTEGER", 256)).toBe(42);
    expect(elideCell(null, "VARCHAR", 256)).toBeNull();
  });

  it("elides an oversized nested value by its serialised size", () => {
    const big = { a: "y".repeat(100) };
    expect(elideCell(big, "STRUCT(a VARCHAR)", 20)).toMatch(/^<STRUCT\(a VARCHAR\) \d+ bytes>$/);
  });
});
