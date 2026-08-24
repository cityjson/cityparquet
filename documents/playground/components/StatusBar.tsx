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
 */
export default function StatusBar({ result, extensions, running }: StatusBarProps) {
  const loaded = extensions.filter((extension) => extension.error === null);

  return (
    <div className="cp-status" role="status">
      <div className="cp-status-metrics">
        {running && <span className="cp-status-item cp-running">Running…</span>}

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

function formatDuration(ms: number): string {
  if (ms < 1_000) return `${Math.round(ms)} ms`;
  return `${(ms / 1_000).toFixed(ms < 10_000 ? 2 : 1)} s`;
}
