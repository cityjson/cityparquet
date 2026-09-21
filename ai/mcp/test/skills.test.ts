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

/** The router teaches no SQL of its own; every other skill must. */
const ROUTER = "cityparquet";
const SKILLS = [ROUTER, "cityparquet-query", "cityparquet-write", "cityparquet-3d-analysis"];
const DELFT = "https://cityparquet.open3d.city/data/delft";

function sqlBlocks(skill: string): string[] {
  const md = readFileSync(new URL(`../../plugin/skills/${skill}/SKILL.md`, import.meta.url), "utf8");
  return [...md.replace(/\r\n/g, "\n").matchAll(/^```sql[^\S\n]*\n([\s\S]*?)^```/gim)].map((m) => m[1]!);
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

  // Running is not enough: the silent failures these skills exist to prevent
  // — a CRS dropped on the way out, a validity gate that excludes every
  // solid — all run without error. These are checked after a skill's blocks
  // have run, on the same engine, against what the blocks should have made.
  const outcomes: Record<string, { sql: string; rows: unknown[][] }[]> = {
    "cityparquet-write": [
      { sql: "SELECT reference_system.code FROM cityjsonseq_metadata('/data/delft.city.jsonl')", rows: [["7415"]] },
      { sql: "SELECT json_extract_string(city, '$.crs.id.code') FROM readback.__cityparquet", rows: [["7415"]] },
      { sql: "SELECT DISTINCT object_type FROM delft.building", rows: [["Building"]] },
      { sql: "SELECT count(*)::INTEGER FROM readback.building", rows: [[2231]] },
    ],
    "cityparquet-3d-analysis": [
      {
        sql: `SELECT count(*)::INTEGER FROM (
                SELECT ST_3DTryFromWKB(geometry_lod2_2, geometry_properties_lod2_2) AS solid
                FROM read_parquet('${DELFT}/building.parquet') WHERE geometry_lod2_2 IS NOT NULL)
              WHERE solid IS NOT NULL AND ST_3DValidationReport(solid).is_valid`,
        rows: [[1098]],
      },
    ],
  };

  it("finds SQL in every skill but the router", () => {
    for (const skill of SKILLS) {
      if (skill === ROUTER) expect(sqlBlocks(skill), skill).toEqual([]);
      else expect(sqlBlocks(skill).length, skill).toBeGreaterThan(0);
    }
  });

  for (const skill of SKILLS.filter((s) => s !== ROUTER)) {
    const blocks = sqlBlocks(skill);
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

      for (const { sql, rows } of outcomes[skill] ?? []) {
        it(`leaves the expected result: ${sql.replace(/\s+/g, " ").slice(0, 60)}`, async () => {
          const [result] = await runQuery(engine, sql.replaceAll("'/data/", `'${data}/`));
          expect(result!.error).toBeUndefined();
          expect(result!.rows).toEqual(rows);
        });
      }
    });
  }
});
