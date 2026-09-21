#!/usr/bin/env node
// The hosted entry point: streamable HTTP, stateless, always sandboxed. Every
// engine it hands out is locked down, used by one request, then discarded.

import { createServer as createHttpServer, type IncomingMessage, type ServerResponse } from "node:http";
import { homedir } from "node:os";
import { join } from "node:path";
import { Readable } from "node:stream";

import { loadCorpus } from "./corpus.js";
import { createEngine, extensionsFromEnv } from "./duckdb.js";
import { createHttpApp } from "./http-app.js";
import { createEnginePool } from "./pool.js";

function integer(name: string, fallback: number): number {
  const raw = process.env[name];
  if (raw === undefined || raw.trim() === "") return fallback;
  const value = Number(raw);
  if (!Number.isInteger(value) || value < 1) throw new Error(`${name} must be a positive integer, not ${raw}`);
  return value;
}

const port = integer("PORT", 8080);
const extensionDirectory =
  process.env.CITYPARQUET_MCP_EXTENSION_DIR ?? join(homedir(), ".cityparquet-mcp", "extensions");
const extensions = extensionsFromEnv(process.env.CITYPARQUET_MCP_EXTENSIONS);
const memoryLimit = process.env.CITYPARQUET_MCP_MEMORY_LIMIT ?? "1GB";
const threads = integer("CITYPARQUET_MCP_THREADS", 1);

const log = (event: string, detail: Record<string, unknown> = {}) =>
  console.log(JSON.stringify({ time: new Date().toISOString(), event, ...detail }));
const logError = (error: unknown) =>
  log("error", { message: error instanceof Error ? error.message : String(error) });

const pool = createEnginePool({
  size: integer("CITYPARQUET_MCP_POOL_SIZE", 2),
  maxWaiting: integer("CITYPARQUET_MCP_MAX_WAITING", 8),
  maxWaitMs: integer("CITYPARQUET_MCP_MAX_WAIT_MS", 30_000),
  // `sandbox: true` is not configurable here: this entry point is the public one.
  create: () => createEngine({ sandbox: true, extensionDirectory, extensions, memoryLimit, threads }),
  onError: logError,
});

const app = createHttpApp({
  corpus: loadCorpus(),
  pool,
  ceilings: {
    maxRows: integer("CITYPARQUET_MCP_MAX_ROWS", 1000),
    timeoutMs: integer("CITYPARQUET_MCP_MAX_TIMEOUT_MS", 60_000),
  },
  maxBodyBytes: integer("CITYPARQUET_MCP_MAX_BODY_BYTES", 256 * 1024),
  onError: logError,
});

function toRequest(req: IncomingMessage): Request {
  const headers = new Headers();
  for (const [name, value] of Object.entries(req.headers)) {
    if (Array.isArray(value)) for (const item of value) headers.append(name, item);
    else if (value !== undefined) headers.set(name, value);
  }
  const hasBody = req.method !== "GET" && req.method !== "HEAD";
  return new Request(`http://${req.headers.host ?? "localhost"}${req.url ?? "/"}`, {
    method: req.method,
    headers,
    body: hasBody ? (Readable.toWeb(req) as ReadableStream<Uint8Array>) : undefined,
    duplex: "half",
  } as RequestInit);
}

async function send(response: Response, res: ServerResponse): Promise<void> {
  res.statusCode = response.status;
  response.headers.forEach((value, name) => res.setHeader(name, value));
  if (!response.body) {
    res.end();
    return;
  }
  for await (const chunk of response.body as unknown as AsyncIterable<Uint8Array>) res.write(chunk);
  res.end();
}

const server = createHttpServer((req, res) => {
  const started = performance.now();
  app(toRequest(req))
    .then((response) => {
      log("request", { method: req.method, path: req.url, status: response.status, ms: Math.round(performance.now() - started) });
      return send(response, res);
    })
    .catch((error: unknown) => {
      logError(error);
      if (!res.headersSent) res.writeHead(500, { "content-type": "text/plain; charset=utf-8" });
      res.end("internal error");
    });
});

server.listen(port, () => log("listening", { port, extensions }));

// Cloudflare sends SIGTERM before stopping a container: finish what is in
// flight, then free the engines.
for (const signal of ["SIGTERM", "SIGINT"] as const) {
  process.on(signal, () => {
    log("shutdown", { signal });
    server.close(() => void pool.close().finally(() => process.exit(0)));
  });
}
