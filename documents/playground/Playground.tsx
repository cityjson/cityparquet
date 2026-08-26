// DuckDB-Wasm touches Worker and window, so this must never be server-rendered.
export const client = "only";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { EXTENSION_SOURCE } from "./config";
import { SchemaCache, cityParquetCompletion } from "./lib/completion";
import { createSession, describe, type Session } from "./lib/duckdb";
import {
  IMPORT_ACCEPT,
  download,
  exportQuery,
  explainExportFailure,
  forgetFile,
  importFile,
  starterSql,
  type ExportFormat,
  type ImportedFile,
} from "./lib/files";
import {
  QueryError,
  describeQuery,
  explainQuery,
  runQuery,
  rowsOf,
  type QueryResult,
} from "./lib/query";
import { parquetTargets, projectedColumns, readStructure, type FileStructure } from "./lib/scan";
import { SavedQueries, deriveName, type SavedQuery } from "./lib/saved";
import { buildHash, parseHash } from "./lib/share";
import { DEFAULT_PRESET_ID, PRESETS, findPreset, type Preset } from "./presets";

import Editor, { type EditorHandle } from "./components/Editor";
import EditorToolbar from "./components/EditorToolbar";
import ImportedFiles from "./components/ImportedFiles";
import PresetList from "./components/PresetList";
import ResultsTable from "./components/ResultsTable";
import ScanViz from "./components/ScanViz";
import SavedList from "./components/SavedList";
import SchemaPanel from "./components/SchemaPanel";
import StatusBar from "./components/StatusBar";

type Boot =
  | { status: "starting"; message: string }
  | { status: "ready"; session: Session }
  | { status: "failed"; message: string };

interface Column {
  name: string;
  type: string;
}

/** Which list the left column is showing. */
type Tab = "examples" | "saved" | "files";

/** The query the page opens with: a shared link if there is one, else a preset. */
function initialState(): { sql: string; presetId: string | null } {
  const shared = typeof location === "undefined" ? null : parseHash(location.hash);
  if (shared?.sql) return { sql: shared.sql, presetId: null };

  const preset = findPreset(shared?.presetId ?? null) ?? findPreset(DEFAULT_PRESET_ID);
  return { sql: preset?.sql ?? "", presetId: preset?.id ?? null };
}

export default function Playground() {
  const [boot, setBoot] = useState<Boot>({ status: "starting", message: "Starting…" });
  const [{ sql, presetId }, setQueryState] = useState(initialState);
  const [result, setResult] = useState<QueryResult | null>(null);
  const [error, setError] = useState<QueryError | Error | null>(null);
  const [running, setRunning] = useState(false);
  const [columns, setColumns] = useState<Column[]>([]);
  const [schemaLoading, setSchemaLoading] = useState(false);
  const [schemaError, setSchemaError] = useState<string | null>(null);
  const [tab, setTab] = useState<Tab>("examples");
  const [plan, setPlan] = useState<string | null>(null);
  const [imported, setImported] = useState<ImportedFile[]>([]);
  const [exporting, setExporting] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [structure, setStructure] = useState<FileStructure | null>(null);
  const [structureLoading, setStructureLoading] = useState(false);
  const [otherFiles, setOtherFiles] = useState(0);

  // Created once, and only in the browser: constructing it touches
  // `localStorage`, which is what throws when a browser refuses storage.
  const [store] = useState(() => SavedQueries.forBrowser());
  const [saved, setSaved] = useState<SavedQuery[]>([]);
  useEffect(() => setSaved(store.list()), [store]);

  const fileInput = useRef<HTMLInputElement | null>(null);

  // One footer read per file, kept for the session. The shape of a file does
  // not change under us, and re-reading it on every keystroke would spend a
  // round trip to learn what is already known.
  const structures = useRef(new Map<string, FileStructure>());

  // The editor's current text, readable from callbacks without re-binding them.
  const sqlRef = useRef(sql);
  sqlRef.current = sql;

  // Imperative handle, so the schema panel can insert at the cursor.
  const editorRef = useRef<EditorHandle | null>(null);

  // What the editor completes against. Bound to the session, because both
  // halves — the columns of the files the statement reads, and the functions
  // the loaded extensions added — are answers only this database can give.
  const readySession = boot.status === "ready" ? boot.session : null;
  const schemaCache = useMemo(
    () => (readySession ? new SchemaCache((statement) => rowsOf(readySession, statement)) : null),
    [readySession],
  );
  const completionSource = useMemo(
    () => (schemaCache ? cityParquetCompletion(schemaCache) : undefined),
    [schemaCache],
  );

  useEffect(() => {
    let cancelled = false;
    createSession((message) => {
      if (!cancelled) setBoot({ status: "starting", message });
    })
      .then((session) => {
        if (cancelled) {
          void session.connection.close();
          return;
        }
        setBoot({ status: "ready", session });
      })
      .catch((cause) => {
        if (!cancelled) setBoot({ status: "failed", message: describe(cause) });
      });
    return () => {
      cancelled = true;
    };
  }, []);

  // Keep the URL in step, so the address bar is always a link to what is shown.
  useEffect(() => {
    const hash = buildHash({ presetId, sql: presetId ? null : sql });
    const next = `${location.pathname}${location.search}${hash}`;
    history.replaceState(null, "", next);
  }, [sql, presetId]);

  const run = useCallback(async () => {
    if (boot.status !== "ready" || running) return;
    const statement = sqlRef.current.trim();
    if (!statement) return;

    setRunning(true);
    setError(null);
    try {
      const next = await runQuery(boot.session, statement);
      setPlan(null);
      setResult(next);
    } catch (cause) {
      setResult(null);
      setError(cause instanceof Error ? cause : new Error(String(cause)));
    } finally {
      setRunning(false);
    }
  }, [boot, running]);

  // Refresh the column list for whatever the editor holds. DESCRIBE reads the
  // footer only, so this is cheap enough to run on a debounce.
  useEffect(() => {
    if (boot.status !== "ready") return;
    const statement = sql.trim().replace(/;\s*$/, "");
    if (!statement || !/^\s*(select|with|from)/i.test(statement)) {
      setColumns([]);
      setSchemaError(null);
      return;
    }

    let cancelled = false;
    setSchemaLoading(true);
    const timer = setTimeout(() => {
      describeQuery(boot.session, statement)
        .then((next) => {
          if (cancelled) return;
          setColumns(next);
          setSchemaError(null);
        })
        .catch((cause) => {
          if (cancelled) return;
          setColumns([]);
          setSchemaError(describe(cause));
        })
        .finally(() => {
          if (!cancelled) setSchemaLoading(false);
        });
    }, 700);

    return () => {
      cancelled = true;
      clearTimeout(timer);
      setSchemaLoading(false);
    };
  }, [sql, boot]);

  // The shape of the file the statement reads, for the scan figure. Footer
  // only, and cached per URL, so it shares the schema panel's debounce rather
  // than firing at a half-typed URL.
  useEffect(() => {
    if (boot.status !== "ready") return;

    const targets = parquetTargets(sql);
    setOtherFiles(Math.max(0, targets.length - 1));
    if (targets.length === 0) {
      setStructure(null);
      setStructureLoading(false);
      return;
    }

    // The first file a query names. A join reads two, and drawing one of them
    // is more use than drawing neither — the figure says how many it is not
    // showing.
    const target = targets[0];
    const known = structures.current.get(target.url);
    if (known) {
      setStructure(known);
      setStructureLoading(false);
      return;
    }

    let cancelled = false;
    setStructureLoading(true);
    const timer = setTimeout(() => {
      readStructure(boot.session, target)
        .then((next) => {
          if (cancelled) return;
          structures.current.set(target.url, next);
          setStructure(next);
        })
        .catch(() => {
          // A file that cannot be described is one the figure cannot draw.
          // The query itself will report the failure; this stays quiet.
          if (!cancelled) setStructure(null);
        })
        .finally(() => {
          if (!cancelled) setStructureLoading(false);
        });
    }, 700);

    return () => {
      cancelled = true;
      clearTimeout(timer);
      setStructureLoading(false);
    };
  }, [sql, boot]);

  // Fetch the schema the editor completes against before anyone asks for it.
  // Sharing the schema panel's debounce is deliberate: both wait for typing to
  // settle, and neither should fire on a half-written URL.
  useEffect(() => {
    if (!schemaCache) return;
    const timer = setTimeout(() => schemaCache.warm(sql), 700);
    return () => clearTimeout(timer);
  }, [sql, schemaCache]);

  const selectPreset = useCallback((preset: Preset) => {
    setQueryState({ sql: preset.sql, presetId: preset.id });
    setError(null);
    setPlan(null);
  }, []);

  // Editing a preset makes the query the user's own, so the share link switches
  // from the preset id to the SQL itself.
  const editSql = useCallback((next: string) => {
    setQueryState((current) => ({
      sql: next,
      presetId:
        current.presetId && next === findPreset(current.presetId)?.sql ? current.presetId : null,
    }));
  }, []);

  // Which of the file's columns this statement names. Recomputed as it is
  // typed, so the figure answers before the query is run.
  const projected = useMemo(
    () => (structure ? projectedColumns(sql, structure.columns) : []),
    [sql, structure],
  );

  const insertColumn = useCallback((name: string) => {
    editorRef.current?.insertAtCursor(name);
  }, []);

  // Running or explaining replaces whatever the other one left on screen, so
  // the panel below the editor always answers the last thing that was asked.
  const explain = useCallback(async () => {
    if (boot.status !== "ready" || running) return;
    const statement = sqlRef.current.trim();
    if (!statement) return;

    setRunning(true);
    setError(null);
    try {
      const text = await explainQuery(boot.session, statement);
      setResult(null);
      setPlan(text || "DuckDB returned no plan for that statement.");
    } catch (cause) {
      setPlan(null);
      setError(cause instanceof Error ? cause : new Error(String(cause)));
    } finally {
      setRunning(false);
    }
  }, [boot, running]);

  const copy = useCallback(async () => {
    try {
      await navigator.clipboard.writeText(sqlRef.current);
      return true;
    } catch {
      // Denied permission, or an insecure context. There is no fallback worth
      // having: `execCommand("copy")` needs a selection the editor already owns.
      return false;
    }
  }, []);

  const save = useCallback(() => {
    const statement = sqlRef.current.trim();
    if (!statement) return;
    const preset = presetId ? findPreset(presetId) : undefined;
    setSaved(store.save(statement, preset?.title ?? deriveName(statement), Date.now()));
    setTab("saved");
    setNotice(
      store.available ? null : "Saved for this session only — this browser is not storing data.",
    );
  }, [presetId, store]);

  const openSaved = useCallback((query: SavedQuery) => {
    setQueryState({ sql: query.sql, presetId: null });
    setError(null);
    setPlan(null);
  }, []);

  const runImport = useCallback(
    async (files: FileList | null) => {
      if (!files || boot.status !== "ready") return;
      setNotice(null);
      for (const file of Array.from(files)) {
        const next = await importFile(boot.session, file);
        // The completion cache keys on the expression text, and an expression
        // naming this file may already be in it from a previous import. Drop it
        // so the new file's columns are the ones offered.
        schemaCache?.forget(starterSql(next));
        setImported((current) => [next, ...current.filter((f) => f.name !== next.name)]);
      }
      setTab("files");
    },
    [boot, schemaCache],
  );

  // An untouched preset is replaced; anything hand-written is inserted into
  // instead. Overwriting SQL someone has written to make room for a starter
  // query destroys work to save a click.
  const useImported = useCallback(
    (sql: string) => {
      if (presetId) setQueryState({ sql, presetId: null });
      else editorRef.current?.insertAtCursor(sql);
      setPlan(null);
    },
    [presetId],
  );

  const forget = useCallback(
    async (name: string) => {
      if (boot.status !== "ready") return;
      await forgetFile(boot.session, name);
      setImported((current) => current.filter((file) => file.name !== name));
    },
    [boot],
  );

  const exportAs = useCallback(
    async (format: ExportFormat) => {
      if (boot.status !== "ready" || exporting) return;
      const statement = sqlRef.current.trim();
      if (!statement) return;

      setExporting(format.label);
      setNotice(null);
      try {
        download(await exportQuery(boot.session, statement, format));
      } catch (cause) {
        const message = describe(cause);
        setNotice(explainExportFailure(format, message) ?? message);
      } finally {
        setExporting(null);
      }
    },
    [boot, exporting],
  );

  return (
    <div className="cp-playground">
      <header className="cp-header">
        <div>
          <h1>SQL playground</h1>
          <p className="cp-subtitle">
            DuckDB, the <code>cityjson</code> and <code>three_d</code> extensions, and 16.4 GB of
            3DBAG on object storage — running entirely in this tab. Nothing is uploaded, and only
            the bytes each query needs are fetched.
          </p>
        </div>
        <button
          type="button"
          className="cp-run"
          onClick={run}
          disabled={boot.status !== "ready" || running}
        >
          {running ? "Running…" : "Run"}
          <kbd>{navigatorIsMac() ? "⌘" : "Ctrl"}+↵</kbd>
        </button>
      </header>

      {boot.status === "starting" && (
        <div className="cp-boot">
          <span className="cp-spinner" aria-hidden="true" />
          <span>{boot.message}</span>
        </div>
      )}

      {boot.status === "failed" && (
        <div className="cp-error">
          <h2>DuckDB could not start</h2>
          <pre>{boot.message}</pre>
          <p>
            This needs WebAssembly and Web Workers. If you are running a browser that blocks either,
            the playground cannot work.
          </p>
        </div>
      )}

      {boot.status === "ready" && (
        <div className="cp-layout">
          <div className="cp-sidebar">
            <div className="cp-tabs" role="tablist" aria-label="Queries and files">
              {(
                [
                  ["examples", "Examples", PRESETS.length],
                  ["saved", "Saved", saved.length],
                  ["files", "Files", imported.length],
                ] as const
              ).map(([id, label, count]) => (
                <button
                  key={id}
                  type="button"
                  role="tab"
                  aria-selected={tab === id}
                  className={tab === id ? "cp-tab cp-tab-active" : "cp-tab"}
                  onClick={() => setTab(id)}
                >
                  {label}
                  {count > 0 && <span className="cp-tab-count">{count}</span>}
                </button>
              ))}
            </div>

            <div className="cp-sidebar-body">
              {tab === "examples" && (
                <PresetList presets={PRESETS} activeId={presetId} onSelect={selectPreset} />
              )}
              {tab === "saved" && (
                <SavedList
                  queries={saved}
                  activeSql={sql.trim()}
                  available={store.available}
                  onSelect={openSaved}
                  onRename={(id, name) => setSaved(store.rename(id, name))}
                  onRemove={(id) => setSaved(store.remove(id))}
                />
              )}
              {tab === "files" && (
                <ImportedFiles files={imported} onUse={useImported} onForget={forget} />
              )}
            </div>
          </div>

          <main className="cp-main">
            <EditorToolbar
              onCopy={copy}
              onSave={save}
              onExplain={explain}
              onImport={() => fileInput.current?.click()}
              onExport={exportAs}
              enabled={!running && !exporting}
              storageAvailable={store.available}
              exporting={exporting}
            />
            <input
              ref={fileInput}
              type="file"
              accept={IMPORT_ACCEPT}
              multiple
              className="cp-sr"
              onChange={(event) => {
                void runImport(event.target.files);
                // Cleared so re-picking the same file fires `change` again.
                event.target.value = "";
              }}
            />

            <div className="cp-editor-shell">
              <Editor
                value={sql}
                onChange={editSql}
                onRun={run}
                disabled={running}
                handleRef={editorRef}
                completionSource={completionSource}
              />
            </div>

            <StatusBar result={result} extensions={boot.session.extensions} running={running} />

            <ScanViz
              structure={structure}
              projected={projected}
              running={running}
              sql={sql}
              result={result}
              loading={structureLoading}
              otherFiles={otherFiles}
            />

            {notice && <p className="cp-notice">{notice}</p>}

            {error && <ErrorPanel error={error} />}

            {!error && plan && (
              <figure className="cp-plan">
                <figcaption>
                  Query plan — <code>EXPLAIN</code> only, so no rows were read.
                </figcaption>
                <pre>{plan}</pre>
              </figure>
            )}

            {!error && !plan && result && <ResultsTable result={result} />}

            {!error && !plan && !result && !running && (
              <p className="cp-empty">
                Pick an example on the left, or write your own, and press Run.
              </p>
            )}
          </main>

          <SchemaPanel
            columns={columns}
            loading={schemaLoading}
            error={schemaError}
            onInsert={insertColumn}
          />
        </div>
      )}

      <footer className="cp-footer">
        <p>
          Extensions load from{" "}
          {EXTENSION_SOURCE.kind === "community" ? (
            <>the DuckDB community repository</>
          ) : (
            <>
              a local build at <code>{EXTENSION_SOURCE.url}</code>
            </>
          )}
          . Queries run against public data over HTTPS range requests.
        </p>
        <p>
          DuckDB runs here as WebAssembly, which fetches those ranges one at a time on a single
          thread; native DuckDB fetches them in parallel. A query over the 16.4 GB package takes
          tens of seconds in a tab where the same query takes a few seconds natively. It reads the
          same bytes either way — the difference is the browser, not the encoding.
        </p>
      </footer>
    </div>
  );
}

/** Distinguish the three failures that need different things from the reader. */
function ErrorPanel({ error }: { error: QueryError | Error }) {
  const failure = error instanceof QueryError ? error.failure : null;

  const heading =
    failure?.kind === "network"
      ? "The data could not be read"
      : failure?.kind === "timeout"
        ? "The query did not finish"
        : failure?.kind === "extension"
          ? "An extension is missing"
          : "That query did not run";

  return (
    <div className="cp-error">
      <h2>{heading}</h2>
      <pre>{error.message}</pre>
      {failure?.kind === "network" && (
        <p>
          A host serving data to a browser must expose its range headers —{" "}
          <code>Accept-Ranges</code>, <code>Content-Range</code> and <code>ETag</code> — through{" "}
          <code>Access-Control-Expose-Headers</code>. Without them a reader cannot tell that the
          file supports range requests.
        </p>
      )}
      {failure?.kind === "extension" && (
        <p>
          The published extension builds lag this project&rsquo;s sources, so a function added
          recently may not exist in the loaded build yet.
        </p>
      )}
    </div>
  );
}

function navigatorIsMac(): boolean {
  if (typeof navigator === "undefined") return false;
  return /mac/i.test(navigator.platform || navigator.userAgent);
}
