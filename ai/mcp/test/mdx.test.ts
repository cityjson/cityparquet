import { describe, expect, it } from "vitest";
import { reduceMdx, splitSections } from "../src/mdx.js";

const SITE = "https://cityparquet.open3d.city";

describe("reduceMdx", () => {
  it("lifts title and description out of the frontmatter", () => {
    const doc = reduceMdx(
      ["---", "title: Object table schema", "description: Reserved columns.", "sidebar:", "  label: Object table schema", "---", "", "Body text."].join("\n"),
      { siteBaseUrl: SITE },
    );
    expect(doc.title).toBe("Object table schema");
    expect(doc.description).toBe("Reserved columns.");
    expect(doc.body.trim()).toBe("Body text.");
  });

  it("unwraps an admonition into a bolded line and its body", () => {
    const doc = reduceMdx(":::note[Coming soon]\nNot yet hosted.\n:::", { siteBaseUrl: SITE });
    expect(doc.body).toContain("**Coming soon**");
    expect(doc.body).toContain("Not yet hosted.");
    expect(doc.body).not.toContain(":::");
  });

  it("drops import statements and JSX elements", () => {
    const doc = reduceMdx('import Foo from "./foo";\n\n<Foo bar="1" />\n\nKept.', { siteBaseUrl: SITE });
    expect(doc.body).not.toContain("import");
    expect(doc.body).not.toContain("<Foo");
    expect(doc.body).toContain("Kept.");
  });

  it("rewrites site-relative links to absolute ones", () => {
    const doc = reduceMdx("See [extensions](/specification/extensions).", { siteBaseUrl: SITE });
    expect(doc.body).toContain(`${SITE}/specification/extensions`);
  });

  it("leaves absolute links alone", () => {
    const doc = reduceMdx("See [duckdb](https://duckdb.org).", { siteBaseUrl: SITE });
    expect(doc.body).toContain("https://duckdb.org");
  });

  it("strips multi-line JSX elements while preserving their content", () => {
    const doc = reduceMdx(
      "<Card\n  title=\"Geometry, LoD & spatial metadata\"\n  href=\"/specification/geometry-semantics\"\n  icon=\"box\"\n>\nWKB encoding, surface semantics.\n</Card>\n\nKept.",
      { siteBaseUrl: SITE },
    );
    expect(doc.body).not.toContain("<Card");
    expect(doc.body).not.toContain("title=");
    expect(doc.body).not.toContain("href=");
    expect(doc.body).not.toContain("icon=");
    expect(doc.body).not.toContain("</Card>");
    expect(doc.body).toContain("WKB encoding, surface semantics.");
    expect(doc.body).toContain("Kept.");
  });

  it("strips single-line and multi-line MDX comments", () => {
    const doc = reduceMdx(
      "Text before.\n{/* single-line comment */}\nText middle.\n{/* multi\nline\ncomment */}\nText after.",
      { siteBaseUrl: SITE },
    );
    expect(doc.body).not.toContain("{/*");
    expect(doc.body).not.toContain("*/}");
    expect(doc.body).toContain("Text before.");
    expect(doc.body).toContain("Text middle.");
    expect(doc.body).toContain("Text after.");
  });

  it("preserves multi-line JSX inside fenced code blocks", () => {
    const doc = reduceMdx(
      "```jsx\n<Card\n  title=\"Example\"\n  href=\"/example\"\n>\nContent\n</Card>\n```\n\nKept.",
      { siteBaseUrl: SITE },
    );
    expect(doc.body).toContain("<Card");
    expect(doc.body).toContain("title=");
    expect(doc.body).toContain("href=");
    expect(doc.body).toContain("</Card>");
    expect(doc.body).toContain("Kept.");
  });

  it("preserves comments inside fenced code blocks", () => {
    const doc = reduceMdx(
      "```mdx\n{/* an example comment */}\n```\n\nKept.",
      { siteBaseUrl: SITE },
    );
    expect(doc.body).toContain("{/* an example comment */}");
    expect(doc.body).toContain("Kept.");
  });

  it("preserves comments spanning fence boundaries", () => {
    const doc = reduceMdx(
      "Before {/* open\n\n```sh\necho hi */} done\n```\n\nAfter.",
      { siteBaseUrl: SITE },
    );
    // Comment and both fence markers must survive
    expect(doc.body).toContain("{/* open");
    expect(doc.body).toContain("*/} done");
    expect(doc.body).toContain("```sh");
    expect(doc.body).toContain("echo hi");
    expect(doc.body).toContain("After.");
  });

  it("preserves JSX in later fenced blocks after fence-crossing comments", () => {
    const doc = reduceMdx(
      "Text {/* comment\n```\ncode */}\n```\n\nMore text.\n\n```jsx\n<Component prop=\"value\" />\n```\n\nEnd.",
      { siteBaseUrl: SITE },
    );
    // Later fence with JSX must survive (JSX inside fence must not be stripped)
    expect(doc.body).toContain("<Component");
    expect(doc.body).toContain("prop=");
    expect(doc.body).toContain("End.");
  });

  it("unwraps an inline JSX element with text content, keeping the text", () => {
    const doc = reduceMdx('<Badge variant="success">decided</Badge>\n\nKept.', { siteBaseUrl: SITE });
    expect(doc.body).toContain("decided");
    expect(doc.body).not.toContain("<Badge");
    expect(doc.body).not.toContain("</Badge>");
    expect(doc.body).toContain("Kept.");
  });

  it("removes an inline self-closing JSX element entirely", () => {
    const doc = reduceMdx('Before <Foo bar="1" /> after.', { siteBaseUrl: SITE });
    expect(doc.body).not.toContain("<Foo");
    expect(doc.body).toContain("Before");
    expect(doc.body).toContain("after.");
  });

  it("leaves LIST<> and STRUCT<> type notation inside backticks untouched", () => {
    const doc = reduceMdx(
      "The `parents` column is `LIST<VARCHAR>`.\n\nA struct: `STRUCT<type VARCHAR, shells LIST<LIST<INT>>>`.",
      { siteBaseUrl: SITE },
    );
    expect(doc.body).toContain("`LIST<VARCHAR>`");
    expect(doc.body).toContain("`STRUCT<type VARCHAR, shells LIST<LIST<INT>>>`");
  });

  it("leaves a bare, unclosed angle-bracket type outside backticks untouched", () => {
    const doc = reduceMdx("Types include LIST<VARCHAR>, STRUCT<...>, and <VARCHAR>.", { siteBaseUrl: SITE });
    expect(doc.body).toContain("LIST<VARCHAR>");
    expect(doc.body).toContain("STRUCT<...>");
    expect(doc.body).toContain("<VARCHAR>");
  });
});

describe("splitSections", () => {
  it("splits on headings and keeps the body of each", () => {
    const sections = splitSections("Intro.\n\n## One\n\nalpha\n\n### One-A\n\nbeta\n\n## Two\n\ngamma");
    expect(sections.map((s) => s.heading)).toEqual(["One", "One-A", "Two"]);
    expect(sections[0]!.level).toBe(2);
    expect(sections[1]!.level).toBe(3);
    expect(sections[2]!.body.trim()).toBe("gamma");
  });

  it("ignores a hash inside a fenced code block", () => {
    const sections = splitSections("## Real\n\n```sh\n## not a heading\n```\n");
    expect(sections.map((s) => s.heading)).toEqual(["Real"]);
  });
});
