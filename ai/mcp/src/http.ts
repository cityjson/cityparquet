#!/usr/bin/env node
// The hosted entry point: streamable HTTP, stateless, always sandboxed. Every
// engine it hands out is locked down, used by one request, then discarded.

import { createServer as createHttpServer, type IncomingMessage, type ServerResponse } from "node:http";
import { homedir } from "node:os";
import { join } from "node:path";

import { loadCorpus } from "./corpus.js";
import { createEngine, extensionsFromEnv } from "./duckdb.js";
import { startEgressProxy } from "./egress-proxy.js";
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

/**
 * The only hosts a query can read. HTTPS only, exact names. The proxy that
 * enforces this runs in this process, and every engine's `http_proxy` is
 * locked to it; see src/egress-proxy.ts for why it is not the platform's job.
 */
const egressHosts = (process.env.CITYPARQUET_MCP_EGRESS_HOSTS ?? "cityparquet.open3d.city,cityjson.open3d.city,flatcitybuf.open3d.city")
  .split(",")
  .map((host) => host.trim().toLowerCase())
  .filter((host) => host.length > 0);

const log = (event: string, detail: Record<string, unknown> = {}) =>
  console.log(JSON.stringify({ time: new Date().toISOString(), event, ...detail }));
const logError = (error: unknown) =>
  log("error", { message: error instanceof Error ? error.message : String(error) });

const egress = await startEgressProxy({
  allowedHosts: egressHosts,
  onRefused: (target, reason) => log("egress refused", { target, reason }),
});

const pool = createEnginePool({
  size: integer("CITYPARQUET_MCP_POOL_SIZE", 2),
  maxWaiting: integer("CITYPARQUET_MCP_MAX_WAITING", 8),
  maxWaitMs: integer("CITYPARQUET_MCP_MAX_WAIT_MS", 30_000),
  // `sandbox: true` is not configurable here: this entry point is the public one.
  create: () =>
    createEngine({
      sandbox: true,
      extensionDirectory,
      extensions,
      memoryLimit,
      threads,
      httpProxy: egress.address,
      allowedHosts: egressHosts,
    }),
  onError: logError,
});

const maxBodyBytes = integer("CITYPARQUET_MCP_MAX_BODY_BYTES", 256 * 1024);

const app = createHttpApp({
  corpus: loadCorpus(),
  pool,
  ceilings: {
    maxRows: integer("CITYPARQUET_MCP_MAX_ROWS", 1000),
    timeoutMs: integer("CITYPARQUET_MCP_MAX_TIMEOUT_MS", 60_000),
  },
  maxBodyBytes,
  onError: logError,
});

class BodyTooLarge extends Error {}

/**
 * The whole body, read before any engine is leased — so a slow upload holds a
 * socket, never an engine — and refused past the cap as it arrives, whatever
 * `content-length` claimed.
 */
async function readBody(req: IncomingMessage): Promise<Buffer> {
  const chunks: Buffer[] = [];
  let total = 0;
  for await (const chunk of req as AsyncIterable<Buffer>) {
    total += chunk.byteLength;
    if (total > maxBodyBytes) throw new BodyTooLarge();
    chunks.push(chunk);
  }
  return Buffer.concat(chunks);
}

async function toRequest(req: IncomingMessage): Promise<Request> {
  const headers = new Headers();
  for (const [name, value] of Object.entries(req.headers)) {
    if (Array.isArray(value)) for (const item of value) headers.append(name, item);
    else if (value !== undefined) headers.set(name, value);
  }
  const hasBody = req.method !== "GET" && req.method !== "HEAD";
  return new Request(`http://${req.headers.host ?? "localhost"}${req.url ?? "/"}`, {
    method: req.method,
    headers,
    body: hasBody ? new Uint8Array(await readBody(req)) : undefined,
  });
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

function fail(res: ServerResponse, status: number, text: string): void {
  if (res.headersSent) {
    res.destroy();
    return;
  }
  // Closing the connection after a refusal: the rest of an oversized body is
  // not worth reading, and the response must be flushed before the socket goes.
  res.writeHead(status, { "content-type": "text/plain; charset=utf-8", connection: "close" });
  res.end(text);
}

const server = createHttpServer((req, res) => {
  const started = performance.now();
  // Everything inside the promise chain: a method or URL the Request
  // constructor rejects (TRACE, a malformed Host) must be a 400, not an
  // exception that takes the process down.
  Promise.resolve()
    .then(() => toRequest(req))
    .then(
      (request) =>
        app(request).then((response) => {
          log("request", { method: req.method, path: req.url, status: response.status, ms: Math.round(performance.now() - started) });
          return send(response, res);
        }),
      (error: unknown) => {
        if (error instanceof BodyTooLarge) fail(res, 413, `request body over ${maxBodyBytes} bytes`);
        else fail(res, 400, "bad request");
      },
    )
    .catch((error: unknown) => {
      logError(error);
      fail(res, 500, "internal error");
    });
});

server.listen(port, () => log("listening", { port, extensions, egressHosts }));

// Cloudflare sends SIGTERM before stopping a container: finish what is in
// flight, then free the engines.
for (const signal of ["SIGTERM", "SIGINT"] as const) {
  process.on(signal, () => {
    log("shutdown", { signal });
    server.close(() => void pool.close().then(() => egress.close()).finally(() => process.exit(0)));
  });
}
