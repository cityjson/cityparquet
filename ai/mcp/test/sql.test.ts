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
  it("always elides a blob, reporting its length", () => {
    expect(elideCell("abcd", "BLOB", 1024)).toBe("<BLOB 4 bytes>");
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
