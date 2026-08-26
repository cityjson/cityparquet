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
  const [, entry] = entries(corpus, corpusId)[0]!;
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
