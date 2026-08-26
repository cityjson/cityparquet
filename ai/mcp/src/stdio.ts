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
