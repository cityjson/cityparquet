import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { createEngine, type Engine } from "../src/duckdb.js";
import { runQuery } from "../src/tools/query.js";

const extensionDirectory = join(mkdtempSync(join(tmpdir(), "cityparquet-mcp-q-")), "extensions");

describe("runQuery", () => {
  let engine: Engine;
  beforeAll(async () => {
    engine = await createEngine({ sandbox: true, extensionDirectory, memoryLimit: "2GB", threads: 4 });
  });
  afterAll(async () => { await engine?.close(); });

  it("returns columns, types and rows", async () => {
    const [result] = await runQuery(engine, "SELECT 1 AS a, 'x' AS b");
    expect(result!.columns).toEqual([{ name: "a", type: "INTEGER" }, { name: "b", type: "VARCHAR" }]);
    expect(result!.rows).toEqual([[1, "x"]]);
    expect(result!.rowCount).toBe(1);
    expect(result!.truncated).toBe(false);
    expect(typeof result!.elapsedMs).toBe("number");
  });

  it("runs statements one at a time and returns one result each", async () => {
    const results = await runQuery(engine, "SELECT 1 AS a; SELECT 2 AS a");
    expect(results).toHaveLength(2);
    expect(results[1]!.rows).toEqual([[2]]);
  });

  it("stops at the first error and reports it", async () => {
    const results = await runQuery(engine, "SELECT 1; SELECT * FROM nope; SELECT 3");
    expect(results).toHaveLength(2);
    expect(results[1]!.error).toMatch(/nope/);
    expect(results[1]!.rows).toBeUndefined();
  });

  it("caps rows and says so, without reading the rest", async () => {
    const [result] = await runQuery(engine, "SELECT * FROM range(500)", { maxRows: 10 });
    expect(result!.rows).toHaveLength(10);
    expect(result!.rowCount).toBe(10); // rows returned, not rows matched
    expect(result!.truncated).toBe(true);
  });

  it("elides a blob rather than returning it", async () => {
    const [result] = await runQuery(engine, "SELECT 'abcd'::BLOB AS g");
    expect(result!.rows![0]![0]).toBe("<BLOB 4 bytes>");
  });

  it("elides an oversized string", async () => {
    const [result] = await runQuery(engine, "SELECT repeat('x', 5000) AS s", { maxCellBytes: 32 });
    expect(result!.rows![0]![0]).toBe("<VARCHAR 5000 bytes>");
  });

  it("times out without killing the engine", async () => {
    const [result] = await runQuery(engine, "SELECT count(*) FROM range(100000000000)", { timeoutMs: 500 });
    expect(result!.error).toMatch(/timed out/i);
    const [after] = await runQuery(engine, "SELECT 1 AS a");
    expect(after!.rows).toEqual([[1]]);
  });
});
