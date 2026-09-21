// The hosted server's request handling, as a web-standard fetch handler so it
// is tested without a socket. `http.ts` puts it behind node:http.

import { createMcpHandler } from "@modelcontextprotocol/server";

import type { Corpus } from "./corpus.js";
import type { Engine } from "./duckdb.js";
import { PoolBusyError, type EnginePool } from "./pool.js";
import { createServer, type QueryCeilings } from "./server.js";

/** The tools that touch DuckDB. Every other request is served without an engine. */
const ENGINE_TOOLS = new Set(["cityparquet_describe", "cityparquet_query"]);

export interface HttpAppOptions {
  readonly corpus: Corpus;
  readonly pool: EnginePool;
  readonly ceilings: QueryCeilings;
  /** A JSON-RPC body larger than this is refused with 413. SQL scripts are small. */
  readonly maxBodyBytes: number;
  readonly onError?: (error: unknown) => void;
}

/**
 * Stands in for an engine on requests that never call a DuckDB tool —
 * `initialize`, `tools/list`, the documentation tools. Leasing a real one
 * for those would cost an engine build per request for nothing. If a code
 * path ever does reach it, it fails loudly rather than silently.
 */
const NO_ENGINE: Engine = {
  sandbox: true,
  extensions: [],
  get connection(): never {
    throw new Error("internal error: this request was served without an engine");
  },
  exclusive: () => Promise.reject(new Error("internal error: this request was served without an engine")),
  async close() {},
};

function needsEngine(body: unknown): boolean {
  const messages = Array.isArray(body) ? body : [body];
  return messages.some((message) => {
    if (message === null || typeof message !== "object") return false;
    const { method, params } = message as { method?: unknown; params?: { name?: unknown } };
    return method === "tools/call" && typeof params?.name === "string" && ENGINE_TOOLS.has(params.name);
  });
}

async function readCapped(request: Request, limit: number): Promise<Uint8Array | null> {
  const declared = Number(request.headers.get("content-length") ?? "0");
  if (declared > limit) return null;
  if (!request.body) return new Uint8Array();
  const chunks: Uint8Array[] = [];
  let total = 0;
  const reader = request.body.getReader();
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    total += value.byteLength;
    if (total > limit) {
      await reader.cancel();
      return null;
    }
    chunks.push(value);
  }
  const body = new Uint8Array(total);
  let offset = 0;
  for (const chunk of chunks) {
    body.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return body;
}

const plain = (status: number, text: string, headers: Record<string, string> = {}) =>
  new Response(text, { status, headers: { "content-type": "text/plain; charset=utf-8", ...headers } });

export function createHttpApp(options: HttpAppOptions): (request: Request) => Promise<Response> {
  async function serveMcp(request: Request, body: Uint8Array | null, engine: Engine): Promise<Response> {
    // One handler, and so one McpServer, per request: the deployment is
    // stateless, and the engine behind it is this request's alone.
    const handler = createMcpHandler(
      () => createServer({ corpus: options.corpus, engine, ceilings: options.ceilings }),
      // JSON for modern (2026-07-28) exchanges: no tool emits anything
      // before its result. 2025-era requests go through the SDK's stateless
      // fallback, which answers with a short SSE stream that ends after the
      // one result. Either way the whole body is read below before the
      // engine is released, so no stream outlives its engine.
      { responseMode: "json", keepAliveMs: 0, onerror: options.onError },
    );
    try {
      const response = await handler.fetch(
        new Request(request.url, { method: request.method, headers: request.headers, body: body as BodyInit | null }),
      );
      const payload = await response.arrayBuffer();
      return new Response(payload, { status: response.status, headers: response.headers });
    } finally {
      await handler.close();
    }
  }

  return async (request: Request): Promise<Response> => {
    const { pathname } = new URL(request.url);

    if (pathname === "/health") {
      return request.method === "GET" ? plain(200, "ok") : plain(405, "method not allowed");
    }
    if (pathname !== "/mcp") return plain(404, "not found — the MCP endpoint is /mcp");

    if (request.method !== "POST") {
      // Stateless: there is no session to open a stream on or to delete.
      return serveMcp(request, null, NO_ENGINE);
    }

    const body = await readCapped(request, options.maxBodyBytes);
    if (body === null) return plain(413, `request body over ${options.maxBodyBytes} bytes`);

    let parsed: unknown = null;
    try {
      parsed = JSON.parse(new TextDecoder().decode(body));
    } catch {
      // Not JSON: the MCP handler answers it with the protocol's own error.
    }
    if (!needsEngine(parsed)) return serveMcp(request, body, NO_ENGINE);

    try {
      return await options.pool.use((engine) => serveMcp(request, body, engine));
    } catch (error) {
      if (error instanceof PoolBusyError) return plain(503, error.message, { "retry-after": "2" });
      throw error;
    }
  };
}
