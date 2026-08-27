import { describe, expect, it } from "vitest";
import { isCorpusFresh } from "../src/check-corpus-fresh.js";
import type { Corpus } from "../src/corpus.js";

function corpus(overrides: Partial<Corpus> = {}): Corpus {
  return {
    generatedFrom: "aaaaaaa",
    corpora: {
      spec: { title: "spec", source: "docs", chapters: [] },
      "duckdb-cityjson": { title: "duckdb-cityjson", source: "docs", chapters: [] },
      "duckdb-3d": { title: "duckdb-3d", source: "docs", chapters: [] },
    },
    ...overrides,
  };
}

describe("isCorpusFresh", () => {
  it("is fresh when only the provenance stamp differs", () => {
    const committed = corpus({ generatedFrom: "3776eca" });
    const current = corpus({ generatedFrom: "8e54256" });
    expect(isCorpusFresh(committed, current)).toBe(true);
  });

  it("is stale when the corpus content differs", () => {
    const committed = corpus();
    const current = corpus({
      corpora: {
        ...corpus().corpora,
        spec: {
          title: "spec",
          source: "docs",
          chapters: [{ id: "new", title: "New", description: "", order: 0, sections: [], body: "" }],
        },
      },
    });
    expect(isCorpusFresh(committed, current)).toBe(false);
  });
});
