import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { createEngine, DEFAULT_EXTENSIONS, type Engine } from "../src/duckdb.js";

const extensionDirectory = join(mkdtempSync(join(tmpdir(), "cityparquet-mcp-")), "extensions");

describe("createEngine", () => {
  let engine: Engine;
  beforeAll(async () => {
    engine = await createEngine({ sandbox: true, extensionDirectory, memoryLimit: "2GB", threads: 4 });
  });
  afterAll(async () => { await engine?.close(); });

  it("loads exactly the default extensions, and never spatial", () => {
    expect(engine.extensions.map((e) => e.name).sort()).toEqual([...DEFAULT_EXTENSIONS].sort());
    expect(engine.extensions.map((e) => e.name)).not.toContain("spatial");
  });

  it("runs DuckDB v1.5.4", async () => {
    const reader = await engine.connection.runAndReadAll("SELECT version() AS v");
    expect(reader.getRowsJson()[0]![0]).toBe("v1.5.4");
  });

  // The security contract. A change that makes one of these pass is a
  // regression, not a test failure.
  const blocked = [
    ["local csv read", "SELECT * FROM read_csv('/etc/passwd')"],
    ["local attach", "ATTACH '/tmp/mcp-probe.db'"],
    ["local copy out", "COPY (SELECT 1) TO '/tmp/mcp-probe.parquet'"],
    ["local parquet read", "SELECT * FROM read_parquet('/etc/hostname')"],
    ["extension install", "INSTALL json"],
    ["unlocking the filesystem", "SET disabled_filesystems = ''"],
    ["raising the memory limit", "SET memory_limit = '400GB'"],
    ["unlocking the configuration", "SET lock_configuration = false"],
  ] as const;

  for (const [name, sql] of blocked) {
    it(`blocks ${name}`, async () => {
      await expect(engine.connection.run(sql)).rejects.toThrow();
    });
  }

  // Not a hole. `json` is statically linked into the DuckDB binary, so loading
  // it reads no file. The property the sandbox provides is that only extensions
  // already in the binary can be loaded — every other one needs a disk read.
  it("permits LOAD of a statically linked extension", async () => {
    await expect(engine.connection.run("LOAD json")).resolves.toBeDefined();
  });

  it("still reads over HTTPS", async () => {
    const reader = await engine.connection.runAndReadAll(
      "SELECT version FROM cityjsonseq_metadata('https://cityjson.open3d.city/cityjsonseq/delft.city.jsonl')",
    );
    expect(reader.getRowsJson()[0]![0]).toBe("2.0");
  });
});

describe("createEngine without the sandbox", () => {
  it("leaves the local filesystem reachable", async () => {
    const engine = await createEngine({ sandbox: false, extensionDirectory });
    await expect(engine.connection.run("SELECT * FROM read_csv('/etc/hostname')")).resolves.toBeDefined();
    await engine.close();
  });
});
