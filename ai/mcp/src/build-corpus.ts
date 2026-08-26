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
