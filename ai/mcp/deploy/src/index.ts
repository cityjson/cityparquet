// The Worker in front of the hosted MCP server. DuckDB's Node binding is
// native code, so the server runs as a Cloudflare Container; this Worker
// routes requests to it and sets the container's network policy.

import { Container, getRandom } from "@cloudflare/containers";

// Required for allowedHosts and deniedHosts to take effect.
export { ContainerProxy } from "@cloudflare/containers";

interface Env {
  readonly MCP: DurableObjectNamespace<CityParquetMcp>;
}

/**
 * Everything the hosted engine may reach. The query tool is public and
 * unauthenticated, and DuckDB's httpfs will fetch any URL a caller names, so
 * egress is an allowlist: the data hosts below and nothing else. A read of
 * any other host fails at the platform, below DuckDB, which no redirect or
 * DNS trick inside a query can get round. Widening it is a code change and a
 * redeploy, on purpose.
 */
export const DATA_HOSTS = ["cityparquet.open3d.city", "cityjson.open3d.city", "flatcitybuf.open3d.city"];

/**
 * Private, loopback and link-local ranges, cloud metadata among them.
 * Defence in depth only: with internet off and an allowlist in place these
 * are already unreachable, and whether Cloudflare matches a denied CIDR
 * against a hostname's resolved address is not documented.
 */
const PRIVATE_RANGES = [
  "10.0.0.0/8",
  "172.16.0.0/12",
  "192.168.0.0/16",
  "169.254.0.0/16",
  "127.0.0.0/8",
  "100.64.0.0/10",
  "fc00::/7",
  "fe80::/10",
  "::1/128",
];

/** Fixed: Containers do not autoscale. Hosted concurrency is this times the pool size. */
const INSTANCES = 2;

export class CityParquetMcp extends Container<Env> {
  defaultPort = 8080;
  // Cold starts take a few seconds; ten idle minutes keeps a session warm
  // between an agent's calls without paying for an idle day.
  sleepAfter = "10m";
  pingEndpoint = "localhost/health";
  enableInternet = false;
  allowedHosts = DATA_HOSTS;
  deniedHosts = PRIVATE_RANGES;
  // standard-1 is 4 GiB: two engines at 1200 MB each leave room for Node.
  envVars = {
    CITYPARQUET_MCP_POOL_SIZE: "2",
    CITYPARQUET_MCP_MEMORY_LIMIT: "1200MB",
    CITYPARQUET_MCP_THREADS: "1",
  };
}

// MCP clients that run in a browser need CORS; the endpoint holds no
// credentials, so any origin may call it.
const CORS = {
  "access-control-allow-origin": "*",
  "access-control-allow-methods": "GET, POST, DELETE, OPTIONS",
  "access-control-allow-headers": "content-type, accept, authorization, mcp-protocol-version, mcp-session-id, last-event-id",
  "access-control-expose-headers": "mcp-session-id, mcp-protocol-version",
};

function withCors(response: Response): Response {
  const headers = new Headers(response.headers);
  for (const [name, value] of Object.entries(CORS)) headers.set(name, value);
  return new Response(response.body, { status: response.status, statusText: response.statusText, headers });
}

const ABOUT = `CityParquet MCP server — streamable HTTP at /mcp.

Read-only: SQL runs in a sandboxed DuckDB with cityjson, three_d and spatial,
and can read data from ${DATA_HOSTS.join(", ")} only.

Setup and tools: https://cityjson.github.io/cityparquet/ai/mcp-server/
Source: https://github.com/cityjson/cityparquet/tree/main/ai/mcp
`;

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const { pathname } = new URL(request.url);
    if (request.method === "OPTIONS") return new Response(null, { status: 204, headers: CORS });
    if (pathname === "/") return withCors(new Response(ABOUT, { headers: { "content-type": "text/plain; charset=utf-8" } }));
    if (pathname === "/mcp" || pathname === "/health") {
      const container = await getRandom(env.MCP, INSTANCES);
      return withCors(await container.fetch(request));
    }
    return withCors(new Response("not found — the MCP endpoint is /mcp", { status: 404 }));
  },
} satisfies ExportedHandler<Env>;
