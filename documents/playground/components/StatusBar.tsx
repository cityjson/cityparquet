import { useEffect, useState } from "react";

import { formatBytes } from "../lib/bytes";
import type { LoadedExtension } from "../lib/duckdb";
import type { QueryResult } from "../lib/query";

interface StatusBarProps {
  result: QueryResult | null;
  extensions: readonly LoadedExtension[];
  running: boolean;
}

/**
 * Rows, time, bytes, and which extensions are actually loaded.
 *
 * The byte figure is the one worth having: a plain SQL console can tell you a
 * query took 900 ms, but not that it read 3 MB of a 16.4 GB file to do it. When
 * the counter is unavailable the field is omitted entirely rather than shown as
 * zero — a wrong number here would undercut the very claim it exists to support.
 *
 * While a query is in flight the time counts up. A read of the national package
 * can run for tens of seconds against a slow host, and a bare "Running…" for
 * that long is indistinguishable from a page that has stopped working; a moving
 * number says which of the two this is, and how close it is to the deadline.
 */
export default function StatusBar({ result, extensions, running }: StatusBarProps) {
  const loaded = extensions.filter((extension) => extension.error === null);
  const elapsed = useElapsed(running);

  return (
    <div className="cp-status" role="status">
      <div className="cp-status-metrics">
        {running && (
          <span className="cp-status-item cp-running">
            Running…{" "}
            {/* Hidden from assistive technology: this sits in a live region, and
                a number changing ten times a second would be announced as often.
                "Running…" beside it carries the same news, once. */}
            <strong aria-hidden="true">{formatTicking(elapsed)}</strong>
          </span>
        )}

        {!running && result && (
          <>
            <span className="cp-status-item">
              <strong>{result.rowCount.toLocaleString()}</strong>{" "}
              {result.rowCount === 1 ? "row" : "rows"}
            </span>
            <span className="cp-status-item">
              <strong>{formatDuration(result.elapsedMs)}</strong>
            </span>
            {result.bytesRead !== null && (
              <span className="cp-status-item" title="Bytes fetched over the network">
                <strong>{formatBytes(result.bytesRead)}</strong> read
              </span>
            )}
          </>
        )}
      </div>

      <div className="cp-status-extensions">
        {loaded.map((extension) => (
          <span
            key={extension.name}
            className="cp-badge"
            title={`${extension.name} ${extension.version ?? ""}`.trim()}
          >
            {extension.name}
            {extension.version && <span className="cp-badge-version">{extension.version}</span>}
          </span>
        ))}
        {extensions
          .filter((extension) => extension.error !== null)
          .map((extension) => (
            <span
              key={extension.name}
              className="cp-badge cp-badge-failed"
              title={extension.error ?? undefined}
            >
              {extension.name} unavailable
            </span>
          ))}
      </div>
    </div>
  );
}

/**
 * Milliseconds since the run began, while one is in flight, and 0 otherwise.
 *
 * Ten ticks a second: fast enough to read as running rather than as frozen,
 * slow enough that the last digit is legible rather than a blur.
 */
function useElapsed(running: boolean): number {
  const [elapsed, setElapsed] = useState(0);

  useEffect(() => {
    if (!running) return;
    const started = performance.now();
    setElapsed(0);
    const timer = setInterval(() => setElapsed(performance.now() - started), 100);
    return () => clearInterval(timer);
  }, [running]);

  return elapsed;
}

/**
 * The running clock, always in seconds to one decimal.
 *
 * `formatDuration` switches units at a second, which is right for a figure that
 * has settled and wrong for one that has not: a counter that reads in
 * milliseconds and then flips to seconds looks like it has restarted.
 */
function formatTicking(ms: number): string {
  return `${(ms / 1_000).toFixed(1)} s`;
}

function formatDuration(ms: number): string {
  if (ms < 1_000) return `${Math.round(ms)} ms`;
  return `${(ms / 1_000).toFixed(ms < 10_000 ? 2 : 1)} s`;
}
