// Registration, and nothing else. Every tool here is a thin call into a module
// that is tested without a transport.
//
// The `cityparquet_` prefix is a phase-1 choice, not a commitment (spec §10):
// a later merge with a sibling CityJSON MCP server may adopt one neutral
// prefix shared by both, so do not lean on this prefix elsewhere.

import { McpServer } from "@modelcontextprotocol/server";
import * as z from "zod/v4";

import { CORPUS_IDS, outline, readChapter, search, type Corpus, type CorpusId } from "./corpus.js";
import type { Engine } from "./duckdb.js";
import { describe } from "./tools/describe.js";
import { QUERY_DEFAULTS, runQuery } from "./tools/query.js";

/**
 * Upper bounds on what one `cityparquet_query` call may ask for. A local
 * server leaves them at the schema's own maxima; the hosted one lowers them,
 * because its engines are shared out one request at a time.
 */
export interface QueryCeilings {
  readonly maxRows: number;
  readonly timeoutMs: number;
}

export interface ServerDeps {
  readonly corpus: Corpus;
  readonly engine: Engine;
  readonly ceilings?: QueryCeilings;
}

const corpusEnum = z.enum(CORPUS_IDS as unknown as [CorpusId, ...CorpusId[]]);

const json = (value: unknown) => ({ content: [{ type: "text" as const, text: JSON.stringify(value, null, 2) }] });
const text = (value: string) => ({ content: [{ type: "text" as const, text: value }] });
const failure = (error: unknown) => ({
  content: [{ type: "text" as const, text: error instanceof Error ? error.message : String(error) }],
  isError: true,
});

export function createServer({ corpus, engine, ceilings }: ServerDeps): McpServer {
  const server = new McpServer({ name: "cityparquet", version: "0.1.0" });

  server.registerTool(
    "cityparquet_docs_outline",
    {
      description:
        "List the chapters of the CityParquet documentation. Corpora: 'spec' (the normative specification and its design decisions), 'duckdb-cityjson' and 'duckdb-3d' (the DuckDB extension function references). Call this first to learn what can be read; omit `corpus` to see all three.",
      inputSchema: z.object({ corpus: corpusEnum.optional().describe("Restrict to one corpus") }),
    },
    async ({ corpus: id }) => json(outline(corpus, id)),
  );

  server.registerTool(
    "cityparquet_docs_search",
    {
      description:
        "Search the CityParquet documentation for a term and return matching sections with snippets. Faster than reading chapters when you are looking for a specific column, function or rule.",
      inputSchema: z.object({
        query: z.string().describe("Term to look for, e.g. 'semantic surfaces' or 'ST_3DVolume'"),
        corpus: corpusEnum.optional(),
        limit: z.number().int().min(1).max(50).optional(),
      }),
    },
    async ({ query, corpus: id, limit }) => json(search(corpus, query, { corpus: id, limit })),
  );

  server.registerTool(
    "cityparquet_docs_read",
    {
      description:
        "Read one chapter of the CityParquet documentation, or one section of it. Get chapter ids from cityparquet_docs_outline or cityparquet_docs_search.",
      inputSchema: z.object({
        corpus: corpusEnum,
        chapter: z.string().describe("Chapter id, e.g. 'object-table-schema'"),
        section: z.string().optional().describe("Read only this section of the chapter"),
      }),
    },
    async ({ corpus: id, chapter, section }) => {
      try {
        return text(readChapter(corpus, id, chapter, section));
      } catch (error) {
        return failure(error);
      }
    },
  );

  server.registerTool(
    "cityparquet_describe",
    {
      description:
        "Describe a CityParquet dataset: its module tables, row counts, LoDs, geometry columns and CRS. Accepts a package directory or a single .parquet file, as an http(s) URL — or, unless the server is sandboxed, a local path or file:// URL. Read `notes`: they say where the file list came from and flag tables whose CRSs disagree. Call this before querying an unfamiliar dataset.",
      inputSchema: z.object({ url: z.string().describe("Package directory or .parquet file: URL or local path") }),
    },
    async ({ url }) => {
      try {
        return json(await describe(engine, url));
      } catch (error) {
        return failure(error);
      }
    },
  );

  server.registerTool(
    "cityparquet_query",
    {
      description:
        `Run SQL against DuckDB with the cityjson and three_d extensions loaded, and spatial on a local server (the hosted one does not load it). Solids (LoD1 and up) need three_d — ST_3DVolume, ST_3DFootprintArea, ST_3DTransform — because spatial cannot read them; where spatial is loaded, its ST_Area and ST_Transform work on the LoD0 column. Check that a function exists with duckdb_functions() before relying on it. A script is split and its statements run one at a time. BLOB columns and oversized values are elided, so SELECT * on an object table is a poor idea — select the columns you need instead. Results are capped at ${QUERY_DEFAULTS.maxRows} rows by default.`,
      inputSchema: z.object({
        sql: z.string().describe("One or more SQL statements, separated by semicolons"),
        max_rows: z.number().int().min(1).max(5000).optional(),
        max_cell_bytes: z.number().int().min(16).max(65536).optional(),
        timeout_ms: z.number().int().min(1000).max(600_000).optional(),
      }),
    },
    async ({ sql, max_rows, max_cell_bytes, timeout_ms }) => {
      try {
        return json(
          await runQuery(engine, sql, {
            maxRows: Math.min(max_rows ?? QUERY_DEFAULTS.maxRows, ceilings?.maxRows ?? Infinity),
            maxCellBytes: max_cell_bytes ?? QUERY_DEFAULTS.maxCellBytes,
            timeoutMs: Math.min(timeout_ms ?? QUERY_DEFAULTS.timeoutMs, ceilings?.timeoutMs ?? Infinity),
          }),
        );
      } catch (error) {
        return failure(error);
      }
    },
  );

  // The chapters again, as resources. Client support is uneven, so nothing may
  // depend on these — the tools are the contract.
  for (const id of CORPUS_IDS) {
    for (const chapter of corpus.corpora[id].chapters) {
      server.registerResource(
        `${id}/${chapter.id}`,
        `cityparquet://${id}/${chapter.id}`,
        { description: chapter.description || chapter.title, mimeType: "text/markdown" },
        async (uri) => ({ contents: [{ uri: uri.href, text: chapter.body }] }),
      );
    }
  }

  return server;
}
