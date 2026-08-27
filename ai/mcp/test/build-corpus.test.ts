import { describe, expect, it } from "vitest";
import { chapterIdFromFilename, chaptersFromFunctionsMarkdown, declaredOrder } from "../src/build-corpus.js";

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

describe("chaptersFromFunctionsMarkdown", () => {
  it("folds a chapter's subsections back into its body instead of dropping them", () => {
    const source = [
      "## A",
      "",
      "Prose for A.",
      "",
      "### A1",
      "",
      "Prose for A1.",
      "",
      "#### A1a",
      "",
      "Prose for A1a.",
      "",
      "## B",
      "",
      "Prose for B.",
    ].join("\n");

    const chapters = chaptersFromFunctionsMarkdown(source);
    expect(chapters.map((c) => c.title)).toEqual(["A", "B"]);

    const a = chapters[0]!;
    expect(a.body).toContain("Prose for A.");
    expect(a.body).toContain("### A1");
    expect(a.body).toContain("Prose for A1.");
    expect(a.body).toContain("#### A1a");
    expect(a.body).toContain("Prose for A1a.");
    expect(a.sections).toEqual([{ heading: "A1", level: 3 }]);

    const b = chapters[1]!;
    expect(b.body).toContain("Prose for B.");
    expect(b.body).not.toContain("A1");
    expect(b.body).not.toContain("Prose for A1");
    expect(b.sections).toEqual([]);
  });
});
