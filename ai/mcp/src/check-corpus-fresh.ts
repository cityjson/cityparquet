// A freshness gate on corpus/corpus.json that never writes to disk.
//
// `corpus.json` carries `generatedFrom`, a `git describe --always --dirty`
// stamp that advances on every commit. Comparing the file byte-for-byte
// against a freshly regenerated one — as `just mcp-corpus` plus `git diff`
// would — is therefore never empty, on any commit, whether or not the
// content is actually stale. That gate is permanently red from its first
// run and cannot tell a genuinely stale corpus from the normal case.
//
// This compares `corpora` only, ignoring `generatedFrom`, and reports
// whether documents/docs/ or either extension's FUNCTIONS.md has changed
// since the committed file was generated. It builds the comparison in
// memory and writes nothing, so the working tree is left exactly as it was
// found, whether the check passes or fails.

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { buildCorpus } from "./build-corpus.js";
import type { Corpus } from "./corpus.js";

export function isCorpusFresh(committed: Corpus, current: Corpus): boolean {
  return JSON.stringify(committed.corpora) === JSON.stringify(current.corpora);
}

if (import.meta.url === `file://${process.argv[1]}`) {
  // Running from dist/: up to ai/mcp, then ai, then the repository root —
  // the same path build-corpus.ts's own CLI entry point uses.
  const here = dirname(fileURLToPath(import.meta.url));
  const repoRoot = join(here, "..", "..", "..");
  const corpusPath = join(here, "..", "corpus", "corpus.json");

  const committed = JSON.parse(readFileSync(corpusPath, "utf8")) as Corpus;
  // The stamp is irrelevant to freshness, so any placeholder will do.
  const current = buildCorpus(repoRoot, "freshness-check");

  if (isCorpusFresh(committed, current)) {
    process.stderr.write("corpus.json is fresh\n");
    process.exit(0);
  }

  process.stderr.write(
    "corpus.json is stale: documents/docs/ or a FUNCTIONS.md has changed since it was\n" +
      "last regenerated. Run `just mcp-corpus` and commit the result.\n",
  );
  process.exit(1);
}
