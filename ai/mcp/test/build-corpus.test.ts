import { describe, expect, it } from "vitest";
import { chapterIdFromFilename, declaredOrder } from "../src/build-corpus.js";

describe("chapterIdFromFilename", () => {
  it("strips the numeric prefix and the extension", () => {
    expect(chapterIdFromFilename("02-object-table-schema.mdx")).toBe("object-table-schema");
    expect(chapterIdFromFilename("index.mdx")).toBe("index");
    expect(chapterIdFromFilename("01-getting-started.mdx")).toBe("getting-started");
  });
});

describe("declaredOrder", () => {
  it("reads the pages array out of a meta.ts", () => {
    const meta = [
      'import { defineMeta } from "blume";',
      "export default defineMeta({",
      '  title: "Specification & file format",',
      '  icon: "file-text",',
      "  pages: [",
      '    "dataset-package",',
      '    "object-table-schema",',
      "  ],",
      "});",
    ].join("\n");
    expect(declaredOrder(meta)).toEqual(["dataset-package", "object-table-schema"]);
  });

  it("returns an empty list when there is no pages array", () => {
    expect(declaredOrder('export default defineMeta({ title: "x" });')).toEqual([]);
  });
});
