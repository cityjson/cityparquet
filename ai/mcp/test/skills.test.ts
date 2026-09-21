import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { mkdtempSync, readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { createEngine, type Engine } from "../src/duckdb.js";
import { runQuery } from "../src/tools/query.js";

// The skills under ai/plugin/skills teach SQL, and SQL that does not run is
// worse than none: an agent trusts it. Every ```sql block in every skill is
// run here, through the same runQuery the MCP tool uses, against the live
// Delft package. Paths under '/data/' in the skills are placeholders for a
// user's own directory; they are pointed at a scratch directory holding a
// byte-for-byte copy of that package.

const SKILLS = ["cityparquet", "cityparquet-query", "cityparquet-write", "cityparquet-3d-analysis"];
const DELFT = "https://cityparquet.open3d.city/data/delft";

function sqlBlocks(skill: string): string[] {
  const md = readFileSync(new URL(`../../plugin/skills/${skill}/SKILL.md`, import.meta.url), "utf8");
  return [...md.matchAll(/```sql\n([\s\S]*?)```/g)].map((m) => m[1]!);
}

describe("the SQL in the skills", () => {
  const extensionDirectory = join(mkdtempSync(join(tmpdir(), "cityparquet-mcp-")), "extensions");
  const data = mkdtempSync(join(tmpdir(), "cityparquet-mcp-skills-"));

  beforeAll(async () => {
    mkdirSync(join(data, "delft"));
    for (const file of ["metadata.json", "building.parquet"]) {
      const response = await fetch(`${DELFT}/${file}`);
      if (!response.ok) throw new Error(`fetching ${file}: HTTP ${response.status}`);
      writeFileSync(join(data, "delft", file), Buffer.from(await response.arrayBuffer()));
    }
  });

  for (const skill of SKILLS) {
    const blocks = sqlBlocks(skill);
    if (blocks.length === 0) continue; // the router skill teaches no SQL of its own
    describe(skill, () => {
      let engine: Engine;
      beforeAll(async () => {
        engine = await createEngine({ sandbox: false, extensionDirectory });
      });
      afterAll(async () => { await engine?.close(); });

      // In order, on one engine: a block may use the schema or the files an
      // earlier one made.
      blocks.forEach((block, index) => {
        it(`runs block ${index + 1}`, async () => {
          const sql = block.replaceAll("'/data/", `'${data}/`);
          const results = await runQuery(engine, sql, { maxRows: 5 });
          for (const result of results) expect(result.error, result.statement).toBeUndefined();
        });
      });
    });
  }
});
