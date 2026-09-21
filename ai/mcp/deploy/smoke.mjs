#!/usr/bin/env node
// Checks a deployed endpoint: node smoke.mjs https://cityparquet-mcp.<subdomain>.workers.dev
//
// The egress cases are the point. The container's network policy is set in
// src/index.ts and cannot be exercised anywhere but on Cloudflare, so this is
// where the sandbox's last control — no route to private ranges or to hosts
// off the allowlist — is shown to hold, from outside, after every deploy.

const base = process.argv[2]?.replace(/\/+$/, "");
if (!base) {
  console.error("usage: node smoke.mjs <deployment URL>");
  process.exit(2);
}

const HEADERS = { "content-type": "application/json", accept: "application/json, text/event-stream" };
let id = 0;

async function rpc(name, args) {
  const response = await fetch(`${base}/mcp`, {
    method: "POST",
    headers: HEADERS,
    body: JSON.stringify({ jsonrpc: "2.0", id: ++id, method: "tools/call", params: { name, arguments: args } }),
  });
  const text = await response.text();
  if (response.status !== 200) throw new Error(`HTTP ${response.status}: ${text.slice(0, 300)}`);
  const json = text.startsWith("event:") || text.startsWith("data:")
    ? text.split("\n").filter((line) => line.startsWith("data:")).map((line) => line.slice(5)).join("")
    : text;
  return JSON.parse(JSON.parse(json).result.content[0].text);
}

const query = async (sql) => (await rpc("cityparquet_query", { sql }))[0];

// The first request after a deploy waits for the image to be provisioned and
// the container to start; give it minutes, not seconds.
async function waitForHealth() {
  const deadline = Date.now() + 10 * 60_000;
  for (;;) {
    try {
      const response = await fetch(`${base}/health`);
      if (response.ok && (await response.text()) === "ok") return;
    } catch {}
    if (Date.now() > deadline) throw new Error("the container did not become healthy within ten minutes");
    await new Promise((resolve) => setTimeout(resolve, 10_000));
  }
}

const checks = [
  ["serves the documentation", async () => {
    const hits = await rpc("cityparquet_docs_search", { query: "feature_id", limit: 1 });
    if (!Array.isArray(hits) || hits.length === 0) throw new Error("no search results");
  }],
  ["reads an allowlisted data host", async () => {
    const result = await query("SELECT count(*)::INTEGER AS n FROM read_parquet('https://cityparquet.open3d.city/data/delft/building.parquet')");
    if (result.error || result.rows[0][0] !== 2231) throw new Error(JSON.stringify(result));
  }],
  ["has cityjson, three_d and spatial", async () => {
    const result = await query("SELECT count(DISTINCT lower(function_name))::INTEGER FROM duckdb_functions() WHERE lower(function_name) IN ('read_cityjsonseq', 'st_3dvolume', 'st_area')");
    if (result.error || result.rows[0][0] !== 3) throw new Error(JSON.stringify(result));
  }],
  ["cannot read a host off the allowlist", async () => {
    const result = await query("SELECT * FROM read_csv('https://example.com/')");
    if (!result.error) throw new Error("a non-allowlisted host was readable");
  }],
  ["cannot reach the cloud metadata address", async () => {
    const result = await query("SELECT * FROM read_csv('http://169.254.169.254/latest/meta-data/')");
    if (!result.error) throw new Error("169.254.169.254 was readable");
  }],
  ["cannot read the local filesystem", async () => {
    const result = await query("SELECT * FROM read_csv('/etc/passwd')");
    if (!result.error) throw new Error("/etc/passwd was readable");
  }],
];

await waitForHealth();
let failed = 0;
for (const [name, check] of checks) {
  try {
    await check();
    console.log(`ok   ${name}`);
  } catch (error) {
    failed += 1;
    console.log(`FAIL ${name}: ${error instanceof Error ? error.message : String(error)}`);
  }
}
process.exit(failed === 0 ? 0 : 1);
