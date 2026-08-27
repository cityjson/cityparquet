// Bringing DuckDB up with the CityParquet extensions, and shutting the doors
// behind it.

import { DuckDBConnection, DuckDBInstance } from "@duckdb/node-api";

import { serialiser } from "./serialise.js";

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
  /**
   * Have the connection to yourself for the duration of `task`. All five
   * tools share this one `DuckDBConnection`; an MCP client pipelines tool
   * calls, and two statements in flight on it at once interleave inside the
   * engine rather than queueing. See `serialise.ts` for the mechanism and
   * why a `query` timeout is the case that made this matter.
   */
  exclusive<T>(task: () => Promise<T>): Promise<T>;
  close(): Promise<void>;
}

export async function createEngine(options: EngineOptions): Promise<Engine> {
  const wanted = options.extensions ?? DEFAULT_EXTENSIONS;

  // 1. The instance.
  const instance = await DuckDBInstance.create(":memory:", {
    extension_directory: options.extensionDirectory,
  });
  const connection = await instance.connect();

  // 2. `allow_persistent_secrets` must be set before any extension that
  //    touches the secret manager loads — `cityjson` uses it on LOAD (it can
  //    read URLs itself), and DuckDB refuses to change secret-manager
  //    settings once the secret manager has been used. Set this one first;
  //    it is not filesystem-dependent, so it does not conflict with the
  //    install-before-sandbox ordering below. It stays enforced even once
  //    locked: DuckDB will not let a later query flip it back, sandbox or
  //    not.
  if (options.sandbox) {
    await connection.run("SET allow_persistent_secrets = false");
  }

  // 3. Extensions — this touches the local filesystem, so it must finish
  //    before step 7 disables it.
  for (const name of wanted) {
    const from = COMMUNITY_EXTENSIONS.has(name) ? " FROM community" : "";
    await connection.run(`INSTALL ${name}${from}`);
    await connection.run(`LOAD ${name}`);
  }

  // 4. `duckdb_extensions()` itself reads the local filesystem (it inspects
  //    the extension directory on disk), so this must run before the sandbox
  //    disables LocalFileSystem below — not after, as a query issued once the
  //    engine is locked down.
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

  // 5. Resource limits.
  if (options.memoryLimit) await connection.run(`SET memory_limit = '${options.memoryLimit}'`);
  if (options.threads !== undefined) await connection.run(`SET threads = ${options.threads}`);

  if (options.sandbox) {
    // 6. Close the doors.
    for (const setting of [
      "autoinstall_known_extensions",
      "autoload_known_extensions",
      "allow_community_extensions",
    ]) {
      await connection.run(`SET ${setting} = false`);
    }

    // 7. No local filesystem. A query that would spill to a temporary file now
    //    fails rather than spilling — the right trade for a public endpoint,
    //    and the reason memory_limit should be generous.
    await connection.run("SET disabled_filesystems = 'LocalFileSystem'");

    // 8. And none of the above can be undone by a query.
    await connection.run("SET lock_configuration = true");
  }

  // 9. A liveness check on the connection itself, nothing more: confirm it
  //    still runs a trivial statement after every setup step above —
  //    including, in sandbox mode, having just locked the configuration — so
  //    a startup misconfiguration surfaces here rather than on the first
  //    real request. `SELECT 1` touches no extension and no network; it does
  //    not warm httpfs or anything else.
  await connection.run("SELECT 1");

  const exclusive = serialiser();

  return {
    connection,
    extensions,
    exclusive,
    async close() {
      connection.closeSync();
    },
  };
}
