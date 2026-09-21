import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { loadCorpus } from "../src/corpus.js";
import { createEngine, type Engine } from "../src/duckdb.js";
import { createHttpApp } from "../src/http-app.js";
import { createEnginePool, type EnginePool } from "../src/pool.js";

// The hosted server end to end, minus the socket: real MCP messages through
// createMcpHandler, real sandboxed engines from a real pool.

const extensionDirectory = join(mkdtempSync(join(tmpdir(), "cityparquet-mcp-")), "extensions");
const ORIGIN = "http://mcp.test";
const HEADERS = { "content-type": "application/json", accept: "application/json, text/event-stream" };

let id = 0;
function rpc(method: string, params: unknown): Request {
  return new Request(`${ORIGIN}/mcp`, {
    method: "POST",
    headers: HEADERS,
    body: JSON.stringify({ jsonrpc: "2.0", id: ++id, method, params }),
  });
}

/**
 * A 2025-era request is answered as a short SSE stream (`event: message` then
 * `data: {…}`), a modern one as plain JSON; both carry one JSON-RPC message.
 */
async function message<T>(response: Response): Promise<T> {
  const text = await response.text();
  if (!text.startsWith("event:") && !text.startsWith("data:")) return JSON.parse(text) as T;
  const data = text.split("\n").filter((line) => line.startsWith("data:")).map((line) => line.slice(5)).join("");
  return JSON.parse(data) as T;
}

async function call(app: (r: Request) => Promise<Response>, name: string, args: unknown) {
  const response = await app(rpc("tools/call", { name, arguments: args }));
  expect(response.status).toBe(200);
  const reply = await message<{ result: { content: { text: string }[]; isError?: boolean } }>(response);
  return { text: reply.result.content[0]!.text, isError: reply.result.isError === true };
}

describe("the hosted HTTP app", () => {
  let pool: EnginePool;
  let created = 0;
  let app: (request: Request) => Promise<Response>;

  beforeAll(async () => {
    const create = async (): Promise<Engine> => {
      created += 1;
      return createEngine({ sandbox: true, extensionDirectory, memoryLimit: "1GB", threads: 2 });
    };
    // Warm the extension directory once, so the pool's builds are disk loads.
    await (await create()).close();
    pool = createEnginePool({ size: 2, maxWaiting: 4, maxWaitMs: 60_000, create });
    app = createHttpApp({
      corpus: loadCorpus(),
      pool,
      ceilings: { maxRows: 10, timeoutMs: 30_000 },
      maxBodyBytes: 64 * 1024,
    });
  });
  afterAll(async () => { await pool?.close(); });

  it("answers the health check", async () => {
    const response = await app(new Request(`${ORIGIN}/health`));
    expect(response.status).toBe(200);
    expect(await response.text()).toBe("ok");
  });

  it("answers initialize", async () => {
    const response = await app(
      rpc("initialize", { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "test", version: "0" } }),
    );
    expect(response.status).toBe(200);
    const reply = await message<{ result: { serverInfo: { name: string } } }>(response);
    expect(reply.result.serverInfo.name).toBe("cityparquet");
  });

  it("serves the documentation tools without building an engine", async () => {
    const before = created;
    const { text } = await call(app, "cityparquet_docs_search", { query: "feature_id", limit: 1 });
    expect(text).toMatch(/feature_id/);
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(created).toBe(before);
  });

  // One function from each extension. Not duckdb_extensions(): it reads the
  // extension directory on disk, which the sandbox has closed.
  it("runs SQL with cityjson, three_d and spatial loaded", async () => {
    const { text } = await call(app, "cityparquet_query", {
      sql: `SELECT count(DISTINCT lower(function_name))::INTEGER AS n FROM duckdb_functions()
            WHERE lower(function_name) IN ('read_cityjsonseq', 'st_3dvolume', 'st_area')`,
    });
    expect(JSON.parse(text)[0].rows).toEqual([[3]]);
  });

  it("does not let one request see what another created", async () => {
    await call(app, "cityparquet_query", { sql: "CREATE TABLE left_behind AS SELECT 42 AS v" });
    const { text } = await call(app, "cityparquet_query", { sql: "SELECT v FROM left_behind" });
    expect(JSON.parse(text)[0].error).toMatch(/left_behind/);
  });

  it("keeps the local filesystem closed", async () => {
    const { text } = await call(app, "cityparquet_query", { sql: "SELECT * FROM read_csv('/etc/passwd')" });
    expect(JSON.parse(text)[0].error).toMatch(/disabled/i);
    const described = await call(app, "cityparquet_describe", { url: "/etc" });
    expect(described.isError).toBe(true);
    expect(described.text).toMatch(/sandboxed/);
  });

  it("holds max_rows to the ceiling whatever the caller asks for", async () => {
    const { text } = await call(app, "cityparquet_query", { sql: "SELECT * FROM range(100)", max_rows: 5000 });
    const [result] = JSON.parse(text);
    expect(result.row_count).toBe(10);
    expect(result.truncated).toBe(true);
  });

  it("describes a remote package", async () => {
    const { text } = await call(app, "cityparquet_describe", { url: "https://cityparquet.open3d.city/data/delft" });
    expect(JSON.parse(text).crs).toMatch(/EPSG:7415/);
  });

  it("refuses an oversized body", async () => {
    const response = await app(
      new Request(`${ORIGIN}/mcp`, { method: "POST", headers: HEADERS, body: "x".repeat(65 * 1024) }),
    );
    expect(response.status).toBe(413);
  });

  it("answers 404 off the MCP path", async () => {
    expect((await app(new Request(`${ORIGIN}/elsewhere`))).status).toBe(404);
  });
});

describe("the hosted HTTP app at capacity", () => {
  it("answers 503 with Retry-After once the queue is full", async () => {
    let release!: () => void;
    const held = new Promise<void>((resolve) => { release = resolve; });
    const slow: Engine = {
      sandbox: true,
      extensions: [],
      connection: {} as Engine["connection"],
      exclusive: async <T>(task: () => Promise<T>) => { await held; return task(); },
      async close() {},
    };
    const pool = createEnginePool({ size: 1, maxWaiting: 0, maxWaitMs: 1000, create: async () => slow });
    const app = createHttpApp({ corpus: loadCorpus(), pool, ceilings: { maxRows: 10, timeoutMs: 1000 }, maxBodyBytes: 65536 });
    const first = app(rpc("tools/call", { name: "cityparquet_query", arguments: { sql: "SELECT 1" } }));
    await new Promise((resolve) => setTimeout(resolve, 50));
    const second = await app(rpc("tools/call", { name: "cityparquet_query", arguments: { sql: "SELECT 1" } }));
    expect(second.status).toBe(503);
    expect(second.headers.get("retry-after")).toBe("2");
    release();
    await first.catch(() => undefined);
    await pool.close();
  });
});
