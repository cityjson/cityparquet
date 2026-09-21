import { describe, expect, it } from "vitest";

import { bbox2d, formatBytes, loadCatalog, summariseItem } from "./stac";

const BASE = "https://example.test/data";

/** A catalogue as the publish step writes it: flat and nested collections. */
const DOCS: Record<string, unknown> = {
  [`${BASE}/catalog.json`]: {
    type: "Catalog",
    id: "root",
    title: "Datasets",
    links: [
      { rel: "self", href: "./catalog.json" },
      { rel: "child", href: "./plateau/collection.json" },
      { rel: "child", href: "./3dbag/collection.json" },
    ],
  },
  [`${BASE}/plateau/collection.json`]: {
    type: "Collection",
    id: "plateau",
    title: "PLATEAU",
    description: "Japan",
    license: "CC-BY-4.0",
    links: [
      { rel: "root", href: "../catalog.json" },
      { rel: "item", href: "./chiyoda-ku/metadata.json" },
      { rel: "item", href: "./minato-ku/metadata.json" },
    ],
  },
  [`${BASE}/3dbag/collection.json`]: {
    type: "Collection",
    id: "3dbag",
    title: "3DBAG",
    links: [{ rel: "item", href: "./metadata.json" }],
  },
  [`${BASE}/plateau/chiyoda-ku/metadata.json`]: item(
    "chiyoda-ku",
    "Chiyoda-ku",
  ),
  [`${BASE}/plateau/minato-ku/metadata.json`]: item("minato-ku", undefined),
  [`${BASE}/3dbag/metadata.json`]: item("3dbag", "3DBAG"),
};

function item(id: string, title: string | undefined) {
  return {
    type: "Feature",
    stac_version: "1.1.0",
    id,
    bbox: [139.7, 35.6, 10, 139.8, 35.7, 200],
    properties: {
      ...(title ? { title } : {}),
      "city3d:city_objects": 1234,
      "city3d:lods": ["0.0", "2.0"],
      "city3d:co_types": ["Building", "Road"],
    },
    assets: {
      data: { href: "./building.parquet", "file:size": 100 },
      "building.parquet": { href: "./building.parquet", "file:size": 100 },
      "road.parquet": { href: "./road.parquet", "file:size": 50 },
    },
    links: [],
  };
}

async function fakeFetch(url: string): Promise<unknown> {
  if (!(url in DOCS)) throw new Error(`404 ${url}`);
  return DOCS[url];
}

describe("loadCatalog", () => {
  it("follows child and item links relative to each document", async () => {
    const cat = await loadCatalog(`${BASE}/catalog.json`, fakeFetch);
    expect(cat.title).toBe("Datasets");
    expect(cat.collections.map((c) => c.id)).toEqual(["plateau", "3dbag"]);
    expect(cat.collections[0].items.map((i) => i.id)).toEqual([
      "chiyoda-ku",
      "minato-ku",
    ]);
    expect(cat.collections[1].items[0].url).toBe(`${BASE}/3dbag/metadata.json`);
  });

  it("keeps the rest of a collection when one item cannot be read", async () => {
    const broken = { ...DOCS };
    delete broken[`${BASE}/plateau/minato-ku/metadata.json`];
    const cat = await loadCatalog(`${BASE}/catalog.json`, async (u) => {
      if (!(u in broken)) throw new Error("404");
      return broken[u];
    });
    expect(cat.collections[0].items.map((i) => i.id)).toEqual(["chiyoda-ku"]);
    expect(cat.collections[0].failed).toEqual([
      `${BASE}/plateau/minato-ku/metadata.json`,
    ]);
  });
});

describe("summariseItem", () => {
  it("reads the city3d properties, the 2D extent and the package size", () => {
    const s = summariseItem(item("x", "X"), `${BASE}/plateau/x/metadata.json`);
    expect(s.title).toBe("X");
    expect(s.objects).toBe(1234);
    expect(s.lods).toEqual(["0", "2"]);
    expect(s.types).toEqual(["Building", "Road"]);
    expect(s.bbox).toEqual([139.7, 35.6, 139.8, 35.7]);
    // `data` and `building.parquet` are one file, counted once.
    expect(s.bytes).toBe(150);
    expect(s.folder).toBe(`${BASE}/plateau/x/`);
  });

  it("falls back to the id for a title", () => {
    expect(
      summariseItem(item("minato-ku", undefined), `${BASE}/m/metadata.json`)
        .title,
    ).toBe("minato-ku");
  });
});

describe("bbox2d", () => {
  it("drops the heights of a 3D box and passes a 2D one through", () => {
    expect(bbox2d([1, 2, -3, 4, 5, 6])).toEqual([1, 2, 4, 5]);
    expect(bbox2d([1, 2, 4, 5])).toEqual([1, 2, 4, 5]);
    expect(bbox2d(undefined)).toBeNull();
  });
});

describe("formatBytes", () => {
  it("uses decimal units", () => {
    expect(formatBytes(16_400_304_209)).toBe("16.4 GB");
    expect(formatBytes(5_466_255)).toBe("5.5 MB");
    expect(formatBytes(512)).toBe("512 B");
  });
});
