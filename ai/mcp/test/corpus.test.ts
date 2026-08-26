import { describe, expect, it } from "vitest";
import { loadCorpus, outline, readChapter, search } from "../src/corpus.js";
import type { Corpus } from "../src/corpus.js";

describe("loadCorpus", () => {
  it("reads the committed corpus.json", () => {
    expect(loadCorpus().corpora.spec.chapters.length).toBeGreaterThan(0);
  });
});

const CORPUS: Corpus = {
  generatedFrom: "test",
  corpora: {
    spec: {
      title: "CityParquet specification",
      source: "documents/docs/03-specification",
      chapters: [
        {
          id: "object-table-schema",
          title: "Object table schema",
          description: "Reserved columns.",
          order: 0,
          sections: [{ heading: "Reserved columns", level: 2 }],
          body: "Intro paragraph.\n\n## Reserved columns\n\nThe bbox column holds the extent.",
        },
      ],
    },
    "duckdb-cityjson": {
      title: "duckdb-cityjson function reference",
      source: "lib/duckdb-cityjson/docs/FUNCTIONS.md",
      chapters: [
        { id: "reading", title: "Reading", description: "", order: 0, sections: [], body: "read_cityjsonseq streams a file." },
      ],
    },
    "duckdb-3d": { title: "duckdb-3d function reference", source: "x", chapters: [] },
  },
};

describe("outline", () => {
  it("lists every corpus when none is named", () => {
    expect(outline(CORPUS).corpora.map((c) => c.id)).toEqual(["spec", "duckdb-cityjson", "duckdb-3d"]);
  });

  it("narrows to one corpus when named", () => {
    const result = outline(CORPUS, "spec");
    expect(result.corpora).toHaveLength(1);
    expect(result.corpora[0]!.chapters[0]!.sections).toEqual(["Reserved columns"]);
  });

  it("carries no chapter bodies", () => {
    expect(JSON.stringify(outline(CORPUS))).not.toContain("Intro paragraph");
  });
});

describe("search", () => {
  it("finds a term in a section body and names its heading", () => {
    const hits = search(CORPUS, "bbox");
    expect(hits).toHaveLength(1);
    expect(hits[0]!.chapter).toBe("object-table-schema");
    expect(hits[0]!.heading).toBe("Reserved columns");
    expect(hits[0]!.snippet).toContain("bbox");
  });

  it("is case-insensitive", () => {
    expect(search(CORPUS, "BBOX")).toHaveLength(1);
  });

  it("restricts to one corpus when asked", () => {
    expect(search(CORPUS, "read_cityjsonseq", { corpus: "spec" })).toHaveLength(0);
    expect(search(CORPUS, "read_cityjsonseq", { corpus: "duckdb-cityjson" })).toHaveLength(1);
  });

  it("honours the limit", () => {
    expect(search(CORPUS, "the", { limit: 1 })).toHaveLength(1);
  });
});

describe("readChapter", () => {
  it("returns the whole chapter body", () => {
    expect(readChapter(CORPUS, "spec", "object-table-schema")).toContain("Intro paragraph");
  });

  it("returns one section when named", () => {
    const text = readChapter(CORPUS, "spec", "object-table-schema", "Reserved columns");
    expect(text).toContain("bbox column");
    expect(text).not.toContain("Intro paragraph");
  });

  it("throws for an unknown chapter, listing what exists", () => {
    expect(() => readChapter(CORPUS, "spec", "nope")).toThrow(/object-table-schema/);
  });

  it("throws for an unknown section", () => {
    expect(() => readChapter(CORPUS, "spec", "object-table-schema", "Nope")).toThrow(/Reserved columns/);
  });
});
