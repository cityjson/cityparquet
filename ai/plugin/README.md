# CityParquet skills

Four agent skills for working with CityParquet in DuckDB. This directory is
both a Claude Code plugin and an APM package, and both read the same
`skills/`:

| Skill | For |
| --- | --- |
| `cityparquet` | What the format is, where to start, and questions about the specification |
| `cityparquet-query` | Reading, filtering and aggregating a package or a CityJSON source |
| `cityparquet-write` | Building, editing and writing a package; converting to CityJSON, CityJSONSeq and FlatCityBuf |
| `cityparquet-3d-analysis` | Volume, footprint, height, validity and reprojection with `three_d` |

The skills are plain Markdown. They name the tools of the MCP server in
[`../mcp`](../mcp), and each one says what to do without it: the same SQL in
`duckdb` v1.5.5 with the community extensions.

## Install

Claude Code:

```text
/plugin marketplace add cityjson/cityparquet
/plugin install cityparquet@cityparquet
```

APM (Claude Code and Codex):

```sh
apm install cityjson/cityparquet/ai/plugin
```

The plugin does **not** register the MCP server. Claude Code copies a plugin
into its own cache, so it cannot reach `../mcp`, and the server is not
published as a package yet. Build the server from a clone and register it
yourself; [`../mcp/README.md`](../mcp/README.md) has the steps.

## Keeping them honest

Every SQL block in these skills was run against DuckDB v1.5.5 with the
published `cityjson` and `three_d` builds, on the Delft package at
`https://cityparquet.open3d.city/data/delft`. When the pin in
`../mcp/package.json` moves, run them again.
