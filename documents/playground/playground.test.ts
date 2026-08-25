import { describe, expect, it } from "vitest";

import { DATA_BASE_URL, EXTENSIONS, ROWS_PER_PAGE, ROW_DISPLAY_CAP } from "./config";
import { PRESETS, DEFAULT_PRESET_ID, findPreset } from "./presets";
import { buildHash, decodeSql, encodeSql, parseHash } from "./lib/share";
import { formatBytes } from "./lib/bytes";

describe("share links", () => {
  it("round-trips SQL, including the characters a URL would otherwise eat", () => {
    const sql = "SELECT 'a+b/c=d', \"quoted\" FROM t WHERE x > 1 AND y < 2; -- ü é 漢";
    expect(decodeSql(encodeSql(sql))).toBe(sql);
  });

  it("produces a hash with nothing needing further escaping", () => {
    const encoded = encodeSql("SELECT * FROM t WHERE a = 'b/c+d'");
    expect(encoded).not.toMatch(/[+/=]/);
    expect(encodeURIComponent(encoded)).toBe(encoded);
  });

  it("prefers the preset id when the query is unmodified", () => {
    expect(buildHash({ presetId: "count", sql: null })).toBe("#preset=count");
  });

  it("carries the SQL once the query is the reader's own", () => {
    const hash = buildHash({ presetId: null, sql: "SELECT 1" });
    expect(parseHash(hash)).toEqual({ presetId: null, sql: "SELECT 1" });
  });

  it("reads back what it wrote", () => {
    for (const state of [
      { presetId: "roof-types", sql: null },
      { presetId: null, sql: "SELECT count(*) FROM t" },
    ]) {
      expect(parseHash(buildHash(state))).toEqual(state);
    }
  });

  it("treats an unparseable hash as empty rather than throwing", () => {
    for (const hash of ["", "#", "#nonsense", "#sql=!!!not-base64!!!"]) {
      expect(() => parseHash(hash)).not.toThrow();
    }
    expect(parseHash("#nonsense")).toEqual({ presetId: null, sql: null });
  });
});

describe("the preset registry", () => {
  it("has unique ids", () => {
    const ids = PRESETS.map((preset) => preset.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("uses ids that survive a URL unescaped, since they are share links", () => {
    for (const preset of PRESETS) {
      expect(preset.id).toMatch(/^[a-z0-9-]+$/);
    }
  });

  it("declares only extensions the playground knows how to load", () => {
    for (const preset of PRESETS) {
      for (const extension of preset.extensions) {
        expect(EXTENSIONS).toContain(extension);
      }
    }
  });

  it("gives every preset a title, a blurb and some SQL", () => {
    for (const preset of PRESETS) {
      expect(preset.title.length).toBeGreaterThan(0);
      expect(preset.blurb.length).toBeGreaterThan(0);
      expect(preset.sql.trim().length).toBeGreaterThan(0);
      expect(preset.group.length).toBeGreaterThan(0);
    }
  });

  it("reads remote data only from the configured hosts", () => {
    const allowed = [DATA_BASE_URL, "https://cityjson.open3d.city"];
    for (const preset of PRESETS) {
      for (const url of preset.sql.match(/https?:\/\/[^\s')]+/g) ?? []) {
        expect(
          allowed.some((prefix) => url.startsWith(prefix)),
          `${preset.id} reads from an unexpected host: ${url}`,
        ).toBe(true);
      }
    }
  });

  it("declares three_d whenever it calls an ST_3D function", () => {
    for (const preset of PRESETS) {
      if (/\bST_3D\w+\s*\(/i.test(preset.sql)) {
        expect(preset.extensions, `${preset.id} calls ST_3D*`).toContain("three_d");
      }
    }
  });

  it("declares cityjson whenever it calls a cityjson function", () => {
    for (const preset of PRESETS) {
      if (/\b(read_cityjson\w*|cityjson\w*)\s*\(/i.test(preset.sql)) {
        expect(preset.extensions, `${preset.id} calls a cityjson function`).toContain("cityjson");
      }
    }
  });

  it("builds a solid before measuring it, rather than passing raw WKB", () => {
    // ST_3DVolume and friends take SOLID_3D, not BLOB: a geometry column has to
    // go through ST_3DFromWKB / ST_3DTryFromWKB first, paired with its
    // geometry_properties_lod* struct so shell grouping survives. Passing the
    // column straight in binds against nothing and fails only at runtime.
    const measures = /ST_3D(?:Volume|SurfaceArea|Area|FootprintArea|Perimeter|NumShells|NumFaces|IsClosed|IsManifold|ValidationReport)\s*\(\s*(geometry_\w+)/i;
    for (const preset of PRESETS) {
      const hit = preset.sql.match(measures);
      expect(
        hit,
        `${preset.id} passes ${hit?.[1]} straight to a measurement function; wrap it in ST_3DTryFromWKB`,
      ).toBeNull();
    }
  });

  it("pairs every solid constructor with its properties column", () => {
    // The second argument is what carries shell structure; without it every
    // PolyhedralSurface imports as one shell and cavities stop subtracting.
    for (const preset of PRESETS) {
      for (const [, wkb, props] of preset.sql.matchAll(
        /ST_3D(?:Try)?FromWKB\s*\(\s*(geometry_\w+)\s*,\s*(\w+)/gi,
      )) {
        expect(props, `${preset.id}: ${wkb} should be paired with its properties column`).toBe(
          wkb.replace("geometry_", "geometry_properties_"),
        );
      }
    }
  });

  it("never selects everything from the 16.4 GB file without a limit", () => {
    for (const preset of PRESETS) {
      if (!preset.sql.includes(DATA_BASE_URL)) continue;
      const selectsAll = /select\s+\*/i.test(preset.sql);
      if (!selectsAll) continue;
      const bounded = /\blimit\b/i.test(preset.sql) || /^\s*describe\b/im.test(preset.sql.trim());
      expect(bounded, `${preset.id} selects * from the full file unbounded`).toBe(true);
    }
  });

  it("resolves the default preset", () => {
    expect(findPreset(DEFAULT_PRESET_ID)).toBeDefined();
  });

  it("returns nothing for an unknown or absent id", () => {
    expect(findPreset("no-such-preset")).toBeUndefined();
    expect(findPreset(null)).toBeUndefined();
  });
});

describe("the results grid", () => {
  it("pages within the cap rather than past it", () => {
    // A page larger than the cap would render one page and hide the control,
    // which is the bug this pairing exists to prevent.
    expect(ROWS_PER_PAGE).toBeGreaterThan(0);
    expect(ROWS_PER_PAGE).toBeLessThanOrEqual(ROW_DISPLAY_CAP);
  });
});

describe("byte formatting", () => {
  it("uses decimal units, as storage is billed", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(999)).toBe("999 B");
    expect(formatBytes(1_000)).toBe("1.0 kB");
    expect(formatBytes(4_709_461)).toBe("4.7 MB");
    expect(formatBytes(16_400_304_209)).toBe("16.4 GB");
  });
});
