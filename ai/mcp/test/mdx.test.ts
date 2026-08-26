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
