#!/usr/bin/env node
// Installs the extensions into CITYPARQUET_MCP_EXTENSION_DIR at image build
// time. The deployed container has no route to the extension repository —
// its egress is an allowlist of data hosts — so an extension that is not in
// the image cannot be installed later, and the server would fail to start.

import { createEngine, extensionsFromEnv } from "./duckdb.js";

const extensionDirectory = process.env.CITYPARQUET_MCP_EXTENSION_DIR;
if (!extensionDirectory) throw new Error("CITYPARQUET_MCP_EXTENSION_DIR must be set");

const engine = await createEngine({
  sandbox: false,
  extensionDirectory,
  extensions: extensionsFromEnv(process.env.CITYPARQUET_MCP_EXTENSIONS),
});
console.log(JSON.stringify({ baked: engine.extensions, into: extensionDirectory }));
await engine.close();
