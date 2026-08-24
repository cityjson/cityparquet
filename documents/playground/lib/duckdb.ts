// Bringing up DuckDB-Wasm in the browser, with the CityParquet extensions.

import * as duckdb from "@duckdb/duckdb-wasm";
import ehWorkerUrl from "@duckdb/duckdb-wasm/dist/duckdb-browser-eh.worker.js?url";
import ehModuleUrl from "@duckdb/duckdb-wasm/dist/duckdb-eh.wasm?url";

import { ALLOW_UNSIGNED, EXTENSIONS, EXTENSION_SOURCE, type ExtensionName } from "../config";
import { createCountingWorker } from "./bytes";

export interface LoadedExtension {
  readonly name: ExtensionName;
  readonly version: string | null;
  readonly error: string | null;
}

export interface Session {
  readonly db: duckdb.AsyncDuckDB;
  readonly connection: duckdb.AsyncDuckDBConnection;
  readonly worker: Worker;
  readonly extensions: readonly LoadedExtension[];
}

export type Progress = (message: string) => void;

/**
 * Only the `eh` bundle is offered.
 *
 * The `coi` bundle needs SharedArrayBuffer, which needs COOP/COEP response
 * headers, which GitHub Pages cannot send. And `mvp` is worse than merely
 * slower: it references `_setThrew` without defining it, so the first C++
 * exception arrives as `ReferenceError: _setThrew is not defined` instead of
 * the DuckDB message — which in a SQL console would make every error useless.
 *
 * Both URLs are made **fully qualified** here, and that is load-bearing.
 *
 * Vite's `?url` gives a root-absolute path (`/_astro/duckdb-eh.<hash>.wasm`),
 * which is fine on the page but not in the worker: the worker is created from a
 * `blob:` URL so that the byte counter can patch XHR first, and inside a blob
 * worker `self.location` is the blob URL itself. A root-absolute path has
 * nothing sensible to resolve against there, so the fetch never happens and the
 * instance stalls with no error — the worker script loads and then silence.
 * Resolving against `location.href` on the main thread, where the base is the
 * real page, removes the ambiguity entirely.
 */
function bundles(): duckdb.DuckDBBundles {
  const absolute = (url: string) => new URL(url, location.href).href;
  return {
    eh: { mainModule: absolute(ehModuleUrl), mainWorker: absolute(ehWorkerUrl) },
  };
}

/** Load one extension, from whichever source `config.ts` selected. */
async function loadExtension(
  connection: duckdb.AsyncDuckDBConnection,
  name: ExtensionName,
): Promise<LoadedExtension> {
  const install =
    EXTENSION_SOURCE.kind === "community" ? `INSTALL ${name} FROM community` : `INSTALL ${name}`;

  // One retry. Loading two community extensions back to back, the second has
  // been observed to fail on its first attempt while succeeding immediately
  // afterwards — seen with either extension in the second position, so it is
  // positional rather than a property of one of them.
  let lastError: unknown = null;
  for (let attempt = 0; attempt < 2; attempt++) {
    try {
      await connection.query(install);
      await connection.query(`LOAD ${name}`);
      const result = await connection.query(
        `SELECT extension_version FROM duckdb_extensions() WHERE extension_name = '${name}'`,
      );
      const row = result.toArray()[0] as { extension_version?: string } | undefined;
      return { name, version: row?.extension_version ?? null, error: null };
    } catch (error) {
      lastError = error;
    }
  }
  return { name, version: null, error: describe(lastError) };
}

/** Boot DuckDB, open an in-memory database, and load the extensions. */
export async function createSession(onProgress: Progress = () => {}): Promise<Session> {
  onProgress("Fetching the DuckDB engine…");
  const bundle = await duckdb.selectBundle(bundles());

  onProgress("Starting the database worker…");
  const worker = createCountingWorker(bundle.mainWorker!);
  const db = new duckdb.AsyncDuckDB(new duckdb.VoidLogger(), worker);

  // Instantiation is where a bad asset URL shows up, and its failure mode is
  // silence rather than an exception: the worker never fetches the module and
  // the promise never settles. Race it against the worker's own error event and
  // a deadline so the page reports something a reader can act on.
  await Promise.race([
    db.instantiate(bundle.mainModule, bundle.pthreadWorker),
    new Promise<never>((_resolve, reject) => {
      worker.addEventListener("error", (event) =>
        reject(new Error(`The database worker failed to start: ${event.message || event.type}`)),
      );
      setTimeout(
        () =>
          reject(
            new Error(
              "The database engine did not start within 60s. The WebAssembly module " +
                "may not have been reachable.",
            ),
          ),
        60_000,
      );
    }),
  ]);

  await db.open({
    path: ":memory:",
    // Locally built extensions are not signed by DuckDB Labs. The published
    // ones are, so this stays off in production.
    allowUnsignedExtensions: ALLOW_UNSIGNED,
    query: { castBigIntToDouble: true },
  });

  const connection = await db.connect();

  if (EXTENSION_SOURCE.kind === "repository") {
    const url = new URL(EXTENSION_SOURCE.url, location.href).href;
    await connection.query(`SET custom_extension_repository='${url}'`);
  }

  // Serialised deliberately, not with Promise.all: see the retry note above.
  const extensions: LoadedExtension[] = [];
  for (const name of EXTENSIONS) {
    onProgress(`Loading the ${name} extension…`);
    extensions.push(await loadExtension(connection, name));
  }

  return { db, connection, worker, extensions };
}

/** Pull a readable message out of whatever DuckDB or the browser threw. */
export function describe(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;
  return String(error);
}
