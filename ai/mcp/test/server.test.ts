import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { DuckDBInstance } from "@duckdb/node-api";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { createServer } from "../src/server.js";
import type { Corpus } from "../src/corpus.js";
import type { Engine } from "../src/duckdb.js";
import { serialiser } from "../src/serialise.js";

const CORPUS: Corpus = {
  generatedFrom: "test",
  corpora: {
    spec: {
      title: "CityParquet specification",
      source: "x",
      chapters: [
        {
          id: "metadata",
          title: "Metadata",
          description: "d",
          order: 0,
          sections: [],
          body: "The footer is authoritative.",
        },
      ],
    },
    "duckdb-cityjson": { title: "y", source: "y", chapters: [] },
    "duckdb-3d": { title: "z", source: "z", chapters: [] },
  },
};

const ENGINE = { connection: {}, extensions: [], async close() {} } as unknown as Engine;

// `McpServer` keeps its registrations on private maps; the public surface is
// the tool/resource list, so every assertion below reaches through them
// rather than re-implementing the protocol. Confirmed against the real
// `@modelcontextprotocol/server@2.0.0` build (`_registeredTools`,
// `_registeredResources`, both keyed as used here) — see the task report for
// how that was checked.
interface ZodLikeSchema {
  safeParse(value: unknown): { success: boolean };
}

interface RegisteredTool {
  readonly inputSchema?: ZodLikeSchema;
  readonly handler?: (args: unknown, extra: unknown) => Promise<unknown>;
}

function registeredTools(server: unknown): Record<string, RegisteredTool> {
  return (server as { _registeredTools: Record<string, RegisteredTool> })._registeredTools;
}

/**
 * Invokes a registered tool's handler directly, in-process — no transport,
 * no client. This is what `server.test.ts` was missing entirely before: every
 * other test here checks registration and schemas but stops short of calling
 * a tool, so an argument-wiring regression (`corpus` and `chapter` swapped in
 * a handler, say) would pass every test in this file while being wrong.
 */
async function invokeTool(server: unknown, name: string, args: unknown): Promise<unknown> {
  const handler = registeredTools(server)[name]?.handler;
  if (!handler) throw new Error(`tool "${name}" has no handler`);
  return handler(args, {});
}

interface CallToolTextResult {
  readonly content: readonly { readonly type: string; readonly text: string }[];
  readonly isError?: boolean;
}

function registeredResources(server: unknown): Record<string, unknown> {
  return (server as { _registeredResources: Record<string, unknown> })._registeredResources;
}

function schemaOf(server: unknown, name: string): ZodLikeSchema {
  const schema = registeredTools(server)[name]?.inputSchema;
  if (!schema) throw new Error(`tool "${name}" has no input schema`);
  return schema;
}

describe("createServer", () => {
  it("registers exactly the five tools", () => {
    const server = createServer({ corpus: CORPUS, engine: ENGINE });
    const names = Object.keys(registeredTools(server));
    expect(names.sort()).toEqual([
      "cityparquet_describe",
      "cityparquet_docs_outline",
      "cityparquet_docs_read",
      "cityparquet_docs_search",
      "cityparquet_query",
    ]);
  });

  it("registers one resource per chapter, addressed as cityparquet://<corpus>/<chapter>", () => {
    const server = createServer({ corpus: CORPUS, engine: ENGINE });
    // The fixture carries one chapter (spec/metadata) and two empty corpora.
    expect(Object.keys(registeredResources(server))).toEqual(["cityparquet://spec/metadata"]);
  });

  describe("the corpus enum is a runtime boundary", () => {
    it("rejects an unknown corpus id on cityparquet_docs_outline", () => {
      const server = createServer({ corpus: CORPUS, engine: ENGINE });
      expect(schemaOf(server, "cityparquet_docs_outline").safeParse({ corpus: "not-a-corpus" }).success).toBe(false);
    });

    it("rejects an unknown corpus id on the required corpus field of cityparquet_docs_read", () => {
      const server = createServer({ corpus: CORPUS, engine: ENGINE });
      expect(
        schemaOf(server, "cityparquet_docs_read").safeParse({ corpus: "not-a-corpus", chapter: "metadata" }).success,
      ).toBe(false);
    });

    it("accepts every real corpus id", () => {
      const server = createServer({ corpus: CORPUS, engine: ENGINE });
      const schema = schemaOf(server, "cityparquet_docs_outline");
      for (const id of ["spec", "duckdb-cityjson", "duckdb-3d"]) {
        expect(schema.safeParse({ corpus: id }).success).toBe(true);
      }
    });
  });

  describe("cityparquet_query's bounds are enforced before runQuery ever sees the input", () => {
    it("accepts values at and inside the documented bounds", () => {
      const server = createServer({ corpus: CORPUS, engine: ENGINE });
      const schema = schemaOf(server, "cityparquet_query");
      for (const options of [
        { sql: "select 1", max_rows: 1, max_cell_bytes: 16, timeout_ms: 1000 },
        { sql: "select 1", max_rows: 5000, max_cell_bytes: 65536, timeout_ms: 600_000 },
        { sql: "select 1" },
      ]) {
        expect(schema.safeParse(options).success).toBe(true);
      }
    });

    it("rejects max_rows outside [1, 5000]", () => {
      const server = createServer({ corpus: CORPUS, engine: ENGINE });
      const schema = schemaOf(server, "cityparquet_query");
      expect(schema.safeParse({ sql: "select 1", max_rows: 0 }).success).toBe(false);
      expect(schema.safeParse({ sql: "select 1", max_rows: 5001 }).success).toBe(false);
    });

    it("rejects max_cell_bytes outside [16, 65536]", () => {
      const server = createServer({ corpus: CORPUS, engine: ENGINE });
      const schema = schemaOf(server, "cityparquet_query");
      expect(schema.safeParse({ sql: "select 1", max_cell_bytes: 15 }).success).toBe(false);
      expect(schema.safeParse({ sql: "select 1", max_cell_bytes: 65537 }).success).toBe(false);
    });

    it("rejects timeout_ms outside [1000, 600000]", () => {
      const server = createServer({ corpus: CORPUS, engine: ENGINE });
      const schema = schemaOf(server, "cityparquet_query");
      expect(schema.safeParse({ sql: "select 1", timeout_ms: 999 }).success).toBe(false);
      expect(schema.safeParse({ sql: "select 1", timeout_ms: 600_001 }).success).toBe(false);
    });
  });
});

describe("every tool, actually invoked", () => {
  let engine: Engine;
  let file: string;

  beforeAll(async () => {
    // A bare DuckDB connection, no network: `cityparquet_describe` and
    // `cityparquet_query` need nothing but core SQL for this round trip.
    const instance = await DuckDBInstance.create(":memory:");
    const connection = await instance.connect();
    engine = {
      connection,
      extensions: [],
      exclusive: serialiser(),
      async close() {
        connection.closeSync();
      },
    };

    const dir = mkdtempSync(join(tmpdir(), "cityparquet-mcp-server-fixture-"));
    file = join(dir, "building.parquet");
    await engine.connection.run(`COPY (SELECT 1 AS id) TO '${file}' (FORMAT PARQUET)`);
  });
  afterAll(async () => { await engine?.close(); });

  function textOf(result: unknown): string {
    return (result as CallToolTextResult).content[0]!.text;
  }

  it("cityparquet_docs_outline lists every corpus", async () => {
    const result = await invokeTool(createServer({ corpus: CORPUS, engine: ENGINE }), "cityparquet_docs_outline", {});
    const parsed = JSON.parse(textOf(result)) as { corpora: { id: string }[] };
    expect(parsed.corpora.map((c) => c.id)).toEqual(["spec", "duckdb-cityjson", "duckdb-3d"]);
  });

  it("cityparquet_docs_search finds the fixture chapter by its body text", async () => {
    const result = await invokeTool(createServer({ corpus: CORPUS, engine: ENGINE }), "cityparquet_docs_search", {
      query: "authoritative",
    });
    const parsed = JSON.parse(textOf(result)) as { chapter: string }[];
    expect(parsed.map((h) => h.chapter)).toEqual(["metadata"]);
  });

  it("cityparquet_docs_read reads the named chapter, not some other one", async () => {
    // The regression this guards: `corpus` and `chapter` swapped inside the
    // handler would still type-check (both are strings) and still return
    // *some* text — only checking the actual content catches it.
    const result = await invokeTool(createServer({ corpus: CORPUS, engine: ENGINE }), "cityparquet_docs_read", {
      corpus: "spec",
      chapter: "metadata",
    });
    expect(textOf(result)).toBe("The footer is authoritative.");
  });

  it("cityparquet_describe reads the real fixture file's footer", async () => {
    const result = await invokeTool(createServer({ corpus: CORPUS, engine }), "cityparquet_describe", { url: file });
    const parsed = JSON.parse(textOf(result)) as { kind: string; tables: { row_count: number | null }[] };
    expect(parsed.kind).toBe("file");
    expect(parsed.tables[0]!.row_count).toBe(1);
  });

  it("cityparquet_query runs the given SQL and returns the given row", async () => {
    const result = await invokeTool(createServer({ corpus: CORPUS, engine }), "cityparquet_query", {
      sql: "SELECT 41 + 1 AS answer",
    });
    const parsed = JSON.parse(textOf(result)) as { rows: number[][]; row_count: number }[];
    expect(parsed[0]!.rows).toEqual([[42]]);
    expect(parsed[0]!.row_count).toBe(1);
  });
});
