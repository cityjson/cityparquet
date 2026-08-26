// Everything environment-dependent about the playground, in one file.

/**
 * Where the CityParquet data lives. The playground reads it over plain HTTPS
 * with range requests, so the host must expose the range headers — see
 * `README.md` for the exact CORS policy, and why `Access-Control-Expose-Headers`
 * is not optional.
 */
export const DATA_BASE_URL = "https://cityparquet.open3d.city/data";

/** Resolve a path against the data host. */
export const data = (path: string): string =>
  `${DATA_BASE_URL}/${path}`.replace(/([^:]\/)\/+/g, "$1");

/** The extensions the playground knows how to load. */
export const EXTENSIONS = ["cityjson", "three_d"] as const;
export type ExtensionName = (typeof EXTENSIONS)[number];

export type ExtensionSource =
  /** `INSTALL <name> FROM community` — the published, signed builds. */
  | { readonly kind: "community" }
  /**
   * A DuckDB extension repository laid out as
   * `<url>/<duckdb-version>/<platform>/<name>.duckdb_extension.wasm`, served
   * from anywhere with permissive CORS.
   */
  | { readonly kind: "repository"; readonly url: string };

/**
 * Community builds by default, because those are the ones a visitor can rely on
 * being there. A local repository is opt-in through
 * `PUBLIC_EXT_REPOSITORY=/ext`, which is how you test against extension code
 * that has not been published yet — the published builds lag this repository's
 * sources considerably, so anything newly added needs the override.
 *
 * Astro only exposes `PUBLIC_`-prefixed variables to client code, and inlines
 * them at build time.
 */
const repository = import.meta.env.PUBLIC_EXT_REPOSITORY as string | undefined;

export const EXTENSION_SOURCE: ExtensionSource = repository
  ? { kind: "repository", url: repository }
  : { kind: "community" };

/**
 * A locally built artefact is not signed by DuckDB Labs, so loading one needs
 * `allowUnsignedExtensions`. The community builds are signed and do not.
 */
export const ALLOW_UNSIGNED = EXTENSION_SOURCE.kind === "repository";

/** Rows held in the results grid. A query may match far more; the UI says so. */
export const ROW_DISPLAY_CAP = 5_000;

/**
 * Rows shown at once. The cap above is what a result may hold; this is what the
 * page renders, so a five-thousand-row answer does not turn into a five-
 * thousand-row page. The grid scrolls within a fixed height and pages through
 * the rest.
 */
export const ROWS_PER_PAGE = 500;

/**
 * Per-query deadline. A host that answers range requests but hides
 * `Accept-Ranges` from the browser produces no error at all — the read simply
 * never completes — so a query with no deadline can hang forever. This turns
 * that into a message the reader can act on.
 *
 * Long, deliberately. The point is to catch a read that will never finish, not
 * to cut short one that is merely large: a scan of the 16.4 GB package over a
 * slow connection is a legitimate several minutes, and a deadline that fires
 * first reports a fault where there is none. The status bar counts the seconds
 * up meanwhile, so a query in flight is visibly working rather than hung.
 */
export const QUERY_TIMEOUT_MS = 600_000;
