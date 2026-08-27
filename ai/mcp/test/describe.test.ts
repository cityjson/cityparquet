import { afterAll, afterEach, beforeAll, describe as suite, expect, it, vi } from "vitest";
import { DuckDBInstance } from "@duckdb/node-api";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import type { Engine } from "../src/duckdb.js";
import { serialiser } from "../src/serialise.js";
import { MODULE_TABLES, SIDECAR_TABLES, describe, geometryColumnsOf, lodsOf } from "../src/tools/describe.js";

suite("the package file inventory", () => {
  it("names every CityGML module table the specification defines", () => {
    expect(MODULE_TABLES).toEqual([
      "building", "bridge", "tunnel", "construction", "transportation",
      "vegetation", "relief", "water_body", "land_use", "city_furniture", "generics",
    ]);
  });

  it("names the three sidecars", () => {
    expect(SIDECAR_TABLES).toEqual(["materials", "textures", "geometry_templates"]);
  });
});

suite("geometryColumnsOf", () => {
  it("picks the geometry columns and ignores their properties siblings", () => {
    expect(geometryColumnsOf(["id", "geometry_lod0_0", "geometry_properties_lod0_0", "bbox"]))
      .toEqual(["geometry_lod0_0"]);
  });

  it("returns an empty list when there are none", () => {
    expect(geometryColumnsOf(["id", "bbox"])).toEqual([]);
  });
});

suite("lodsOf", () => {
  it("derives the distinct LoDs from the geometry column names", () => {
    expect(lodsOf(["geometry_lod0_0", "geometry_lod2_2", "geometry_lod2_2"])).toEqual(["0.0", "2.2"]);
  });
});

// Both inventory paths, without a network or a real engine. The STAC Item's
// assets map is a SHOULD, so the probe fallback is a designed path and gets the
// same coverage as the happy one.
//
// `kvRows` stands in for `parquet_kv_metadata`'s `(key, decode(value))` rows.
// The default carries a `city` entry whose crs has no name, just an id — so
// `footerCrs` renders it as "EPSG:7415", matching what earlier fixtures in
// this file expect.
function fakeEngine(
  known: Record<string, string[]>,
  kvRows: [string, string][] = [["city", JSON.stringify({ crs: { id: { authority: "EPSG", code: 7415 } } })]],
): Engine {
  return {
    extensions: [],
    async close() {},
    // A pass-through. These fixtures never have two calls in flight at
    // once, so there is nothing here for `exclusive` to serialise against —
    // it exists only so `describe()`'s call to it type-checks and runs.
    exclusive: <T>(task: () => Promise<T>) => task(),
    connection: {
      async runAndReadAll(sql: string) {
        const url = /'([^']+)'/.exec(sql)?.[1] ?? "";
        if (sql.includes("parquet_schema")) {
          const columns = known[url];
          if (!columns) throw new Error(`no such file: ${url}`);
          return { getRowsJson: () => columns.map((c) => [c]) };
        }
        if (sql.includes("parquet_file_metadata")) return { getRowsJson: () => [[7]] };
        if (sql.includes("parquet_kv_metadata")) return { getRowsJson: () => kvRows };
        throw new Error(`fakeEngine: unexpected query: ${sql}`);
      },
    },
  } as unknown as Engine;
}

suite("describe, package inventory", () => {
  afterEach(() => { vi.unstubAllGlobals(); });

  it("uses the STAC assets map when it enumerates Parquet files", async () => {
    vi.stubGlobal("fetch", async () => ({
      ok: true,
      status: 200,
      json: async () => ({ assets: { building: { href: "building.parquet" } } }),
    }));
    const result = await describe(
      fakeEngine({ "https://example.test/pkg/building.parquet": ["id", "geometry_lod2_2", "bbox"] }),
      "https://example.test/pkg",
    );
    expect(result.inventory).toBe("stac");
    expect(result.tables.map((t) => t.name)).toEqual(["building"]);
    expect(result.tables[0]!.lods).toEqual(["2.2"]);
    expect(result.crs).toBe("EPSG:7415");
  });

  it("falls back to probing the normative basenames when there is no metadata.json", async () => {
    vi.stubGlobal("fetch", async () => ({ ok: false, status: 404, json: async () => ({}) }));
    const result = await describe(
      fakeEngine({ "https://example.test/pkg/building.parquet": ["id", "geometry_lod0_0"] }),
      "https://example.test/pkg",
    );
    expect(result.inventory).toBe("probe");
    expect(result.tables.map((t) => t.name)).toEqual(["building"]);
    expect(result.notes.join(" ")).toMatch(/no metadata\.json/);
  });

  it("probes too when the Item carries no Parquet assets", async () => {
    vi.stubGlobal("fetch", async () => ({ ok: true, status: 200, json: async () => ({ assets: {} }) }));
    const result = await describe(
      fakeEngine({ "https://example.test/pkg/relief.parquet": ["id"] }),
      "https://example.test/pkg",
    );
    expect(result.inventory).toBe("probe");
    expect(result.tables.map((t) => t.name)).toEqual(["relief"]);
  });

  it("throws when nothing under the URL is readable", async () => {
    vi.stubGlobal("fetch", async () => ({ ok: false, status: 404, json: async () => ({}) }));
    await expect(describe(fakeEngine({}), "https://example.test/empty")).rejects.toThrow(/no readable Parquet/);
  });

  it("deduplicates a STAC asset map that lists the same file under two keys", async () => {
    // Real STAC Items produced by this stack list a generic "data" role
    // alongside a module-named one, both pointing at the same href — an
    // agent reading two table entries would wrongly conclude there are two
    // building tables.
    vi.stubGlobal("fetch", async () => ({
      ok: true,
      status: 200,
      json: async () => ({
        assets: {
          data: { href: "building.parquet" },
          "building.parquet": { href: "building.parquet" },
        },
      }),
    }));
    const result = await describe(
      fakeEngine({ "https://example.test/pkg/building.parquet": ["id", "geometry_lod2_2"] }),
      "https://example.test/pkg",
    );
    expect(result.tables).toHaveLength(1);
    expect(result.tables.map((t) => t.name)).toEqual(["data"]); // first occurrence wins
  });
});

suite("describe, CRS rendering from the footer", () => {
  afterEach(() => { vi.unstubAllGlobals(); });

  it("renders '<name> (<authority>:<code>)' when the city footer's crs carries an id", async () => {
    const engine = fakeEngine(
      { "https://example.test/pkg/building.parquet": ["id"] },
      [["city", JSON.stringify({
        crs: { name: "Amersfoort / RD New + NAP height", id: { authority: "EPSG", code: 7415 } },
      })]],
    );
    const result = await describe(engine, "https://example.test/pkg/building.parquet");
    expect(result.crs).toBe("Amersfoort / RD New + NAP height (EPSG:7415)");
  });

  it("renders the name alone when the crs carries no id", async () => {
    const engine = fakeEngine(
      { "https://example.test/pkg/building.parquet": ["id"] },
      [["city", JSON.stringify({ crs: { name: "Amersfoort / RD New + NAP height" } })]],
    );
    const result = await describe(engine, "https://example.test/pkg/building.parquet");
    expect(result.crs).toBe("Amersfoort / RD New + NAP height");
  });

  it("falls back to the geo footer's primary column crs when the city entry has none", async () => {
    const engine = fakeEngine(
      { "https://example.test/pkg/building.parquet": ["id"] },
      [
        ["city", JSON.stringify({ crs: null })],
        ["geo", JSON.stringify({
          primary_column: "geometry_lod0_0",
          columns: { geometry_lod0_0: { crs: { name: "WGS 84", id: { authority: "EPSG", code: 4326 } } } },
        })],
      ],
    );
    const result = await describe(engine, "https://example.test/pkg/building.parquet");
    expect(result.crs).toBe("WGS 84 (EPSG:4326)");
  });

  it("returns null when neither footer entry carries a usable crs", async () => {
    const engine = fakeEngine({ "https://example.test/pkg/building.parquet": ["id"] }, []);
    const result = await describe(engine, "https://example.test/pkg/building.parquet");
    expect(result.crs).toBeNull();
  });
});

// A real DuckDB engine and a real Parquet file, no network. The fakeEngine
// suites above pin describe()'s branching logic, and their canned responses
// were themselves updated once already to encode a fix after a defect this
// mock shape could not have caught — see the task report. Only a real engine
// proves the SQL describe() actually issues (parquet_schema,
// parquet_file_metadata, parquet_kv_metadata, decode()) still parses and
// still means what this module assumes it means.
//
// `describe()` given a local directory path throws inside `fetch()` on a
// non-URL base, so this exercises the single-file `.parquet` path rather
// than the package path — see the CLAUDE.md note on that gap.
suite("describe, against a real engine and a real fixture file", () => {
  let engine: Engine;
  let file: string;

  beforeAll(async () => {
    // A bare DuckDB connection, not `createEngine`: `describe()` needs
    // nothing but core Parquet functions (`parquet_schema`,
    // `parquet_file_metadata`, `parquet_kv_metadata`, `decode`), and no
    // extension load — so no network — is required to exercise it for real.
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

    const dir = mkdtempSync(join(tmpdir(), "cityparquet-mcp-fixture-"));
    file = join(dir, "building.parquet");
    const city = JSON.stringify({
      crs: { name: "Amersfoort / RD New + NAP height", id: { authority: "EPSG", code: 7415 } },
    }).replace(/'/g, "''");
    await engine.connection.run(
      `COPY (SELECT 1 AS id, encode('geom')::BLOB AS geometry_lod2_2) TO '${file}' ` +
        `(FORMAT PARQUET, KV_METADATA {city: '${city}'})`,
    );
  });
  afterAll(async () => { await engine?.close(); });

  it("reads the table's row count, geometry columns and LoDs from the real footer", async () => {
    const result = await describe(engine, file);
    expect(result.kind).toBe("file");
    expect(result.tables).toHaveLength(1);
    expect(result.tables[0]!.row_count).toBe(1);
    expect(result.tables[0]!.geometry_columns).toEqual(["geometry_lod2_2"]);
    expect(result.tables[0]!.lods).toEqual(["2.2"]);
  });

  it("decodes the CRS from the real KV metadata", async () => {
    const result = await describe(engine, file);
    expect(result.crs).toBe("Amersfoort / RD New + NAP height (EPSG:7415)");
  });
});
