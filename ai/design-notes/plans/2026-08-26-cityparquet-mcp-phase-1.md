# CityParquet MCP — Phase 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A working stdio MCP server at `ai/mcp/` serving the CityParquet specification, the two DuckDB extension function references, dataset description and sandboxed SQL — five tools, backed by a generated documentation corpus.

**Architecture:** A generated `corpus.json` (built from `documents/docs/` and the two submodules' `FUNCTIONS.md`, committed so the server needs neither) is served by three pure documentation functions. A single DuckDB instance is brought up at startup with `httpfs`, `cityjson` and `three_d` loaded, optionally hardened into a read-only sandbox; `describe` and `query` run against it. The MCP layer is a thin registration file over pure, separately tested modules.

**Tech Stack:** TypeScript (ESM, Node 24), pnpm, vitest, `@modelcontextprotocol/server@2.0.0`, `@duckdb/node-api@1.5.4-r.1`, zod v4.

**Spec:** `ai/design-notes/specs/2026-08-26-cityparquet-mcp-and-skills-design.md` — read it before starting; this plan argues from it and does not restate its reasoning.

## Global Constraints

- **Node 24**, ESM only (`"type": "module"`). No CommonJS.
- **`@duckdb/node-api` pinned exactly to `1.5.4-r.1`** — not a caret range. It carries DuckDB v1.5.4, the only version where both `cityjson` and `three_d` exist in the community repository (spec §6.1). Same for `@modelcontextprotocol/server`: exactly `2.0.0`.
- **Extensions loaded: `httpfs`, `cityjson`, `three_d`. Never `spatial`** — it cannot coexist with `three_d` in either order (spec §6.4).
- **`extension_directory` is always set explicitly**, never inherited from `~/.duckdb`.
- **British English** in all prose, comments and documentation.
- **Every new `CLAUDE.md` has a byte-identical `AGENTS.md`.**
- **Breaking changes are welcome** — no shims, no deprecation paths. Update every call site.
- Tests are **vitest**, matching `documents/playground/playground.test.ts`.
- Commit messages follow the repository's Conventional Commits style (`feat(mcp): …`, `fix(mcp): …`, `docs(mcp): …`).

## File Structure

| File | Responsibility |
| --- | --- |
| `ai/mcp/package.json`, `tsconfig.json`, `tsconfig.build.json`, `vitest.config.ts` | Package, pinned deps, build and test config |
| `ai/mcp/src/mdx.ts` | MDX → Markdown reduction; frontmatter; section splitting; link rewriting |
| `ai/mcp/src/build-corpus.ts` | Reads sources, asserts order against `meta.ts`, writes `corpus/corpus.json` |
| `ai/mcp/src/corpus.ts` | Corpus types, loading, and the three pure query functions |
| `ai/mcp/src/sql.ts` | Statement splitting and cell-value elision |
| `ai/mcp/src/duckdb.ts` | Engine bring-up, extension loading, the sandbox sequence |
| `ai/mcp/src/tools/query.ts` | `cityparquet_query` logic |
| `ai/mcp/src/tools/describe.ts` | `cityparquet_describe` logic |
| `ai/mcp/src/server.ts` | Tool and resource registration; transport-agnostic |
| `ai/mcp/src/stdio.ts` | The stdio entry point |
| `ai/mcp/CLAUDE.md`, `AGENTS.md` | Package instructions |

Each `src/` module is pure and separately testable except `duckdb.ts` and `stdio.ts`. `server.ts` contains no logic — only registration — so the tools are tested without a transport.

---

### Task 1: Package scaffold and MDX reduction

**Files:**
- Create: `ai/mcp/package.json`, `ai/mcp/tsconfig.json`, `ai/mcp/vitest.config.ts`, `ai/mcp/.gitignore`
- Create: `ai/mcp/src/mdx.ts`
- Test: `ai/mcp/test/mdx.test.ts`

**Interfaces:**
- Consumes: nothing.
- Produces:
  ```ts
  export interface Section { heading: string; level: number; body: string }
  export interface ReducedDoc { title: string; description: string; body: string; sections: Section[] }
  export function reduceMdx(source: string, options: { siteBaseUrl: string }): ReducedDoc
  export function splitSections(markdown: string): Section[]
  ```

- [ ] **Step 1: Create the package**

`ai/mcp/package.json`:

```json
{
  "name": "@cityparquet/mcp",
  "version": "0.1.0",
  "private": true,
  "type": "module",
  "bin": { "cityparquet-mcp": "./dist/stdio.js" },
  "scripts": {
    "build": "tsc -p tsconfig.build.json",
    "corpus": "pnpm build && node dist/build-corpus.js",
    "test": "vitest run",
    "typecheck": "tsc -p tsconfig.json"
  },
  "dependencies": {
    "@duckdb/node-api": "1.5.4-r.1",
    "@modelcontextprotocol/server": "2.0.0",
    "zod": "^4.0.0"
  },
  "devDependencies": {
    "@types/node": "^26.1.1",
    "typescript": "^5.9.0",
    "vitest": "^3.2.0"
  }
}
```

Two configs, because the tests must be typechecked but must not be emitted.

`ai/mcp/tsconfig.json` — typechecks everything, emits nothing:

```json
{
  "compilerOptions": {
    "target": "ES2023",
    "module": "NodeNext",
    "moduleResolution": "NodeNext",
    "strict": true,
    "noUncheckedIndexedAccess": true,
    "resolveJsonModule": true,
    "skipLibCheck": true,
    "noEmit": true
  },
  "include": ["src", "test"]
}
```

`ai/mcp/tsconfig.build.json` — emits `src` alone:

```json
{
  "extends": "./tsconfig.json",
  "compilerOptions": {
    "noEmit": false,
    "declaration": true,
    "outDir": "dist",
    "rootDir": "src"
  },
  "include": ["src"]
}
```

`ai/mcp/vitest.config.ts`:

```ts
import { defineConfig } from "vitest/config";

export default defineConfig({
  test: { include: ["test/**/*.test.ts"], testTimeout: 120_000 },
});
```

`ai/mcp/.gitignore`:

```
dist/
node_modules/
extensions/
```

Run: `cd ai/mcp && pnpm install`
Expected: lockfile written, `@duckdb/node-api` resolved to exactly `1.5.4-r.1`.

- [ ] **Step 2: Write the failing test**

`ai/mcp/test/mdx.test.ts`:

```ts
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
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cd ai/mcp && pnpm vitest run test/mdx.test.ts`
Expected: FAIL — cannot resolve `../src/mdx.js`.

- [ ] **Step 4: Implement `src/mdx.ts`**

```ts
// Reducing the documentation site's MDX to the Markdown the corpus carries.

export interface Section {
  readonly heading: string;
  readonly level: number;
  readonly body: string;
}

export interface ReducedDoc {
  readonly title: string;
  readonly description: string;
  readonly body: string;
  readonly sections: readonly Section[];
}

const FRONTMATTER = /^---\r?\n([\s\S]*?)\r?\n---\r?\n?/;

/** Only the two scalar keys the corpus needs; the sidebar block is ignored. */
function frontmatterField(block: string, key: string): string {
  const match = new RegExp(`^${key}:\\s*(.+)$`, "m").exec(block);
  if (!match) return "";
  return match[1]!.trim().replace(/^["']|["']$/g, "");
}

/**
 * Blume admonitions carry normative text often enough that dropping them would
 * lose specification content, so they are unwrapped rather than stripped.
 */
function unwrapAdmonitions(markdown: string): string {
  return markdown.replace(
    /^:::[a-z]+(?:\[([^\]]*)\])?\r?\n([\s\S]*?)^:::[ \t]*$/gm,
    (_all, title: string | undefined, body: string) =>
      title ? `**${title}**\n\n${body.trim()}\n` : `${body.trim()}\n`,
  );
}

function stripJsx(markdown: string): string {
  return markdown
    .replace(/^import\s+[^\n]*\n/gm, "")
    .replace(/^export\s+[^\n]*\n/gm, "")
    .replace(/^[ \t]*<\/?[A-Z][\w.]*(?:\s[^>]*)?\/?>[ \t]*$/gm, "");
}

function absolutiseLinks(markdown: string, siteBaseUrl: string): string {
  return markdown.replace(/\]\((\/[^)\s]*)\)/g, (_all, path: string) => `](${siteBaseUrl}${path})`);
}

/** Heading scan that respects fenced code blocks, where `##` is not a heading. */
export function splitSections(markdown: string): Section[] {
  const lines = markdown.split(/\r?\n/);
  const sections: Section[] = [];
  let current: { heading: string; level: number; body: string[] } | null = null;
  let fence: string | null = null;

  for (const line of lines) {
    const fenceMatch = /^\s*(```+|~~~+)/.exec(line);
    if (fenceMatch) {
      const marker = fenceMatch[1]!;
      if (fence === null) fence = marker;
      else if (marker.startsWith(fence[0]!) && marker.length >= fence.length) fence = null;
    }

    const heading = fence === null ? /^(#{2,6})\s+(.*)$/.exec(line) : null;
    if (heading) {
      if (current) sections.push({ ...current, body: current.body.join("\n") });
      current = { heading: heading[2]!.trim(), level: heading[1]!.length, body: [] };
      continue;
    }
    current?.body.push(line);
  }
  if (current) sections.push({ ...current, body: current.body.join("\n") });
  return sections;
}

export function reduceMdx(source: string, options: { siteBaseUrl: string }): ReducedDoc {
  const frontmatter = FRONTMATTER.exec(source);
  const block = frontmatter?.[1] ?? "";
  const rest = frontmatter ? source.slice(frontmatter[0].length) : source;

  const body = absolutiseLinks(stripJsx(unwrapAdmonitions(rest)), options.siteBaseUrl)
    .replace(/\n{3,}/g, "\n\n")
    .trim();

  return {
    title: frontmatterField(block, "title"),
    description: frontmatterField(block, "description"),
    body,
    sections: splitSections(body),
  };
}
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cd ai/mcp && pnpm vitest run test/mdx.test.ts`
Expected: PASS, 7 tests.

- [ ] **Step 6: Commit**

```bash
git add ai/mcp/package.json ai/mcp/tsconfig.json ai/mcp/tsconfig.build.json ai/mcp/vitest.config.ts ai/mcp/.gitignore ai/mcp/pnpm-lock.yaml ai/mcp/src/mdx.ts ai/mcp/test/mdx.test.ts
git commit -m "feat(mcp): reduce documentation MDX to corpus Markdown"
```

---

### Task 2: Corpus types and queries — outline, search, read

**Files:**
- Create: `ai/mcp/src/corpus.ts`
- Test: `ai/mcp/test/corpus.test.ts`

**Interfaces:**
- Consumes: `splitSections` from `src/mdx.ts`.
- Produces — **this module owns the corpus types**; Task 3's build script imports them rather than redeclaring them:
  ```ts
  export type CorpusId = "spec" | "duckdb-cityjson" | "duckdb-3d"
  export const CORPUS_IDS: readonly CorpusId[]
  export interface Chapter { id: string; title: string; description: string; order: number; sections: readonly { heading: string; level: number }[]; body: string }
  export interface CorpusEntry { title: string; source: string; chapters: readonly Chapter[] }
  export interface Corpus { generatedFrom: string; corpora: Readonly<Record<CorpusId, CorpusEntry>> }
  export function loadCorpus(path?: string): Corpus
  export function outline(corpus: Corpus, corpusId?: CorpusId): OutlineResult
  export function search(corpus: Corpus, query: string, options?: { corpus?: CorpusId; limit?: number }): SearchHit[]
  export function readChapter(corpus: Corpus, corpusId: CorpusId, chapterId: string, section?: string): string
  export interface SearchHit { corpus: CorpusId; chapter: string; title: string; heading: string | null; snippet: string }
  export interface OutlineResult { corpora: { id: CorpusId; title: string; chapters: { id: string; title: string; description: string; sections: string[] }[] }[] }
  ```

- [ ] **Step 1: Write the failing test**

`ai/mcp/test/corpus.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { outline, readChapter, search } from "../src/corpus.js";
import type { Corpus } from "../src/corpus.js";

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
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd ai/mcp && pnpm vitest run test/corpus.test.ts`
Expected: FAIL — cannot resolve `../src/corpus.js`.

- [ ] **Step 3: Implement `src/corpus.ts`**

```ts
// The corpus, and the three ways a tool asks it anything.

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { splitSections } from "./mdx.js";

export type CorpusId = "spec" | "duckdb-cityjson" | "duckdb-3d";
export const CORPUS_IDS: readonly CorpusId[] = ["spec", "duckdb-cityjson", "duckdb-3d"];

export interface Chapter {
  readonly id: string;
  readonly title: string;
  readonly description: string;
  readonly order: number;
  readonly sections: readonly { heading: string; level: number }[];
  readonly body: string;
}

export interface CorpusEntry {
  readonly title: string;
  readonly source: string;
  readonly chapters: readonly Chapter[];
}

export interface Corpus {
  readonly generatedFrom: string;
  readonly corpora: Readonly<Record<CorpusId, CorpusEntry>>;
}

export interface OutlineResult {
  readonly corpora: readonly {
    id: CorpusId;
    title: string;
    chapters: { id: string; title: string; description: string; sections: string[] }[];
  }[];
}

export interface SearchHit {
  readonly corpus: CorpusId;
  readonly chapter: string;
  readonly title: string;
  readonly heading: string | null;
  readonly snippet: string;
}

export function loadCorpus(path?: string): Corpus {
  const resolved =
    path ?? join(dirname(fileURLToPath(import.meta.url)), "..", "corpus", "corpus.json");
  return JSON.parse(readFileSync(resolved, "utf8")) as Corpus;
}

function entries(corpus: Corpus, corpusId?: CorpusId): [CorpusId, CorpusEntry][] {
  const ids = corpusId ? [corpusId] : CORPUS_IDS;
  return ids.map((id) => {
    const entry = corpus.corpora[id];
    if (!entry) throw new Error(`unknown corpus "${id}"; known: ${CORPUS_IDS.join(", ")}`);
    return [id, entry];
  });
}

/** Headings and one-line descriptions only — never a body. */
export function outline(corpus: Corpus, corpusId?: CorpusId): OutlineResult {
  return {
    corpora: entries(corpus, corpusId).map(([id, entry]) => ({
      id,
      title: entry.title,
      chapters: entry.chapters.map((c) => ({
        id: c.id,
        title: c.title,
        description: c.description,
        sections: c.sections.map((s) => s.heading),
      })),
    })),
  };
}

const SNIPPET_RADIUS = 160;

function snippetAround(body: string, index: number): string {
  const start = Math.max(0, index - SNIPPET_RADIUS);
  const end = Math.min(body.length, index + SNIPPET_RADIUS);
  return `${start > 0 ? "…" : ""}${body.slice(start, end).replace(/\s+/g, " ").trim()}${end < body.length ? "…" : ""}`;
}

/**
 * A case-folded scan, deliberately. The whole corpus is a few hundred kilobytes;
 * an index would be more code, another artefact to keep fresh, and no faster at
 * this size.
 */
export function search(
  corpus: Corpus,
  query: string,
  options: { corpus?: CorpusId; limit?: number } = {},
): SearchHit[] {
  const needle = query.toLowerCase();
  const limit = options.limit ?? 10;
  const hits: SearchHit[] = [];

  for (const [id, entry] of entries(corpus, options.corpus)) {
    for (const chapter of entry.chapters) {
      const units = [
        { heading: null as string | null, body: chapter.body.split(/^#{2,6}\s/m)[0] ?? "" },
        ...splitSections(chapter.body).map((s) => ({ heading: s.heading, body: s.body })),
      ];
      for (const unit of units) {
        const index = unit.body.toLowerCase().indexOf(needle);
        if (index === -1) continue;
        hits.push({
          corpus: id,
          chapter: chapter.id,
          title: chapter.title,
          heading: unit.heading,
          snippet: snippetAround(unit.body, index),
        });
        if (hits.length >= limit) return hits;
      }
    }
  }
  return hits;
}

export function readChapter(
  corpus: Corpus,
  corpusId: CorpusId,
  chapterId: string,
  section?: string,
): string {
  const [[, entry]] = entries(corpus, corpusId);
  const chapter = entry.chapters.find((c) => c.id === chapterId);
  if (!chapter) {
    throw new Error(
      `unknown chapter "${chapterId}" in corpus "${corpusId}"; known: ${entry.chapters.map((c) => c.id).join(", ")}`,
    );
  }
  if (section === undefined) return chapter.body;

  const found = splitSections(chapter.body).find(
    (s) => s.heading.toLowerCase() === section.toLowerCase(),
  );
  if (!found) {
    throw new Error(
      `unknown section "${section}" in "${chapterId}"; known: ${chapter.sections.map((s) => s.heading).join(", ")}`,
    );
  }
  return `## ${found.heading}\n\n${found.body.trim()}`;
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd ai/mcp && pnpm vitest run test/corpus.test.ts`
Expected: PASS, 11 tests.

- [ ] **Step 5: Commit**

```bash
git add ai/mcp/src/corpus.ts ai/mcp/test/corpus.test.ts
git commit -m "feat(mcp): outline, search and read over the documentation corpus"
```

---

### Task 3: The corpus build script

**Files:**
- Create: `ai/mcp/src/build-corpus.ts`
- Create: `ai/mcp/corpus/corpus.json` (generated, committed)
- Modify: `justfile` (repository root) — add `mcp-corpus`
- Test: `ai/mcp/test/build-corpus.test.ts`

**Interfaces:**
- Consumes: `reduceMdx`, `splitSections` from `src/mdx.ts`; the `Corpus`, `CorpusEntry`, `Chapter` and `CorpusId` types from `src/corpus.ts`. **Do not redeclare them here** — one owner, one definition.
- Produces:
  ```ts
  export function chapterIdFromFilename(filename: string): string
  export function declaredOrder(metaSource: string): string[]
  export function buildCorpus(repoRoot: string, gitStamp: string): Corpus
  ```

- [ ] **Step 1: Write the failing test**

`ai/mcp/test/build-corpus.test.ts`:

```ts
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
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd ai/mcp && pnpm vitest run test/build-corpus.test.ts`
Expected: FAIL — cannot resolve `../src/build-corpus.js`.

- [ ] **Step 3: Implement `src/build-corpus.ts`**

```ts
// documents/docs + the two FUNCTIONS.md -> corpus/corpus.json.
//
// It lives in src/ rather than scripts/ so that one tsconfig typechecks it and
// its imports resolve against the built output — `pnpm corpus` builds first and
// runs `dist/build-corpus.js`.
//
// Committed output. lib/duckdb-cityjson and lib/duckdb-3d are submodules, so a
// server that read them at request time would serve two of its three corpora
// only in a fully checked-out clone — and never in the container image.

import { execFileSync } from "node:child_process";
import { readdirSync, readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import type { Chapter, Corpus, CorpusEntry } from "./corpus.js";
import { reduceMdx, splitSections } from "./mdx.js";

const SITE_BASE_URL = "https://cityparquet.open3d.city";

export function chapterIdFromFilename(filename: string): string {
  return filename.replace(/\.mdx?$/, "").replace(/^\d+-/, "");
}

/**
 * The `pages` array from a Blume `meta.ts`, read as text rather than imported —
 * importing would mean evaluating `defineMeta` from `blume`, which this package
 * does not depend on and should not.
 */
export function declaredOrder(metaSource: string): string[] {
  const block = /pages:\s*\[([\s\S]*?)\]/.exec(metaSource);
  if (!block) return [];
  return [...block[1]!.matchAll(/["']([^"']+)["']/g)].map((m) => m[1]!);
}

function mdxDirectory(root: string, relative: string): Chapter[] {
  const directory = join(root, relative);
  const files = readdirSync(directory)
    .filter((f) => f.endsWith(".mdx"))
    .sort();

  const chapters = files.map((file, index) => {
    const doc = reduceMdx(readFileSync(join(directory, file), "utf8"), { siteBaseUrl: SITE_BASE_URL });
    return {
      id: chapterIdFromFilename(file),
      title: doc.title || chapterIdFromFilename(file),
      description: doc.description,
      order: index,
      sections: doc.sections.map(({ heading, level }) => ({ heading, level })),
      body: doc.body,
    };
  });

  // The filename prefixes carry the order; meta.ts declares it too. They agree
  // today, and a future disagreement should be a build failure rather than a
  // corpus whose order silently diverges from the published sidebar.
  let declared: string[] = [];
  try {
    declared = declaredOrder(readFileSync(join(directory, "meta.ts"), "utf8"));
  } catch {
    declared = [];
  }
  if (declared.length > 0) {
    const fromFiles = chapters.map((c) => c.id).filter((id) => id !== "index");
    if (JSON.stringify(fromFiles) !== JSON.stringify(declared)) {
      throw new Error(
        `${relative}: filename order [${fromFiles.join(", ")}] disagrees with meta.ts [${declared.join(", ")}]`,
      );
    }
  }
  return chapters;
}

/** A FUNCTIONS.md normalises to the same two levels: `##` chapter, `###` section. */
function functionsReference(root: string, relative: string, title: string): CorpusEntry {
  const source = readFileSync(join(root, relative), "utf8");
  const tops = splitSections(source).filter((s) => s.level === 2);

  const chapters = tops.map((section, index) => ({
    id: section.heading.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, ""),
    title: section.heading,
    description: "",
    order: index,
    sections: splitSections(section.body)
      .filter((s) => s.level === 3)
      .map(({ heading, level }) => ({ heading, level })),
    body: section.body.trim(),
  }));

  return { title, source: relative, chapters };
}

export function buildCorpus(repoRoot: string, gitStamp: string): Corpus {
  return {
    generatedFrom: gitStamp,
    corpora: {
      spec: {
        title: "CityParquet specification",
        source: "documents/docs/03-specification, documents/docs/04-design-decisions",
        chapters: [
          ...mdxDirectory(repoRoot, "documents/docs/03-specification"),
          ...mdxDirectory(repoRoot, "documents/docs/04-design-decisions").map((c) => ({
            ...c,
            id: `design-${c.id}`,
          })),
        ].map((chapter, order) => ({ ...chapter, order })),
      },
      "duckdb-cityjson": functionsReference(
        repoRoot,
        "lib/duckdb-cityjson/docs/FUNCTIONS.md",
        "duckdb-cityjson function reference",
      ),
      "duckdb-3d": functionsReference(
        repoRoot,
        "lib/duckdb-3d/docs/FUNCTIONS.md",
        "duckdb-3d function reference",
      ),
    },
  };
}

if (import.meta.url === `file://${process.argv[1]}`) {
  // Running from dist/: up to ai/mcp, then ai, then the repository root.
  const here = dirname(fileURLToPath(import.meta.url));
  const repoRoot = join(here, "..", "..", "..");
  const stamp = execFileSync("git", ["describe", "--always", "--dirty"], { cwd: repoRoot })
    .toString()
    .trim();
  const corpus = buildCorpus(repoRoot, stamp);
  mkdirSync(join(here, "..", "corpus"), { recursive: true });
  writeFileSync(join(here, "..", "corpus", "corpus.json"), `${JSON.stringify(corpus, null, 2)}\n`);
  const counts = Object.entries(corpus.corpora)
    .map(([id, entry]) => `${id}: ${entry.chapters.length}`)
    .join(", ");
  process.stderr.write(`corpus.json written (${counts})\n`);
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd ai/mcp && pnpm vitest run test/build-corpus.test.ts`
Expected: PASS, 3 tests.

- [ ] **Step 5: Generate the corpus for real**

The two `FUNCTIONS.md` live in submodules. If `lib/duckdb-cityjson/docs/` is empty, run `just setup` from the repository root first.

Run: `cd ai/mcp && pnpm corpus`
Expected: `corpus.json written (spec: 15, duckdb-cityjson: N, duckdb-3d: N)` on stderr. If it throws a "disagrees with meta.ts" error, that is the assertion working — reconcile the filenames with `meta.ts` rather than weakening the check.

- [ ] **Step 6: Add the justfile recipe**

Append to the repository-root `justfile`:

```just
# Regenerate the MCP server's documentation corpus from documents/docs and the
# two extension function references. Needs the submodules — `just setup` first.
mcp-corpus:
    cd ai/mcp && pnpm install --frozen-lockfile && pnpm corpus
```

Run: `just mcp-corpus`
Expected: same output, from the repository root.

- [ ] **Step 7: Commit**

```bash
git add ai/mcp/src/build-corpus.ts ai/mcp/test/build-corpus.test.ts ai/mcp/corpus/corpus.json justfile
git commit -m "feat(mcp): build the documentation corpus from the docs and function references"
```

---

### Task 4: SQL statement splitting and value elision

**Files:**
- Create: `ai/mcp/src/sql.ts`
- Test: `ai/mcp/test/sql.test.ts`

**Interfaces:**
- Consumes: nothing.
- Produces:
  ```ts
  export function splitStatements(sql: string): string[]
  export function elideCell(value: unknown, type: string, maxBytes: number): unknown
  ```

**Why hand-written.** `DuckDBConnection.extractStatements` exists and cannot be used: it resolves pragmas at extract time, so a script containing `PRAGMA cityparquet_init(…)` throws *"Catalog Error: Pragma Function with name … does not exist"* during extraction, before a single statement runs. That was reproduced directly. The splitter must be textual.

- [ ] **Step 1: Write the failing test**

`ai/mcp/test/sql.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { elideCell, splitStatements } from "../src/sql.js";

describe("splitStatements", () => {
  it("splits on semicolons", () => {
    expect(splitStatements("SELECT 1; SELECT 2")).toEqual(["SELECT 1", "SELECT 2"]);
  });

  it("ignores a semicolon inside a single-quoted string", () => {
    expect(splitStatements("SELECT 'a;b'")).toEqual(["SELECT 'a;b'"]);
  });

  it("handles the doubled quotes in a cityparquet_delete predicate", () => {
    const sql = "PRAGMA cityparquet_delete('delft', 'object_type = ''Building''');\nSELECT 1";
    expect(splitStatements(sql)).toEqual([
      "PRAGMA cityparquet_delete('delft', 'object_type = ''Building''')",
      "SELECT 1",
    ]);
  });

  it("ignores a semicolon inside a double-quoted identifier", () => {
    expect(splitStatements('SELECT 1 AS "a;b"')).toEqual(['SELECT 1 AS "a;b"']);
  });

  it("ignores a semicolon inside a dollar-quoted string", () => {
    expect(splitStatements("SELECT $$a;b$$")).toEqual(["SELECT $$a;b$$"]);
  });

  it("ignores a semicolon in a line comment", () => {
    expect(splitStatements("SELECT 1 -- a;b\n; SELECT 2")).toEqual(["SELECT 1 -- a;b", "SELECT 2"]);
  });

  it("ignores a semicolon in a block comment", () => {
    expect(splitStatements("SELECT 1 /* a;b */; SELECT 2")).toEqual([
      "SELECT 1 /* a;b */",
      "SELECT 2",
    ]);
  });

  it("drops empty statements and trailing semicolons", () => {
    expect(splitStatements("SELECT 1;;\n  \n")).toEqual(["SELECT 1"]);
  });
});

describe("elideCell", () => {
  it("always elides a blob, reporting its length", () => {
    expect(elideCell("abcd", "BLOB", 1024)).toBe("<BLOB 4 bytes>");
  });

  it("truncates an oversized string", () => {
    expect(elideCell("x".repeat(50), "VARCHAR", 10)).toBe("<VARCHAR 50 bytes>");
  });

  it("leaves a small value alone", () => {
    expect(elideCell("short", "VARCHAR", 256)).toBe("short");
    expect(elideCell(42, "INTEGER", 256)).toBe(42);
    expect(elideCell(null, "VARCHAR", 256)).toBeNull();
  });

  it("elides an oversized nested value by its serialised size", () => {
    const big = { a: "y".repeat(100) };
    expect(elideCell(big, "STRUCT(a VARCHAR)", 20)).toMatch(/^<STRUCT\(a VARCHAR\) \d+ bytes>$/);
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd ai/mcp && pnpm vitest run test/sql.test.ts`
Expected: FAIL — cannot resolve `../src/sql.js`.

- [ ] **Step 3: Implement `src/sql.ts`**

```ts
// Splitting a script into statements, and keeping a result from flooding the
// caller's context.

/**
 * DuckDB expands every pragma in a submitted script before running any of it,
 * so the package workflow — `CREATE SCHEMA d;` then `PRAGMA cityparquet_init('d')`
 * — fails outright when submitted as one script. Statements therefore run one
 * at a time, which means splitting them here.
 *
 * `DuckDBConnection.extractStatements` cannot do this: it resolves pragmas at
 * extract time and throws on the very scripts that need splitting.
 */
export function splitStatements(sql: string): string[] {
  const statements: string[] = [];
  let start = 0;
  let i = 0;

  while (i < sql.length) {
    const ch = sql[i]!;

    if (ch === "'" || ch === '"') {
      const quote = ch;
      i += 1;
      while (i < sql.length) {
        if (sql[i] === quote) {
          if (sql[i + 1] === quote) i += 2; // a doubled quote is an escaped one
          else { i += 1; break; }
        } else i += 1;
      }
      continue;
    }

    if (ch === "$") {
      const tag = /^\$[A-Za-z_]*\$/.exec(sql.slice(i));
      if (tag) {
        const marker = tag[0];
        const end = sql.indexOf(marker, i + marker.length);
        i = end === -1 ? sql.length : end + marker.length;
        continue;
      }
    }

    if (ch === "-" && sql[i + 1] === "-") {
      const end = sql.indexOf("\n", i);
      i = end === -1 ? sql.length : end;
      continue;
    }

    if (ch === "/" && sql[i + 1] === "*") {
      const end = sql.indexOf("*/", i + 2);
      i = end === -1 ? sql.length : end + 2;
      continue;
    }

    if (ch === ";") {
      const statement = sql.slice(start, i).trim();
      if (statement) statements.push(statement);
      i += 1;
      start = i;
      continue;
    }

    i += 1;
  }

  const tail = sql.slice(start).trim();
  if (tail) statements.push(tail);
  return statements;
}

/**
 * A single LoD2 solid's WKB runs to kilobytes, and `SELECT *` on an object
 * table returns one per row — so an un-elided result is at its most destructive
 * on the most obvious query anyone will write. The `other` overflow column and
 * long attribute strings are the same bomb by a different route, which is why
 * this is not blob-only.
 *
 * The type is passed in rather than sniffed: `getRowsJson()` renders a BLOB as
 * an ordinary string, so the value alone cannot tell you what it was.
 */
export function elideCell(value: unknown, type: string, maxBytes: number): unknown {
  if (value === null || value === undefined) return value;

  if (type.toUpperCase().startsWith("BLOB")) {
    const length = typeof value === "string" ? Buffer.byteLength(value, "utf8") : String(value).length;
    return `<BLOB ${length} bytes>`;
  }

  if (typeof value === "number" || typeof value === "boolean") return value;

  const text = typeof value === "string" ? value : JSON.stringify(value);
  const length = Buffer.byteLength(text, "utf8");
  return length > maxBytes ? `<${type} ${length} bytes>` : value;
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd ai/mcp && pnpm vitest run test/sql.test.ts`
Expected: PASS, 12 tests.

- [ ] **Step 5: Commit**

```bash
git add ai/mcp/src/sql.ts ai/mcp/test/sql.test.ts
git commit -m "feat(mcp): split SQL scripts textually and elide oversized cells"
```

---

### Task 5: The DuckDB engine and its sandbox

**Files:**
- Create: `ai/mcp/src/duckdb.ts`
- Test: `ai/mcp/test/duckdb.test.ts`

**Interfaces:**
- Consumes: nothing.
- Produces:
  ```ts
  export const DEFAULT_EXTENSIONS: readonly string[]  // ["httpfs", "cityjson", "three_d"]
  export interface EngineOptions { sandbox: boolean; extensionDirectory: string; extensions?: readonly string[]; memoryLimit?: string; threads?: number }
  export interface Engine { readonly connection: DuckDBConnection; readonly extensions: readonly { name: string; version: string }[]; close(): Promise<void> }
  export function createEngine(options: EngineOptions): Promise<Engine>
  ```

**The ordering is not stylistic.** Extensions install and load from disk; the sandbox disables the local filesystem. Reverse them and the server cannot load its own extensions.

**`spatial` is never in the set.** It cannot coexist with `three_d` — `spatial` then `three_d` fails with *"Cannot AlterEntry without client context"*, and `three_d` then `spatial` fails with *"Scalar Function with name …"*. Both were reproduced.

- [ ] **Step 1: Write the failing test**

`ai/mcp/test/duckdb.test.ts`:

```ts
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { createEngine, DEFAULT_EXTENSIONS, type Engine } from "../src/duckdb.js";

const extensionDirectory = join(mkdtempSync(join(tmpdir(), "cityparquet-mcp-")), "extensions");

describe("createEngine", () => {
  let engine: Engine;
  beforeAll(async () => {
    engine = await createEngine({ sandbox: true, extensionDirectory, memoryLimit: "2GB", threads: 4 });
  });
  afterAll(async () => { await engine?.close(); });

  it("loads exactly the default extensions, and never spatial", () => {
    expect(engine.extensions.map((e) => e.name).sort()).toEqual([...DEFAULT_EXTENSIONS].sort());
    expect(engine.extensions.map((e) => e.name)).not.toContain("spatial");
  });

  it("runs DuckDB v1.5.4", async () => {
    const reader = await engine.connection.runAndReadAll("SELECT version() AS v");
    expect(reader.getRowsJson()[0]![0]).toBe("v1.5.4");
  });

  // The security contract. A change that makes one of these pass is a
  // regression, not a test failure.
  const blocked = [
    ["local csv read", "SELECT * FROM read_csv('/etc/passwd')"],
    ["local attach", "ATTACH '/tmp/mcp-probe.db'"],
    ["local copy out", "COPY (SELECT 1) TO '/tmp/mcp-probe.parquet'"],
    ["local parquet read", "SELECT * FROM read_parquet('/etc/hostname')"],
    ["extension install", "INSTALL json"],
    ["unlocking the filesystem", "SET disabled_filesystems = ''"],
    ["raising the memory limit", "SET memory_limit = '400GB'"],
    ["unlocking the configuration", "SET lock_configuration = false"],
  ] as const;

  for (const [name, sql] of blocked) {
    it(`blocks ${name}`, async () => {
      await expect(engine.connection.run(sql)).rejects.toThrow();
    });
  }

  // Not a hole. `json` is statically linked into the DuckDB binary, so loading
  // it reads no file. The property the sandbox provides is that only extensions
  // already in the binary can be loaded — every other one needs a disk read.
  it("permits LOAD of a statically linked extension", async () => {
    await expect(engine.connection.run("LOAD json")).resolves.toBeDefined();
  });

  it("still reads over HTTPS", async () => {
    const reader = await engine.connection.runAndReadAll(
      "SELECT version FROM cityjsonseq_metadata('https://cityjson.open3d.city/cityjsonseq/delft.city.jsonl')",
    );
    expect(reader.getRowsJson()[0]![0]).toBe("2.0");
  });
});

describe("createEngine without the sandbox", () => {
  it("leaves the local filesystem reachable", async () => {
    const engine = await createEngine({ sandbox: false, extensionDirectory });
    await expect(engine.connection.run("SELECT * FROM read_csv('/etc/hostname')")).resolves.toBeDefined();
    await engine.close();
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd ai/mcp && pnpm vitest run test/duckdb.test.ts`
Expected: FAIL — cannot resolve `../src/duckdb.js`.

- [ ] **Step 3: Implement `src/duckdb.ts`**

```ts
// Bringing DuckDB up with the CityParquet extensions, and shutting the doors
// behind it.

import { DuckDBConnection, DuckDBInstance } from "@duckdb/node-api";

/**
 * `spatial` is deliberately absent. It cannot be loaded alongside `three_d` in
 * either order — `spatial` first breaks `three_d` with "Cannot AlterEntry
 * without client context", `three_d` first breaks `spatial` with "Scalar
 * Function with name …". The playground's extension list has never included it
 * either, so this is existing practice made explicit.
 *
 * The cost, which the skills must state: no ST_Area, no ST_GeomFromWKB, none of
 * the 2D vocabulary. ST_3DFootprintArea is the substitute.
 */
export const DEFAULT_EXTENSIONS = ["httpfs", "cityjson", "three_d"] as const;

const COMMUNITY_EXTENSIONS = new Set(["cityjson", "three_d"]);

export interface EngineOptions {
  /** Hosted deployments: true. Local stdio: false — the user's own machine is the trust boundary. */
  readonly sandbox: boolean;
  /**
   * Always explicit, never `~/.duckdb`. A shared directory may hold artefacts
   * built for another DuckDB version, and the failure is an opaque
   * initialisation error at LOAD time.
   */
  readonly extensionDirectory: string;
  readonly extensions?: readonly string[];
  readonly memoryLimit?: string;
  readonly threads?: number;
}

export interface Engine {
  readonly connection: DuckDBConnection;
  readonly extensions: readonly { name: string; version: string }[];
  close(): Promise<void>;
}

export async function createEngine(options: EngineOptions): Promise<Engine> {
  const wanted = options.extensions ?? DEFAULT_EXTENSIONS;

  // 1. The instance.
  const instance = await DuckDBInstance.create(":memory:", {
    extension_directory: options.extensionDirectory,
  });
  const connection = await instance.connect();

  // 2. Extensions — this touches the local filesystem, so it must finish
  //    before step 5 disables it.
  for (const name of wanted) {
    const from = COMMUNITY_EXTENSIONS.has(name) ? " FROM community" : "";
    await connection.run(`INSTALL ${name}${from}`);
    await connection.run(`LOAD ${name}`);
  }

  // 3. Resource limits.
  if (options.memoryLimit) await connection.run(`SET memory_limit = '${options.memoryLimit}'`);
  if (options.threads !== undefined) await connection.run(`SET threads = ${options.threads}`);

  if (options.sandbox) {
    // 4. Close the doors.
    for (const setting of [
      "autoinstall_known_extensions",
      "autoload_known_extensions",
      "allow_community_extensions",
      "allow_persistent_secrets",
    ]) {
      await connection.run(`SET ${setting} = false`);
    }

    // 5. No local filesystem. A query that would spill to a temporary file now
    //    fails rather than spilling — the right trade for a public endpoint,
    //    and the reason memory_limit should be generous.
    await connection.run("SET disabled_filesystems = 'LocalFileSystem'");

    // 6. And none of the above can be undone by a query.
    await connection.run("SET lock_configuration = true");
  }

  const reader = await connection.runAndReadAll(
    `SELECT extension_name, extension_version FROM duckdb_extensions()
     WHERE loaded AND extension_name IN (${wanted.map((n) => `'${n}'`).join(", ")})`,
  );
  const extensions = reader.getRowsJson().map((row) => ({
    name: String(row[0]),
    version: String(row[1]),
  }));

  const missing = wanted.filter((n) => !extensions.some((e) => e.name === n));
  if (missing.length > 0) {
    throw new Error(`extensions failed to load: ${missing.join(", ")}`);
  }

  // 7. Warm the HTTP path so the first real request does not pay for TLS and
  //    httpfs initialisation.
  await connection.run("SELECT 1");

  return {
    connection,
    extensions,
    async close() {
      connection.closeSync();
    },
  };
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd ai/mcp && pnpm vitest run test/duckdb.test.ts`
Expected: PASS, 13 tests. The HTTPS test needs network; if it is the only failure, the sandbox is still correct.

- [ ] **Step 5: Commit**

```bash
git add ai/mcp/src/duckdb.ts ai/mcp/test/duckdb.test.ts
git commit -m "feat(mcp): bring up DuckDB with the CityParquet extensions and a locked sandbox"
```

---

### Task 6: The query tool

**Files:**
- Create: `ai/mcp/src/tools/query.ts`
- Test: `ai/mcp/test/query.test.ts`

**Interfaces:**
- Consumes: `Engine` from `src/duckdb.ts`; `splitStatements`, `elideCell` from `src/sql.ts`.
- Produces:
  ```ts
  export interface QueryOptions { maxRows: number; maxCellBytes: number; timeoutMs: number }
  export const QUERY_DEFAULTS: QueryOptions  // { maxRows: 100, maxCellBytes: 256, timeoutMs: 120_000 }
  export interface StatementResult { statement: string; columns?: { name: string; type: string }[]; rows?: unknown[][]; rowCount?: number; truncated?: boolean; elapsedMs?: number; error?: string }
  // rowCount is rows RETURNED, not rows matched — the cap is enforced by reading
  // only maxRows + 1, so the true total is never learned.
  export function runQuery(engine: Engine, sql: string, options?: Partial<QueryOptions>): Promise<StatementResult[]>
  ```

- [ ] **Step 1: Write the failing test**

`ai/mcp/test/query.test.ts`:

```ts
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { createEngine, type Engine } from "../src/duckdb.js";
import { runQuery } from "../src/tools/query.js";

const extensionDirectory = join(mkdtempSync(join(tmpdir(), "cityparquet-mcp-q-")), "extensions");

describe("runQuery", () => {
  let engine: Engine;
  beforeAll(async () => {
    engine = await createEngine({ sandbox: true, extensionDirectory, memoryLimit: "2GB", threads: 4 });
  });
  afterAll(async () => { await engine?.close(); });

  it("returns columns, types and rows", async () => {
    const [result] = await runQuery(engine, "SELECT 1 AS a, 'x' AS b");
    expect(result!.columns).toEqual([{ name: "a", type: "INTEGER" }, { name: "b", type: "VARCHAR" }]);
    expect(result!.rows).toEqual([[1, "x"]]);
    expect(result!.rowCount).toBe(1);
    expect(result!.truncated).toBe(false);
    expect(typeof result!.elapsedMs).toBe("number");
  });

  it("runs statements one at a time and returns one result each", async () => {
    const results = await runQuery(engine, "SELECT 1 AS a; SELECT 2 AS a");
    expect(results).toHaveLength(2);
    expect(results[1]!.rows).toEqual([[2]]);
  });

  it("stops at the first error and reports it", async () => {
    const results = await runQuery(engine, "SELECT 1; SELECT * FROM nope; SELECT 3");
    expect(results).toHaveLength(2);
    expect(results[1]!.error).toMatch(/nope/);
    expect(results[1]!.rows).toBeUndefined();
  });

  it("caps rows and says so, without reading the rest", async () => {
    const [result] = await runQuery(engine, "SELECT * FROM range(500)", { maxRows: 10 });
    expect(result!.rows).toHaveLength(10);
    expect(result!.rowCount).toBe(10); // rows returned, not rows matched
    expect(result!.truncated).toBe(true);
  });

  it("elides a blob rather than returning it", async () => {
    const [result] = await runQuery(engine, "SELECT 'abcd'::BLOB AS g");
    expect(result!.rows![0]![0]).toBe("<BLOB 4 bytes>");
  });

  it("elides an oversized string", async () => {
    const [result] = await runQuery(engine, "SELECT repeat('x', 5000) AS s", { maxCellBytes: 32 });
    expect(result!.rows![0]![0]).toBe("<VARCHAR 5000 bytes>");
  });

  it("times out without killing the engine", async () => {
    const [result] = await runQuery(engine, "SELECT count(*) FROM range(100000000000)", { timeoutMs: 500 });
    expect(result!.error).toMatch(/timed out/i);
    const [after] = await runQuery(engine, "SELECT 1 AS a");
    expect(after!.rows).toEqual([[1]]);
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd ai/mcp && pnpm vitest run test/query.test.ts`
Expected: FAIL — cannot resolve `../src/tools/query.js`.

- [ ] **Step 3: Implement `src/tools/query.ts`**

```ts
// Running a caller's SQL, one statement at a time, and shaping what comes back.

import type { Engine } from "../duckdb.js";
import { elideCell, splitStatements } from "../sql.js";

export interface QueryOptions {
  readonly maxRows: number;
  readonly maxCellBytes: number;
  readonly timeoutMs: number;
}

export const QUERY_DEFAULTS: QueryOptions = {
  maxRows: 100,
  maxCellBytes: 256,
  timeoutMs: 120_000,
};

export interface StatementResult {
  readonly statement: string;
  readonly columns?: { name: string; type: string }[];
  readonly rows?: unknown[][];
  /** Rows returned, not rows matched — see `truncated`. */
  readonly rowCount?: number;
  readonly truncated?: boolean;
  readonly elapsedMs?: number;
  readonly error?: string;
}

export async function runQuery(
  engine: Engine,
  sql: string,
  options: Partial<QueryOptions> = {},
): Promise<StatementResult[]> {
  const { maxRows, maxCellBytes, timeoutMs } = { ...QUERY_DEFAULTS, ...options };
  const results: StatementResult[] = [];

  for (const statement of splitStatements(sql)) {
    const started = Date.now();
    try {
      // `interrupt` is what makes the deadline recoverable: the statement is
      // cancelled inside the engine rather than abandoned, so the connection is
      // still usable afterwards.
      const timer = setTimeout(() => engine.connection.interrupt(), timeoutMs);
      let reader;
      try {
        // One row past the cap, never `runAndReadAll`. Reading everything and
        // slicing afterwards would materialise the whole result in Node's heap
        // first — and DuckDB's memory_limit does not govern the JS side, so
        // `SELECT * FROM read_cityjsonseq(<big>)` would exhaust the process
        // despite a 100-row cap. Reading cap+1 is what makes the cap real.
        reader = await engine.connection.runAndReadUntil(statement, maxRows + 1);
      } finally {
        clearTimeout(timer);
      }

      const names = reader.columnNames();
      const types = reader.columnTypes().map((t) => String(t));
      const fetched = reader.getRowsJson();
      const truncated = fetched.length > maxRows;
      const rows = fetched.slice(0, maxRows).map((row) =>
        row.map((value, index) => elideCell(value, types[index] ?? "VARCHAR", maxCellBytes)),
      );

      results.push({
        statement,
        columns: names.map((name, index) => ({ name, type: types[index] ?? "VARCHAR" })),
        rows,
        // Rows returned, not rows matched. The exact total is unknowable
        // without reading the whole result, which is precisely what this
        // avoids — `truncated` says there are more. Do not "fix" this back to
        // a total; a caller who needs one should SELECT count(*).
        rowCount: rows.length,
        truncated,
        elapsedMs: Date.now() - started,
      });
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      const elapsed = Date.now() - started;
      results.push({
        statement,
        error:
          elapsed >= timeoutMs
            ? `timed out after ${timeoutMs} ms: ${message}`
            : message,
        elapsedMs: elapsed,
      });
      break; // a script's later statements almost always depend on its earlier ones
    }
  }

  return results;
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd ai/mcp && pnpm vitest run test/query.test.ts`
Expected: PASS, 7 tests.

- [ ] **Step 5: Commit**

```bash
git add ai/mcp/src/tools/query.ts ai/mcp/test/query.test.ts
git commit -m "feat(mcp): run caller SQL statement by statement with elision and a deadline"
```

---

### Task 7: The describe tool

**Files:**
- Create: `ai/mcp/src/tools/describe.ts`
- Test: `ai/mcp/test/describe.test.ts`

**Interfaces:**
- Consumes: `Engine` from `src/duckdb.ts`.
- Produces:
  ```ts
  export const MODULE_TABLES: readonly string[]
  export const SIDECAR_TABLES: readonly string[]
  export interface TableSummary { name: string; file: string; rowCount: number | null; geometryColumns: string[]; lods: string[] }
  export interface DescribeResult { url: string; kind: "file" | "package"; inventory: "stac" | "probe"; crs: string | null; stac: Record<string, unknown> | null; tables: TableSummary[]; notes: string[] }
  export function geometryColumnsOf(columns: readonly string[]): string[]
  export function lodsOf(geometryColumns: readonly string[]): string[]
  export function describe(engine: Engine, url: string): Promise<DescribeResult>
  ```

**Built on footer and STAC reads, never on `PRAGMA cityparquet_read`** — that takes a directory path and has no documented remote form, so a describe built on it would work on stdio and fail on the deployment that needs it most.

- [ ] **Step 1: Write the failing test**

`ai/mcp/test/describe.test.ts`:

```ts
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
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd ai/mcp && pnpm vitest run test/describe.test.ts`
Expected: FAIL — cannot resolve `../src/tools/describe.js`.

- [ ] **Step 3: Implement `src/tools/describe.ts`**

```ts
// Answering "what is in this dataset" in one call.

import type { Engine } from "../duckdb.js";

/** Normative, from the specification's dataset-package chapter. */
export const MODULE_TABLES = [
  "building", "bridge", "tunnel", "construction", "transportation",
  "vegetation", "relief", "water_body", "land_use", "city_furniture", "generics",
] as const;

export const SIDECAR_TABLES = ["materials", "textures", "geometry_templates"] as const;

export interface TableSummary {
  readonly name: string;
  readonly file: string;
  readonly rowCount: number | null;
  readonly geometryColumns: string[];
  readonly lods: string[];
}

export interface DescribeResult {
  readonly url: string;
  readonly kind: "file" | "package";
  /** Where the file list came from — the STAC Item, or a probe of the normative basenames. */
  readonly inventory: "stac" | "probe";
  readonly crs: string | null;
  readonly stac: Record<string, unknown> | null;
  readonly tables: TableSummary[];
  readonly notes: string[];
}

export function geometryColumnsOf(columns: readonly string[]): string[] {
  return columns.filter((c) => /^geometry_lod\d+_\d+$/.test(c));
}

export function lodsOf(geometryColumns: readonly string[]): string[] {
  const lods = geometryColumns
    .map((c) => /^geometry_lod(\d+)_(\d+)$/.exec(c))
    .filter((m): m is RegExpExecArray => m !== null)
    .map((m) => `${m[1]}.${m[2]}`);
  return [...new Set(lods)].sort();
}

function sqlLiteral(value: string): string {
  return `'${value.replace(/'/g, "''")}'`;
}

async function summariseFile(engine: Engine, url: string, name: string): Promise<TableSummary | null> {
  try {
    const schema = await engine.connection.runAndReadAll(
      `SELECT name FROM parquet_schema(${sqlLiteral(url)})`,
    );
    const columns = schema.getRowsJson().map((row) => String(row[0]));
    const geometryColumns = geometryColumnsOf(columns);

    let rowCount: number | null = null;
    try {
      const meta = await engine.connection.runAndReadAll(
        `SELECT sum(num_rows)::BIGINT FROM parquet_file_metadata(${sqlLiteral(url)})`,
      );
      const value = meta.getRowsJson()[0]?.[0];
      rowCount = value === null || value === undefined ? null : Number(value);
    } catch {
      rowCount = null;
    }

    return { name, file: url, rowCount, geometryColumns, lods: lodsOf(geometryColumns) };
  } catch {
    return null;
  }
}

/** The `city` footer is authoritative for decoding; the STAC Item is a mirror. */
async function footerCrs(engine: Engine, url: string): Promise<string | null> {
  try {
    const reader = await engine.connection.runAndReadAll(
      `SELECT cityparquet_city_field(city, 'referenceSystem')
       FROM (SELECT cityjson_geoparquet_geo(${sqlLiteral(url)}).city AS city)`,
    );
    const value = reader.getRowsJson()[0]?.[0];
    return value === null || value === undefined ? null : String(value);
  } catch {
    return null;
  }
}

export async function describe(engine: Engine, url: string): Promise<DescribeResult> {
  const notes: string[] = [];
  const trimmed = url.replace(/\/+$/, "");

  if (/\.parquet$/i.test(trimmed)) {
    const table = await summariseFile(engine, trimmed, trimmed.split("/").pop() ?? trimmed);
    if (!table) throw new Error(`could not read a Parquet footer at ${trimmed}`);
    return {
      url: trimmed,
      kind: "file",
      inventory: "probe",
      crs: await footerCrs(engine, trimmed),
      stac: null,
      tables: [table],
      notes,
    };
  }

  // A package. The STAC Item's assets map is the file inventory — but the
  // specification makes that a SHOULD, and the Item may be absent entirely, so
  // the normative basenames are the fallback.
  let stac: Record<string, unknown> | null = null;
  let files: { name: string; url: string }[] = [];
  let inventory: "stac" | "probe" = "probe";

  try {
    const response = await fetch(`${trimmed}/metadata.json`);
    if (response.ok) {
      stac = (await response.json()) as Record<string, unknown>;
      const assets = stac.assets as Record<string, { href?: string }> | undefined;
      if (assets) {
        files = Object.entries(assets)
          .filter(([, asset]) => asset.href?.endsWith(".parquet"))
          .map(([name, asset]) => ({
            name,
            url: new URL(asset.href!, `${trimmed}/`).toString(),
          }));
        if (files.length > 0) inventory = "stac";
      }
      if (files.length === 0) {
        notes.push("metadata.json carries no Parquet assets; probing the normative basenames instead.");
      }
    } else {
      notes.push(`no metadata.json (HTTP ${response.status}); probing the normative basenames instead.`);
    }
  } catch (error) {
    notes.push(
      `metadata.json unreachable (${error instanceof Error ? error.message : String(error)}); probing the normative basenames instead.`,
    );
  }

  if (files.length === 0) {
    files = [...MODULE_TABLES, ...SIDECAR_TABLES].map((name) => ({
      name,
      url: `${trimmed}/${name}.parquet`,
    }));
  }

  const summaries = await Promise.all(files.map((f) => summariseFile(engine, f.url, f.name)));
  const tables = summaries.filter((t): t is TableSummary => t !== null);
  if (tables.length === 0) throw new Error(`no readable Parquet files under ${trimmed}`);

  const crs = await footerCrs(engine, tables[0]!.file);
  if (crs === null) notes.push("no CRS in the footer — the package states nothing about its coordinate system.");

  return { url: trimmed, kind: "package", inventory, crs, stac, tables, notes };
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd ai/mcp && pnpm vitest run test/describe.test.ts`
Expected: PASS, 9 tests — the four inventory tests cover both paths the specification allows, with no network.

- [ ] **Step 5: Verify against a real package by hand**

Run:

```bash
cd ai/mcp && node --experimental-strip-types -e "
import { createEngine } from './src/duckdb.ts';
import { describe } from './src/tools/describe.ts';
const engine = await createEngine({ sandbox: true, extensionDirectory: '/tmp/cp-ext' });
console.log(JSON.stringify(await describe(engine, 'https://cityparquet.open3d.city/data/delft'), null, 2));
await engine.close();
"
```

Expected: a package description naming its tables. If the URL 404s, find a live one in `documents/playground/presets.ts` and use that. Record whatever it returns in the commit message — this is the first end-to-end evidence the tool works.

- [ ] **Step 6: Commit**

```bash
git add ai/mcp/src/tools/describe.ts ai/mcp/test/describe.test.ts
git commit -m "feat(mcp): describe a CityParquet package from its STAC Item and footers"
```

---

### Task 8: Server wiring and the stdio entry point

**Files:**
- Create: `ai/mcp/src/server.ts`, `ai/mcp/src/stdio.ts`
- Test: `ai/mcp/test/server.test.ts`

**Interfaces:**
- Consumes: everything above.
- Produces:
  ```ts
  export interface ServerDeps { corpus: Corpus; engine: Engine }
  export function createServer(deps: ServerDeps): McpServer
  ```

Registration only — no logic. The five tools are `cityparquet_docs_outline`, `cityparquet_docs_search`, `cityparquet_docs_read`, `cityparquet_describe`, `cityparquet_query`.

**The `cityparquet_` prefix is a phase-1 choice, not a commitment** (spec §10): the merged CityJSON/CityParquet server will want one neutral prefix.

- [ ] **Step 1: Write the failing test**

`ai/mcp/test/server.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { createServer } from "../src/server.js";
import type { Corpus } from "../src/corpus.js";
import type { Engine } from "../src/duckdb.js";

const CORPUS: Corpus = {
  generatedFrom: "test",
  corpora: {
    spec: {
      title: "CityParquet specification",
      source: "x",
      chapters: [{ id: "metadata", title: "Metadata", description: "d", order: 0, sections: [], body: "The footer is authoritative." }],
    },
    "duckdb-cityjson": { title: "y", source: "y", chapters: [] },
    "duckdb-3d": { title: "z", source: "z", chapters: [] },
  },
};

const ENGINE = { connection: {}, extensions: [], async close() {} } as unknown as Engine;

describe("createServer", () => {
  it("registers exactly the five tools", () => {
    const server = createServer({ corpus: CORPUS, engine: ENGINE });
    // McpServer keeps its registrations on a private map; the public surface is
    // the tool list, so assert through it.
    const names = Object.keys((server as unknown as { _registeredTools: Record<string, unknown> })._registeredTools);
    expect(names.sort()).toEqual([
      "cityparquet_describe",
      "cityparquet_docs_outline",
      "cityparquet_docs_read",
      "cityparquet_docs_search",
      "cityparquet_query",
    ]);
  });
});
```

If `_registeredTools` is not the private field name in `@modelcontextprotocol/server@2.0.0`, connect an `InMemoryTransport` client instead and assert on `listTools()`. Do not weaken the assertion to "at least five".

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd ai/mcp && pnpm vitest run test/server.test.ts`
Expected: FAIL — cannot resolve `../src/server.js`.

- [ ] **Step 3: Implement `src/server.ts`**

```ts
// Registration, and nothing else. Every tool here is a thin call into a module
// that is tested without a transport.

import { McpServer } from "@modelcontextprotocol/server";
import * as z from "zod/v4";

import { CORPUS_IDS, outline, readChapter, search, type Corpus, type CorpusId } from "./corpus.js";
import type { Engine } from "./duckdb.js";
import { describe } from "./tools/describe.js";
import { QUERY_DEFAULTS, runQuery } from "./tools/query.js";

export interface ServerDeps {
  readonly corpus: Corpus;
  readonly engine: Engine;
}

const corpusEnum = z.enum(CORPUS_IDS as unknown as [CorpusId, ...CorpusId[]]);

const json = (value: unknown) => ({ content: [{ type: "text" as const, text: JSON.stringify(value, null, 2) }] });
const text = (value: string) => ({ content: [{ type: "text" as const, text: value }] });
const failure = (error: unknown) => ({
  content: [{ type: "text" as const, text: error instanceof Error ? error.message : String(error) }],
  isError: true,
});

export function createServer({ corpus, engine }: ServerDeps): McpServer {
  const server = new McpServer({ name: "cityparquet", version: "0.1.0" });

  server.registerTool(
    "cityparquet_docs_outline",
    {
      description:
        "List the chapters of the CityParquet documentation. Corpora: 'spec' (the normative specification and its design decisions), 'duckdb-cityjson' and 'duckdb-3d' (the DuckDB extension function references). Call this first to learn what can be read; omit `corpus` to see all three.",
      inputSchema: z.object({ corpus: corpusEnum.optional().describe("Restrict to one corpus") }),
    },
    async ({ corpus: id }) => json(outline(corpus, id)),
  );

  server.registerTool(
    "cityparquet_docs_search",
    {
      description:
        "Search the CityParquet documentation for a term and return matching sections with snippets. Faster than reading chapters when you are looking for a specific column, function or rule.",
      inputSchema: z.object({
        query: z.string().describe("Term to look for, e.g. 'semantic surfaces' or 'ST_3DVolume'"),
        corpus: corpusEnum.optional(),
        limit: z.number().int().min(1).max(50).optional(),
      }),
    },
    async ({ query, corpus: id, limit }) => json(search(corpus, query, { corpus: id, limit })),
  );

  server.registerTool(
    "cityparquet_docs_read",
    {
      description:
        "Read one chapter of the CityParquet documentation, or one section of it. Get chapter ids from cityparquet_docs_outline or cityparquet_docs_search.",
      inputSchema: z.object({
        corpus: corpusEnum,
        chapter: z.string().describe("Chapter id, e.g. 'object-table-schema'"),
        section: z.string().optional().describe("Read only this section of the chapter"),
      }),
    },
    async ({ corpus: id, chapter, section }) => {
      try {
        return text(readChapter(corpus, id, chapter, section));
      } catch (error) {
        return failure(error);
      }
    },
  );

  server.registerTool(
    "cityparquet_describe",
    {
      description:
        "Describe a CityParquet dataset: its module tables, row counts, LoDs, geometry columns and CRS. Accepts a package directory URL or a single .parquet URL. Call this before querying an unfamiliar dataset.",
      inputSchema: z.object({ url: z.string().describe("Package directory URL or .parquet URL") }),
    },
    async ({ url }) => {
      try {
        return json(await describe(engine, url));
      } catch (error) {
        return failure(error);
      }
    },
  );

  server.registerTool(
    "cityparquet_query",
    {
      description:
        `Run SQL against DuckDB with the cityjson and three_d extensions loaded (note: the spatial extension is NOT available, so use ST_3DFootprintArea rather than ST_Area). A script is split and its statements run one at a time. BLOB columns and oversized values are elided — select the columns you need rather than *. Results are capped at ${QUERY_DEFAULTS.maxRows} rows by default.`,
      inputSchema: z.object({
        sql: z.string().describe("One or more SQL statements, separated by semicolons"),
        max_rows: z.number().int().min(1).max(5000).optional(),
        max_cell_bytes: z.number().int().min(16).max(65536).optional(),
        timeout_ms: z.number().int().min(1000).max(600_000).optional(),
      }),
    },
    async ({ sql, max_rows, max_cell_bytes, timeout_ms }) =>
      json(
        await runQuery(engine, sql, {
          maxRows: max_rows ?? QUERY_DEFAULTS.maxRows,
          maxCellBytes: max_cell_bytes ?? QUERY_DEFAULTS.maxCellBytes,
          timeoutMs: timeout_ms ?? QUERY_DEFAULTS.timeoutMs,
        }),
      ),
  );

  // The chapters again, as resources. Client support is uneven, so nothing may
  // depend on these — the tools are the contract.
  for (const id of CORPUS_IDS) {
    for (const chapter of corpus.corpora[id].chapters) {
      server.registerResource(
        `${id}/${chapter.id}`,
        `cityparquet://${id}/${chapter.id}`,
        { description: chapter.description || chapter.title, mimeType: "text/markdown" },
        async (uri) => ({ contents: [{ uri: uri.href, text: chapter.body }] }),
      );
    }
  }

  return server;
}
```

- [ ] **Step 4: Implement `src/stdio.ts`**

```ts
#!/usr/bin/env node
// The local entry point. The user's own machine is the trust boundary, so the
// sandbox is off unless CITYPARQUET_MCP_SANDBOX asks for it.

import { homedir } from "node:os";
import { join } from "node:path";

import { serveStdio } from "@modelcontextprotocol/server/stdio";

import { loadCorpus } from "./corpus.js";
import { createEngine, DEFAULT_EXTENSIONS } from "./duckdb.js";
import { createServer } from "./server.js";

const extensionDirectory =
  process.env.CITYPARQUET_MCP_EXTENSION_DIR ?? join(homedir(), ".cityparquet-mcp", "extensions");

const extensions = process.env.CITYPARQUET_MCP_EXTENSIONS?.split(",").map((s) => s.trim());

const corpus = loadCorpus();
const engine = await createEngine({
  sandbox: process.env.CITYPARQUET_MCP_SANDBOX === "1",
  extensionDirectory,
  extensions: extensions ?? DEFAULT_EXTENSIONS,
  memoryLimit: process.env.CITYPARQUET_MCP_MEMORY_LIMIT,
});

process.on("SIGINT", () => void engine.close().finally(() => process.exit(0)));
process.on("SIGTERM", () => void engine.close().finally(() => process.exit(0)));

serveStdio(() => createServer({ corpus, engine }));
```

- [ ] **Step 5: Run the test and the build**

Run: `cd ai/mcp && pnpm vitest run test/server.test.ts && pnpm typecheck && pnpm build`
Expected: PASS, then a clean `tsc` and a populated `dist/`.

- [ ] **Step 6: Smoke-test the server over stdio**

Run:

```bash
cd ai/mcp && printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"smoke","version":"0"}}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
  | node dist/stdio.js 2>/dev/null | tail -1 | head -c 600
```

Expected: a `tools/list` response naming all five tools.

- [ ] **Step 7: Commit**

```bash
git add ai/mcp/src/server.ts ai/mcp/src/stdio.ts ai/mcp/test/server.test.ts
git commit -m "feat(mcp): register the five tools and serve them over stdio"
```

---

### Task 9: Package documentation and the repository gates

**Files:**
- Create: `ai/mcp/CLAUDE.md`, `ai/mcp/AGENTS.md`, `ai/mcp/README.md`
- Modify: `justfile` (root) — add `mcp-check`, extend `check`
- Modify: `CLAUDE.md`, `AGENTS.md` (root) — layout table
- Create: `.github/workflows/mcp.yml`

- [ ] **Step 1: Write `ai/mcp/README.md`**

Cover: what the server is; the five tools in a table; `pnpm install && pnpm corpus && pnpm build`; the stdio client configuration below; every environment variable (`CITYPARQUET_MCP_SANDBOX`, `CITYPARQUET_MCP_EXTENSION_DIR`, `CITYPARQUET_MCP_EXTENSIONS`, `CITYPARQUET_MCP_MEMORY_LIMIT`); and **why `spatial` is unavailable**, with the substitution table:

| Instead of | Use |
| --- | --- |
| `ST_Area(ST_GeomFromWKB(geometry_lod0_0))` | `ST_3DFootprintArea(ST_3DTryFromWKB(geometry_lod0_0))` |
| `ST_GeomFromWKB(...)` | `ST_3DTryFromWKB(wkb, geometry_properties)` |

Client configuration:

```json
{
  "mcpServers": {
    "cityparquet": { "command": "node", "args": ["/absolute/path/to/ai/mcp/dist/stdio.js"] }
  }
}
```

- [ ] **Step 2: Write `ai/mcp/CLAUDE.md`, and copy it to `AGENTS.md`**

It must state: the exact pins and why (spec §6.1); that `spatial` is never loaded and why (spec §6.4); that the sandbox ordering in `duckdb.ts` is load-bearing; that `corpus.json` is generated and never hand-edited; that the negative tests in `test/duckdb.test.ts` are a security contract; and that the `cityparquet_` tool prefix is provisional.

```bash
cp ai/mcp/CLAUDE.md ai/mcp/AGENTS.md
diff ai/mcp/CLAUDE.md ai/mcp/AGENTS.md && echo "byte-identical"
```

- [ ] **Step 3: Add the root justfile recipes**

```just
# The MCP server's gate: typecheck, tests, and a freshness check on the corpus.
mcp-check:
    cd ai/mcp && pnpm install --frozen-lockfile && pnpm typecheck && pnpm test
    just mcp-corpus
    git diff --quiet -- ai/mcp/corpus/corpus.json || \
      (echo "corpus.json is stale — commit the regenerated file" && exit 1)
```

Add `mcp-check` to the root `check` recipe's body, alongside the existing gates.

- [ ] **Step 4: Add the root layout table rows**

In the root `CLAUDE.md` layout table, after the `ai/design-notes/` row:

```markdown
| `ai/mcp/`              | The **MCP server** — the specification, the function references, dataset description and sandboxed SQL, for agents | its `CLAUDE.md`                             |
```

Then `cp CLAUDE.md AGENTS.md` and confirm they are byte-identical.

- [ ] **Step 5: Add CI**

`.github/workflows/mcp.yml`:

```yaml
name: mcp

on:
  push:
    paths: ["ai/mcp/**", "documents/docs/**", ".github/workflows/mcp.yml"]
  pull_request:
    paths: ["ai/mcp/**", "documents/docs/**", ".github/workflows/mcp.yml"]

jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        # One level, not recursive: the corpus needs each extension's own
        # docs/FUNCTIONS.md, and recursing would pull their vendored duckdb/
        # and extension-ci-tools/ checkouts — the ~1.2 GB `just setup` warns about.
        with: { submodules: true }
      - uses: pnpm/action-setup@v4
      - uses: actions/setup-node@v4
        with: { node-version: "24", cache: "pnpm", cache-dependency-path: ai/mcp/pnpm-lock.yaml }
      - run: pnpm install --frozen-lockfile
        working-directory: ai/mcp
      - run: pnpm typecheck
        working-directory: ai/mcp
      - run: pnpm test
        working-directory: ai/mcp
      - name: corpus is fresh
        run: |
          pnpm corpus
          cd ../.. && git diff --exit-code -- ai/mcp/corpus/corpus.json
        working-directory: ai/mcp
```

- [ ] **Step 6: Run the full gate**

Run: `just mcp-check`
Expected: typecheck clean, all tests pass, corpus unchanged.

- [ ] **Step 7: Commit**

```bash
git add ai/mcp/README.md ai/mcp/CLAUDE.md ai/mcp/AGENTS.md justfile CLAUDE.md AGENTS.md .github/workflows/mcp.yml
git commit -m "docs(mcp): document the server and gate it in CI"
```

---

## Follow-on work, not in this plan

- **Phase 2** — the four skills and the plugin/APM packaging (spec §9).
- **Phase 3** — HTTP transport, the container, Cloudflare, CI/CD (spec §8).
- **Report the `spatial` / `three_d` collision** in `lib/duckdb-3d`'s own repository. It is a defect in a sibling library and a false claim in its README; it must not be worked around here beyond choosing which extension to load.
- **Verify at implementation** (spec §12): whether `allowed_paths` can readmit a temp directory for spilling; the cost of per-request `ATTACH`/`DETACH`, which phase 3 needs and phase 1 does not.
