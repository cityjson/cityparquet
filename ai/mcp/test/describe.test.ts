import { afterEach, describe as suite, expect, it, vi } from "vitest";
import type { Engine } from "../src/duckdb.js";
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
function fakeEngine(known: Record<string, string[]>): Engine {
  return {
    extensions: [],
    async close() {},
    connection: {
      async runAndReadAll(sql: string) {
        const url = /'([^']+)'/.exec(sql)?.[1] ?? "";
        if (sql.includes("parquet_schema")) {
          const columns = known[url];
          if (!columns) throw new Error(`no such file: ${url}`);
          return { getRowsJson: () => columns.map((c) => [c]) };
        }
        if (sql.includes("parquet_file_metadata")) return { getRowsJson: () => [[7]] };
        return { getRowsJson: () => [["EPSG:7415"]] }; // the footer CRS probe
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
});
