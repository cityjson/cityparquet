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
    expect(result!.row_count).toBe(1);
    expect(result!.truncated).toBe(false);
    expect(typeof result!.elapsed_ms).toBe("number");
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
    expect(result!.row_count).toBe(10); // rows returned, not rows matched
    expect(result!.truncated).toBe(true);
  });

  it("elides a blob rather than returning it, with its true byte length", async () => {
    const [result] = await runQuery(engine, "SELECT 'abcd'::BLOB AS g");
    expect(result!.rows![0]![0]).toBe("<BLOB 4 bytes>");
  });

  it("reports a blob's true byte length, not the length of its escape-rendered display string", async () => {
    // 0x01, 0x02 and 0xFF are non-printable and each render as a 4-character
    // `\xNN` escape when DuckDB converts a BLOB to a string — 3 bytes render
    // as 12 characters, plus 3 printable bytes ('a','b','c') as themselves,
    // for a 15-character rendered string from a 6-byte blob. Real WKB is
    // mostly non-printable bytes like these, which is exactly why the naive
    // "length of the string" approach inflated the reported size up to 4×.
    const [result] = await runQuery(engine, "SELECT '\\x01\\x02\\xFFabc'::BLOB AS g");
    expect(result!.rows![0]![0]).toBe("<BLOB 6 bytes>");
  });

  it("elides an oversized string", async () => {
    const [result] = await runQuery(engine, "SELECT repeat('x', 5000) AS s", { maxCellBytes: 32 });
    expect(result!.rows![0]![0]).toBe("<VARCHAR 5000 bytes>");
  });

  it("does not mark an exact-boundary result truncated", async () => {
    const [result] = await runQuery(engine, "SELECT * FROM range(10)", { maxRows: 10 });
    expect(result!.rows).toHaveLength(10);
    expect(result!.row_count).toBe(10);
    expect(result!.truncated).toBe(false);
  });

  it("times out without killing the engine", async () => {
    const [result] = await runQuery(engine, "SELECT count(*) FROM range(100000000000)", { timeoutMs: 500 });
    expect(result!.error).toMatch(/timed out/i);
    const [after] = await runQuery(engine, "SELECT 1 AS a");
    expect(after!.rows).toEqual([[1]]);
  });

  it("does not let a concurrent timeout interrupt a different statement", async () => {
    // Two tool calls sharing one connection, pipelined by the client exactly
    // as an MCP client would: the slow one is timed out and interrupted, and
    // that must not touch the fast one issued alongside it. Before
    // serialisation, `connection.interrupt()` was connection-global and
    // could cancel whichever statement happened to be executing — here, the
    // fast one — mislabelling its result as a timeout that was never its own.
    const [[slow], [fast]] = await Promise.all([
      runQuery(engine, "SELECT count(*) FROM range(100000000000)", { timeoutMs: 200 }),
      runQuery(engine, "SELECT 42 AS a"),
    ]);
    expect(slow!.error).toMatch(/timed out/i);
    expect(fast!.error).toBeUndefined();
    expect(fast!.rows).toEqual([[42]]);
  });
});

// A query-created secret's HTTP_PROXY overrides the engine's locked
// http_proxy, and its EXTRA_HTTP_HEADERS can carry the header the cloud
// metadata server wants — so a sandboxed engine allows no secrets at all.
describe("runQuery on a sandboxed engine, and secrets", () => {
  let engine: Engine;
  beforeAll(async () => {
    engine = await createEngine({ sandbox: true, extensionDirectory });
  });
  afterAll(async () => { await engine?.close(); });

  it.each([
    ["a proxy override", "CREATE SECRET s (TYPE http, HTTP_PROXY '127.0.0.1:9')"],
    ["a metadata header", "CREATE SECRET s (TYPE http, EXTRA_HTTP_HEADERS MAP {'Metadata-Flavor': 'Google'})"],
    ["a temporary secret in lower case", "create temporary secret s (type http)"],
    ["the word inside a string", "SELECT 'secret' AS x"],
  ])("refuses %s before running it", async (_name, sql) => {
    const results = await runQuery(engine, `SELECT 1; ${sql}; SELECT 3`);
    expect(results).toHaveLength(2);
    expect(results[1]!.error).toMatch(/secret/i);
    // Checked on the raw connection: runQuery itself refuses the word.
    const reader = await engine.connection.runAndReadAll("SELECT count(*)::INTEGER FROM duckdb_secrets()");
    expect(reader.getRowsJson()).toEqual([[0]]);
  });

  it("stops, and drops the secret, if one exists by any route", async () => {
    // Created behind runQuery's back: the check after each statement is the
    // backstop for a route the statement filter has not thought of.
    await engine.connection.run("CREATE SECRET planted (TYPE http, HTTP_PROXY '127.0.0.1:9')");
    const results = await runQuery(engine, "SELECT 1; SELECT 2");
    expect(results).toHaveLength(1);
    expect(results[0]!.error).toMatch(/secret/i);
    const [next] = await runQuery(engine, "SELECT 1");
    expect(next!.error).toBeUndefined(); // the engine is usable once the secret is gone
    const reader = await engine.connection.runAndReadAll("SELECT count(*)::INTEGER FROM duckdb_secrets()");
    expect(reader.getRowsJson()).toEqual([[0]]);
  });
});

describe("runQuery on an unsandboxed engine, and secrets", () => {
  it("allows them: a local user's own credentials are theirs to use", async () => {
    const engine = await createEngine({ sandbox: false, extensionDirectory });
    const [result] = await runQuery(engine, "CREATE SECRET mine (TYPE http, BEARER_TOKEN 'x')");
    expect(result!.error).toBeUndefined();
    await engine.close();
  });
});
