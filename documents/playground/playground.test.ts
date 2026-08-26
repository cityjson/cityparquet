import { describe, expect, it } from "vitest";
import { CompletionContext } from "@codemirror/autocomplete";
import { PostgreSQL, sql } from "@codemirror/lang-sql";
import { EditorState } from "@codemirror/state";

import { DATA_BASE_URL, EXTENSIONS, ROWS_PER_PAGE, ROW_DISPLAY_CAP } from "./config";
import {
  SchemaCache,
  cityParquetCompletion,
  elementType,
  structFields,
  tableExpressions,
} from "./lib/completion";
import { serialiser } from "./lib/serialise";
import {
  EXPORT_FORMATS,
  IMPORT_ACCEPT,
  IMPORT_FORMATS,
  explainExportFailure,
  formatFor,
  safeName,
  starterSql,
} from "./lib/files";
import { SAVED_KEY, SAVED_LIMIT, SAVED_VERSION, SavedQueries, deriveName } from "./lib/saved";
import { PRESETS, DEFAULT_PRESET_ID, findPreset } from "./presets";
import { buildHash, decodeSql, encodeSql, parseHash } from "./lib/share";
import { formatBytes } from "./lib/bytes";
import { formatDeadline } from "./lib/query";
import { QUERY_TIMEOUT_MS } from "./config";

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
    const measures =
      /ST_3D(?:Volume|SurfaceArea|Area|FootprintArea|Perimeter|NumShells|NumFaces|IsClosed|IsManifold|ValidationReport)\s*\(\s*(geometry_\w+)/i;
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

describe("the query deadline", () => {
  it("reads in minutes once it is minutes long", () => {
    // The message is built from the constant, so raising the deadline without
    // this would have the page announce a "Timeout after 600s".
    expect(formatDeadline(QUERY_TIMEOUT_MS)).toBe("10 min");
    expect(formatDeadline(60_000)).toBe("60s");
    expect(formatDeadline(600_000)).toBe("10 min");
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

describe("reading sources out of a statement", () => {
  it("finds the table functions a statement reads from", () => {
    expect(tableExpressions("SELECT * FROM read_parquet('a.parquet')")).toEqual([
      "read_parquet('a.parquet')",
    ]);
  });

  it("finds one per source across CTEs and joins, without repeating any", () => {
    const found = tableExpressions(PRESETS.find((p) => p.id === "volume-check")!.sql);
    expect(found).toHaveLength(1);
    expect(found[0]).toMatch(/^read_parquet\('https:\/\/.*building\.parquet'\)$/);
  });

  it("ignores calls that are not sources", () => {
    // A projection is not a FROM clause, and a CTE name is not describable on
    // its own — offering either to DESCRIBE would just fail.
    expect(
      tableExpressions("SELECT strftime(d, '%Y') FROM parts JOIN buildings b ON b.id = p.id"),
    ).toEqual([]);
  });

  it("normalises whitespace so one source is not cached twice", () => {
    expect(tableExpressions("FROM read_parquet(\n  'a.parquet'\n)")).toEqual([
      "read_parquet( 'a.parquet' )",
    ]);
  });

  it("covers every preset that reads a file", () => {
    for (const preset of PRESETS) {
      if (!/\bfrom\s+\w+\s*\(/i.test(preset.sql)) continue;
      expect(tableExpressions(preset.sql).length, `${preset.id}`).toBeGreaterThan(0);
    }
  });
});

describe("STRUCT types", () => {
  it("reads the bbox every CityParquet object carries", () => {
    const type =
      "STRUCT(xmin DOUBLE, ymin DOUBLE, zmin DOUBLE, xmax DOUBLE, ymax DOUBLE, zmax DOUBLE)";
    expect(structFields(type).map((f) => f.name)).toEqual([
      "xmin",
      "ymin",
      "zmin",
      "xmax",
      "ymax",
      "zmax",
    ]);
  });

  it("reads quoted names and nested lists", () => {
    // The real geometry_properties_lod* type: `type` is a keyword so DuckDB
    // quotes it, and `shells` is a list of lists.
    const type =
      'STRUCT("type" VARCHAR, surfaces VARCHAR, face_semantics INTEGER[], shells INTEGER[][])';
    expect(structFields(type)).toEqual([
      { name: "type", type: "VARCHAR" },
      { name: "surfaces", type: "VARCHAR" },
      { name: "face_semantics", type: "INTEGER[]" },
      { name: "shells", type: "INTEGER[][]" },
    ]);
  });

  it("does not split a comma inside a nested struct", () => {
    expect(structFields("STRUCT(a STRUCT(x INTEGER, y INTEGER), b VARCHAR)")).toEqual([
      { name: "a", type: "STRUCT(x INTEGER, y INTEGER)" },
      { name: "b", type: "VARCHAR" },
    ]);
  });

  it("is not fooled by anything that is not a struct", () => {
    for (const type of ["VARCHAR", "BLOB", "INTEGER[]", "MAP(VARCHAR, VARCHAR)", "STRUCTURED"]) {
      expect(structFields(type), type).toEqual([]);
    }
  });

  it("unwraps one level of list, which is what an index does", () => {
    // The 3DBAG address column is a list of structs: address[1].street.
    const type = "STRUCT(street VARCHAR, house_number VARCHAR)[]";
    expect(structFields(type)).toEqual([]);
    expect(structFields(elementType(type)).map((f) => f.name)).toEqual(["street", "house_number"]);
  });
});

describe("the schema cache", () => {
  const columnsOf = (name: string) => [{ column_name: name, column_type: "VARCHAR" }];

  it("describes each source once, however often it is asked", async () => {
    const seen: string[] = [];
    const cache = new SchemaCache(async (sql) => {
      seen.push(sql);
      return columnsOf("id");
    });
    const sql = "SELECT * FROM read_parquet('a.parquet')";
    await Promise.all([cache.columns(sql), cache.columns(sql), cache.columns(sql)]);
    expect(seen).toEqual(["DESCRIBE SELECT * FROM read_parquet('a.parquet')"]);
  });

  it("remembers a failure too, so a half-typed URL is not retried per keystroke", async () => {
    let calls = 0;
    const cache = new SchemaCache(async () => {
      calls++;
      throw new Error("IO Error: 404");
    });
    const sql = "SELECT * FROM read_parquet('https://example.invalid/hal')";
    expect(await cache.columns(sql)).toEqual([]);
    expect(await cache.columns(sql)).toEqual([]);
    expect(calls).toBe(1);
  });

  it("unions the sources a statement reads, first occurrence winning", async () => {
    const cache = new SchemaCache(async (sql) =>
      sql.includes("a.parquet") ? columnsOf("id") : [...columnsOf("id"), ...columnsOf("extra")],
    );
    const columns = await cache.columns(
      "SELECT * FROM read_parquet('a.parquet') JOIN read_parquet('b.parquet') ON true",
    );
    expect(columns.map((c) => c.name)).toEqual(["id", "extra"]);
  });

  it("survives a database that cannot answer at all", async () => {
    const cache = new SchemaCache(async () => {
      throw new Error("Connection closed");
    });
    expect(await cache.functions()).toEqual([]);
  });
});

describe("what the editor offers", () => {
  const cache = new SchemaCache(async (statement) => {
    if (statement.startsWith("DESCRIBE")) {
      return [
        { column_name: "id", column_type: "VARCHAR" },
        { column_name: "b3_dak_type", column_type: "VARCHAR" },
        { column_name: "bbox", column_type: "STRUCT(xmin DOUBLE, ymin DOUBLE, zmin DOUBLE)" },
        { column_name: "address", column_type: "STRUCT(street VARCHAR)[]" },
      ];
    }
    return [{ name: "st_3dvolume", kind: "scalar", returns: "DOUBLE", description: null }];
  });
  const complete = cityParquetCompletion(cache);

  /** `|` marks the cursor — the position is the whole point of these cases. */
  const at = (marked: string, explicit = false) => {
    const pos = marked.indexOf("|");
    const doc = marked.replace("|", "");
    return complete(
      new CompletionContext(
        EditorState.create({ doc, extensions: [sql({ dialect: PostgreSQL })] }),
        pos,
        explicit,
      ),
    );
  };

  const labels = async (marked: string, explicit = false) =>
    (await at(marked, explicit))?.options.map((option) => option.label);

  it("offers the columns of the file the statement reads", async () => {
    expect(await labels("SELECT b3| FROM read_parquet('a.parquet')")).toContain("b3_dak_type");
  });

  it("offers the functions the loaded extensions added", async () => {
    expect(await labels("SELECT st_3| FROM read_parquet('a.parquet')")).toContain("st_3dvolume");
  });

  it("says nothing inside a comment", async () => {
    // Every preset opens with one, so this is the common case, not an edge.
    expect(await at("-- b3| is the reconstruction prefix\nSELECT 1", true)).toBeNull();
  });

  it("says nothing inside a string, where the URLs live", async () => {
    expect(await at("SELECT * FROM read_parquet('https://host/buil|", true)).toBeNull();
  });

  it("offers the fields of a struct after its dot", async () => {
    const result = await at("SELECT bbox.x| FROM read_parquet('a.parquet')");
    expect(result?.options.map((option) => option.label)).toEqual(["xmin", "ymin", "zmin"]);
    // Replacing only what was typed after the dot, not the column name.
    expect(result?.from).toBe(12);
  });

  it("offers a list of structs only once the text has indexed into it", async () => {
    expect(await at("SELECT address.| FROM read_parquet('a.parquet')")).toBeNull();
    expect(await labels("SELECT address[1].| FROM read_parquet('a.parquet')")).toEqual(["street"]);
  });

  it("says nothing after an alias dot rather than guessing", async () => {
    // `p` is not a column, so its fields are unknowable here — and a list of
    // every column would be wrong in a way the reader would have to undo.
    expect(await at("SELECT p.x| FROM read_parquet('a.parquet') p")).toBeNull();
  });

  it("stays quiet on a bare keystroke unless asked", async () => {
    // Nine hundred functions is not a helpful response to pressing space.
    expect(await at("SELECT id, | FROM read_parquet('a.parquet')")).toBeNull();
    expect(await at("SELECT id, | FROM read_parquet('a.parquet')", true)).not.toBeNull();
  });
});

describe("running one statement at a time", () => {
  it("does not start a task until the previous one has settled", async () => {
    const serialise = serialiser();
    const order: string[] = [];
    let releaseFirst: () => void = () => {};

    const first = serialise(async () => {
      order.push("first started");
      await new Promise<void>((resolve) => (releaseFirst = resolve));
      order.push("first finished");
    });
    const second = serialise(async () => {
      order.push("second started");
    });

    // A microtask turn is enough for the second to start if it were going to.
    await Promise.resolve();
    expect(order).toEqual(["first started"]);

    releaseFirst();
    await Promise.all([first, second]);
    expect(order).toEqual(["first started", "first finished", "second started"]);
  });

  it("rejects for the failed caller alone, and keeps the queue moving", async () => {
    const serialise = serialiser();
    const failed = serialise(async () => {
      throw new Error("memory access out of bounds");
    });
    const after = serialise(async () => "ran anyway");

    await expect(failed).rejects.toThrow("memory access out of bounds");
    expect(await after).toBe("ran anyway");
  });
});

describe("naming a saved query", () => {
  it("prefers the leading comment, which is what a preset explains itself with", () => {
    expect(deriveName("-- Roof types across the country\nSELECT 1")).toBe(
      "Roof types across the country",
    );
  });

  it("falls back to the first line of SQL", () => {
    expect(deriveName("SELECT count(*) FROM t;")).toBe("SELECT count(*) FROM t");
  });

  it("truncates rather than filling the sidebar with one name", () => {
    const name = deriveName(`-- ${"very long ".repeat(20)}`);
    expect(name.length).toBeLessThanOrEqual(60);
    expect(name.endsWith("…")).toBe(true);
  });

  it("has something to say about a query that is only whitespace", () => {
    expect(deriveName("   \n\n  ")).toBe("Untitled query");
  });
});

describe("saved queries", () => {
  /** A Storage that behaves, for the cases where storage works. */
  const fakeStorage = (): Storage => {
    const map = new Map<string, string>();
    return {
      get length() {
        return map.size;
      },
      clear: () => map.clear(),
      getItem: (key: string) => map.get(key) ?? null,
      key: (index: number) => [...map.keys()][index] ?? null,
      removeItem: (key: string) => void map.delete(key),
      setItem: (key: string, value: string) => void map.set(key, value),
    } as Storage;
  };

  it("keeps a query and reads it back", () => {
    const store = new SavedQueries(fakeStorage());
    store.save("SELECT 1", "One", 1_000);
    expect(store.list().map((q) => [q.name, q.sql])).toEqual([["One", "SELECT 1"]]);
  });

  it("survives a new store over the same storage, which is the whole point", () => {
    const storage = fakeStorage();
    new SavedQueries(storage).save("SELECT 1", "One", 1_000);
    expect(new SavedQueries(storage).list()).toHaveLength(1);
  });

  it("updates rather than duplicating when the same SQL is saved twice", () => {
    const store = new SavedQueries(fakeStorage());
    store.save("SELECT 1", "One", 1_000);
    const after = store.save("SELECT 1", "Renamed", 2_000);
    expect(after).toHaveLength(1);
    expect(after[0].name).toBe("Renamed");
  });

  it("lists newest first", () => {
    const store = new SavedQueries(fakeStorage());
    store.save("SELECT 1", "Older", 1_000);
    store.save("SELECT 2", "Newer", 2_000);
    expect(store.list().map((q) => q.name)).toEqual(["Newer", "Older"]);
  });

  it("renames and removes by id", () => {
    const store = new SavedQueries(fakeStorage());
    const [saved] = store.save("SELECT 1", "One", 1_000);
    expect(store.rename(saved.id, "Two")[0].name).toBe("Two");
    expect(store.remove(saved.id)).toEqual([]);
  });

  it("drops the oldest at the limit instead of refusing to save", () => {
    const store = new SavedQueries(fakeStorage());
    for (let i = 0; i < SAVED_LIMIT + 5; i++) store.save(`SELECT ${i}`, `q${i}`, 1_000 + i);
    const list = store.list();
    expect(list).toHaveLength(SAVED_LIMIT);
    expect(list[0].name).toBe(`q${SAVED_LIMIT + 4}`);
  });

  it("ignores data written by a version that is not this one", () => {
    const storage = fakeStorage();
    storage.setItem(SAVED_KEY, JSON.stringify({ version: 999, queries: [{ id: "q1" }] }));
    expect(new SavedQueries(storage).list()).toEqual([]);
  });

  it("ignores anything malformed rather than rendering it", () => {
    const storage = fakeStorage();
    storage.setItem(
      SAVED_KEY,
      JSON.stringify({ version: SAVED_VERSION, queries: [{ id: "q1" }, null, "nonsense"] }),
    );
    expect(new SavedQueries(storage).list()).toEqual([]);
  });

  it("still runs when the browser refuses to store anything", () => {
    // Private mode: the list is right for this session and simply does not last.
    const store = new SavedQueries(null);
    expect(store.available).toBe(false);
    expect(store.save("SELECT 1", "One", 1_000)).toHaveLength(1);
    expect(store.list()).toEqual([]);
  });

  it("does not throw when storage throws on write", () => {
    const storage = {
      ...fakeStorage(),
      setItem: () => {
        throw new Error("QuotaExceededError");
      },
    } as Storage;
    const store = new SavedQueries(storage);
    expect(() => store.save("SELECT 1", "One", 1_000)).not.toThrow();
  });
});

describe("importing a local file", () => {
  it("picks the reader from the longest matching suffix", () => {
    // `.city.jsonl` must not be read as `.json`-something.
    expect(formatFor("delft.city.jsonl")?.reader).toBe("read_cityjsonseq");
    expect(formatFor("delft.city.json")?.reader).toBe("read_cityjson");
    expect(formatFor("building.parquet")?.reader).toBe("read_parquet");
    expect(formatFor("delft.fcb")?.reader).toBe("read_flatcitybuf");
    expect(formatFor("notes.txt")).toBeNull();
  });

  it("is case-insensitive, because file managers are not consistent", () => {
    expect(formatFor("BUILDING.PARQUET")?.reader).toBe("read_parquet");
  });

  it("makes a name that cannot break out of the quotes it is pasted into", () => {
    // The name lands inside read_parquet('…'), so a quote would end the string.
    expect(safeName("my data'; DROP TABLE x; --.parquet")).not.toContain("'");
    expect(safeName("a file with spaces.parquet")).toBe("a_file_with_spaces.parquet");
    expect(safeName("/tmp/nested/path/file.parquet")).toBe("file.parquet");
    expect(safeName("...")).toBe("...");
    expect(safeName("'''")).toBe("imported");
  });

  it("reads text formats whole and binary ones lazily", () => {
    // A text reader walks the file start to end and hangs on a lazy handle;
    // Parquet seeks, which is what makes a local package cheap to open.
    const by = (id: string) => IMPORT_FORMATS.find((f) => f.id === id)!;
    expect(by("cityparquet").registration).toBe("handle");
    expect(by("flatcitybuf").registration).toBe("handle");
    expect(by("cityjson").registration).toBe("buffer");
    expect(by("cityjsonseq").registration).toBe("buffer");
  });

  it("offers every known extension to the file picker", () => {
    for (const format of IMPORT_FORMATS) {
      for (const extension of format.extensions) expect(IMPORT_ACCEPT).toContain(extension);
    }
  });

  it("starts the reader from a query that names the file", () => {
    const sql = starterSql({
      name: "delft.parquet",
      format: IMPORT_FORMATS[0],
      size: 1,
      columns: [],
      error: null,
    });
    expect(sql).toContain("read_parquet('delft.parquet')");
    expect(sql).toMatch(/\blimit\b/i);
  });
});

describe("explaining an export that did not work", () => {
  const city = EXPORT_FORMATS.find((f) => f.id === "cityjsonseq")!;
  const csv = EXPORT_FORMATS.find((f) => f.id === "csv")!;

  it("names the writer that reports rows and writes nothing", () => {
    // The published cityjson build does exactly this in DuckDB-Wasm, and
    // handing the reader an empty file would be the worst of the outcomes.
    const said = explainExportFailure(
      city,
      "The CityJSONSeq writer reported 20 rows but produced an empty file.",
    );
    expect(said).toContain("empty file");
    expect(said).toContain("Parquet or CSV");
  });

  it("blames the build when the writer is not in it", () => {
    expect(
      explainExportFailure(city, "Copy Function with name cityjsonseq does not exist"),
    ).toContain("lag");
  });

  it("explains that a city model needs city columns", () => {
    expect(explainExportFailure(city, "Binder Error: no column named id")).toContain("id");
  });

  it("has nothing to add about a plain SQL mistake in a plain format", () => {
    expect(explainExportFailure(csv, "Parser Error: syntax error at or near")).toBeNull();
  });
});
