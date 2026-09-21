import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { createEngine, DEFAULT_EXTENSIONS, extensionsFromEnv, type Engine } from "../src/duckdb.js";

const extensionDirectory = join(mkdtempSync(join(tmpdir(), "cityparquet-mcp-")), "extensions");

describe("createEngine", () => {
  let engine: Engine;
  beforeAll(async () => {
    engine = await createEngine({ sandbox: true, extensionDirectory, memoryLimit: "2GB", threads: 4 });
  });
  afterAll(async () => { await engine?.close(); });

  it("loads exactly the default extensions, spatial and three_d together", () => {
    expect(engine.extensions.map((e) => e.name).sort()).toEqual([...DEFAULT_EXTENSIONS].sort());
    expect(engine.extensions.map((e) => e.name)).toEqual(expect.arrayContaining(["spatial", "three_d"]));
  });

  it("runs DuckDB v1.5.5", async () => {
    const reader = await engine.connection.runAndReadAll("SELECT version() AS v");
    expect(reader.getRowsJson()[0]![0]).toBe("v1.5.5");
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

describe("createEngine with no extensions", () => {
  it("comes up with none loaded rather than failing on an empty list", async () => {
    const engine = await createEngine({ sandbox: false, extensionDirectory, extensions: [] });
    expect(engine.extensions).toEqual([]);
    await expect(engine.connection.run("SELECT 1")).resolves.toBeDefined();
    await engine.close();
  });
});

describe("extensionsFromEnv", () => {
  it("falls back to the defaults when unset", () => {
    expect(extensionsFromEnv(undefined)).toEqual(DEFAULT_EXTENSIONS);
  });

  it("falls back to the defaults when empty or blank, never to an empty name", () => {
    expect(extensionsFromEnv("")).toEqual(DEFAULT_EXTENSIONS);
    expect(extensionsFromEnv(" , ")).toEqual(DEFAULT_EXTENSIONS);
  });

  it("trims names and drops empty entries", () => {
    expect(extensionsFromEnv(" httpfs, ,cityjson ,")).toEqual(["httpfs", "cityjson"]);
  });
});

// `spatial` brings GDAL — a second file reader with its own path grammar —
// so the sandbox must be shown to cover it too. The positive control matters:
// GDAL reports an unreadable path and an unparseable one with the same "Could
// not open GDAL dataset", so a refusal proves nothing unless the same read
// succeeds with the sandbox off.
describe("GDAL, through spatial", () => {
  const extensions = DEFAULT_EXTENSIONS;
  const dir = mkdtempSync(join(tmpdir(), "cityparquet-mcp-gdal-"));
  const file = join(dir, "probe.geojson");
  const read = `SELECT secret FROM ST_Read('${file}')`;

  beforeAll(() => {
    writeFileSync(file, JSON.stringify({
      type: "FeatureCollection",
      features: [{ type: "Feature", properties: { secret: "s3cr3t" }, geometry: { type: "Point", coordinates: [1, 2] } }],
    }));
  });

  it("reads a local file through GDAL without the sandbox", async () => {
    const engine = await createEngine({ sandbox: false, extensionDirectory, extensions });
    const reader = await engine.connection.runAndReadAll(read);
    expect(reader.getRowsJson()).toEqual([["s3cr3t"]]);
    await engine.close();
  });

  it("blocks the same GDAL read with the sandbox", async () => {
    const engine = await createEngine({ sandbox: true, extensionDirectory, extensions });
    await expect(engine.connection.run(read)).rejects.toThrow();
    await engine.close();
  });
});
