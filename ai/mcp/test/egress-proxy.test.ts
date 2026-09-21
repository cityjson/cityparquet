import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { mkdtempSync } from "node:fs";
import { createServer, request } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { createEngine, HOSTED_EXTENSIONS, type Engine } from "../src/duckdb.js";
import { isInternalAddress, startEgressProxy, type EgressProxy } from "../src/egress-proxy.js";

// The egress proxy is the hosted server's network boundary: a sandboxed
// engine's http_proxy is locked to it. Like the blocked table in
// duckdb.test.ts, a test here that starts passing the other way is a hole.

const DATA_HOST = "cityparquet.open3d.city";

function connectStatus(proxy: EgressProxy, target: string): Promise<number> {
  const [host, port] = proxy.address.split(":");
  return new Promise((resolve, reject) => {
    const req = request({ host, port: Number(port), method: "CONNECT", path: target });
    req.on("connect", (res, socket) => {
      socket.destroy();
      resolve(res.statusCode ?? 0);
    });
    req.on("error", reject);
    req.end();
  });
}

function getStatus(proxy: EgressProxy, url: string): Promise<number> {
  const [host, port] = proxy.address.split(":");
  return new Promise((resolve, reject) => {
    const req = request({ host, port: Number(port), method: "GET", path: url }, (res) => {
      res.resume();
      resolve(res.statusCode ?? 0);
    });
    req.on("error", reject);
    req.end();
  });
}

describe("isInternalAddress", () => {
  it.each([
    "169.254.169.254", "10.1.2.3", "172.20.0.1", "192.168.1.1", "127.0.0.1", "100.64.0.1", "0.0.0.0",
    "::1", "fe80::1", "fd00::1", "::ffff:169.254.169.254",
  ])("treats %s as internal", (address) => {
    expect(isInternalAddress(address)).toBe(true);
  });

  it.each(["8.8.8.8", "104.21.0.1", "2606:4700::1"])("treats %s as public", (address) => {
    expect(isInternalAddress(address)).toBe(false);
  });
});

describe("the egress proxy", () => {
  let proxy: EgressProxy;
  const refused: string[] = [];
  beforeAll(async () => {
    proxy = await startEgressProxy({ allowedHosts: [DATA_HOST], onRefused: (target) => refused.push(target) });
  });
  afterAll(async () => { await proxy?.close(); });

  it("tunnels HTTPS to an allowlisted host", async () => {
    expect(await connectStatus(proxy, `${DATA_HOST}:443`)).toBe(200);
  });

  it("matches hostnames case-insensitively", async () => {
    expect(await connectStatus(proxy, `CityParquet.Open3D.City:443`)).toBe(200);
  });

  it.each([
    ["a host off the allowlist", "example.com:443"],
    ["an allowlisted host on another port", `${DATA_HOST}:80`],
    ["the metadata address", "169.254.169.254:443"],
    ["an IPv6 literal", "[::1]:443"],
    ["a malformed target", "no-port-here"],
  ])("refuses %s", async (_name, target) => {
    expect(await connectStatus(proxy, target)).toBe(403);
  });

  it("refuses plain HTTP, the metadata server's protocol, whatever the host", async () => {
    expect(await getStatus(proxy, "http://169.254.169.254/computeMetadata/v1/")).toBe(403);
    expect(await getStatus(proxy, `http://${DATA_HOST}/data/delft/metadata.json`)).toBe(403);
    expect(refused).toContain("http://169.254.169.254/computeMetadata/v1/");
  });
});

describe("a sandboxed engine locked to the egress proxy", () => {
  let proxy: EgressProxy;
  let engine: Engine;
  beforeAll(async () => {
    proxy = await startEgressProxy({ allowedHosts: [DATA_HOST] });
    engine = await createEngine({
      sandbox: true,
      extensionDirectory: join(mkdtempSync(join(tmpdir(), "cityparquet-mcp-")), "extensions"),
      extensions: HOSTED_EXTENSIONS,
      httpProxy: proxy.address,
      allowedHosts: [DATA_HOST],
    });
  });
  afterAll(async () => {
    await engine?.close();
    await proxy?.close();
  });

  it("reads an allowlisted host", async () => {
    const reader = await engine.connection.runAndReadAll(
      `SELECT count(*)::INTEGER FROM read_parquet('https://${DATA_HOST}/data/delft/building.parquet')`,
    );
    expect(reader.getRowsJson()).toEqual([[2231]]);
  });

  it.each([
    ["another host", "SELECT * FROM read_text('https://example.com/')"],
    ["the metadata server over plain HTTP", "SELECT * FROM read_text('http://169.254.169.254/computeMetadata/v1/')"],
  ])("cannot reach %s", async (_name, sql) => {
    await expect(engine.connection.run(sql)).rejects.toThrow();
  });

  it("has no GDAL to reach round the proxy with", async () => {
    await expect(engine.connection.run("SELECT * FROM ST_Read('/vsicurl/https://example.com/x.geojson')"))
      .rejects.toThrow(/ST_Read/i);
  });

  it.each([
    ["clearing the proxy", "SET http_proxy = ''"],
    ["resetting the proxy", "RESET http_proxy"],
  ])("refuses %s", async (_name, sql) => {
    await expect(engine.connection.run(sql)).rejects.toThrow(/locked/);
  });
});

// Why the hosted server does not load spatial. GDAL has its own HTTP client:
// /vsicurl/ and /vsicurl_streaming/ fetch directly, outside both
// disabled_filesystems and the locked http_proxy, and a filename can carry its
// own `proxy=` override. Found by probing a sandboxed engine against a local
// server. If this test ever fails, GDAL's networking has changed and spatial
// on the hosted server can be reconsidered — not before.
describe("spatial's GDAL, on a sandboxed engine locked to the egress proxy", () => {
  it("still reaches the network directly, which is why the hosted server does not load it", async () => {
    const hits: string[] = [];
    const victim = createServer((req, res) => {
      hits.push(req.url ?? "");
      res.writeHead(200, { "content-type": "application/json" });
      res.end('{"type":"FeatureCollection","features":[]}');
    });
    await new Promise<void>((resolve) => victim.listen(0, "127.0.0.1", resolve));
    const { port } = victim.address() as { port: number };
    const proxy = await startEgressProxy({ allowedHosts: [DATA_HOST] });
    const engine = await createEngine({
      sandbox: true,
      extensionDirectory: join(mkdtempSync(join(tmpdir(), "cityparquet-mcp-")), "extensions"),
      extensions: ["httpfs", "spatial"],
      httpProxy: proxy.address,
    });
    try {
      await engine.connection.run(`SELECT * FROM ST_Read('/vsicurl_streaming/http://127.0.0.1:${port}/x.geojson')`).catch(() => undefined);
      expect(hits.length).toBeGreaterThan(0);
    } finally {
      await engine.close();
      await proxy.close();
      victim.close();
    }
  });
});
