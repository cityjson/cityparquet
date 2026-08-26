import { useEffect, useMemo, useState } from "react";

import { formatBytes } from "../lib/bytes";
import { rowBands, type FileStructure, type ScanColumn } from "../lib/scan";
import type { QueryResult } from "../lib/query";

/** Drawn rows. Past this each row stands for several row groups, and says so. */
const MAX_BANDS = 40;

interface ScanVizProps {
  /** Null until the footer has been read, or when the query reads no one file. */
  structure: FileStructure | null;
  /** The columns the statement names — a subset of the file's, or all of them. */
  projected: readonly ScanColumn[];
  running: boolean;
  /** The statement as it stands, so a stale result can be told from a current one. */
  sql: string;
  result: QueryResult | null;
  loading: boolean;
  /** Further Parquet files the statement reads but this figure does not draw. */
  otherFiles: number;
}

/**
 * The file as a grid of column chunks, with the part this query reads lit up.
 *
 * A Parquet file is a rectangle: one chunk per (row group, column). A reader
 * that understands the footer fetches only the chunks it needs, which is the
 * whole argument for keeping city models in this encoding rather than in one
 * document that has to be parsed from the top. The status bar states that as a
 * number — "6.6 MB read" — and this says where those bytes were.
 *
 * Three honesty constraints shape what is drawn:
 *
 *   * The projection is real. It comes from the identifiers in the statement
 *     intersected with the file's own schema, so a `WHERE` column counts.
 *   * The **row groups are not**. Which ones survive a filter is decided inside
 *     the engine against footer statistics, and mapping the fetched byte ranges
 *     back onto chunks needs data the worker cannot hand over mid-statement.
 *     So every row group is drawn as a candidate, and nothing here claims a
 *     count of "row groups scanned".
 *   * Nothing animates as if it were being measured live. While the statement
 *     runs, a sweep says "working" the way a spinner does; the figures appear
 *     only once they are facts.
 */
export default function ScanViz({
  structure,
  projected,
  running,
  sql,
  result,
  loading,
  otherFiles,
}: ScanVizProps) {
  const [hovered, setHovered] = useState<number | null>(null);
  // Held back a beat after a result lands, so the fill is a transition the eye
  // can follow rather than a state the grid is simply already in. A timer
  // rather than `requestAnimationFrame`, which does not fire in a background
  // tab — a reader who switches away mid-query would come back to a figure
  // still waiting to draw itself.
  const [filled, setFilled] = useState(false);

  // Only a result for the statement now in the editor describes the file now
  // drawn. Switching preset leaves the previous result on screen, and billing
  // its bytes to a file it never read would invert the figure's whole point.
  const measured = result && result.sql.trim() === sql.trim() ? result : null;

  // A different file means the column under the pointer no longer exists.
  useEffect(() => setHovered(null), [structure]);

  useEffect(() => {
    if (running || !measured) {
      setFilled(false);
      return;
    }
    const timer = setTimeout(() => setFilled(true), 30);
    return () => clearTimeout(timer);
  }, [running, measured]);

  const bands = useMemo(
    () => (structure ? rowBands(structure.rowGroups, MAX_BANDS) : []),
    [structure],
  );

  const isProjected = useMemo(() => {
    const names = new Set(projected.map((column) => column.name));
    return (column: ScanColumn) => names.has(column.name);
  }, [projected]);

  if (loading && !structure) {
    return (
      <div className="cp-scan cp-scan-waiting">
        <span className="cp-spinner" aria-hidden="true" />
        <span>Reading the file&rsquo;s footer…</span>
      </div>
    );
  }

  if (!structure || structure.columns.length === 0 || bands.length === 0) return null;

  const { columns, rowGroups, rows, target, bytes } = structure;
  const readBytes = !running && measured ? measured.bytesRead : null;
  const share = readBytes !== null && bytes ? (readBytes / bytes) * 100 : null;
  const active = hovered !== null ? columns[hovered] : null;

  const state = running ? "running" : filled ? "read" : "plan";

  return (
    <figure className={`cp-scan cp-scan-${state}`}>
      <figcaption className="cp-scan-head">
        <span className="cp-scan-file">{target.label}</span>
        <span className="cp-scan-shape">
          {columns.length} columns × {rowGroups.toLocaleString()}{" "}
          {rowGroups === 1 ? "row group" : "row groups"} · {rows.toLocaleString()} rows
          {bytes !== null && <> · {formatBytes(bytes)}</>}
        </span>
        {otherFiles > 0 && (
          <span className="cp-scan-more">first of {otherFiles + 1} files this query reads</span>
        )}
      </figcaption>

      <div className="cp-scan-readout" aria-live="off">
        {active ? (
          <>
            <code>{active.name}</code>
            <span className="cp-scan-type">{active.type}</span>
            <span className={isProjected(active) ? "cp-scan-tag-on" : "cp-scan-tag-off"}>
              {isProjected(active) ? "read by this query" : "not read"}
            </span>
          </>
        ) : (
          <span className="cp-scan-hint">Hover a column to name it.</span>
        )}
      </div>

      {/* Hover is read from the pointer's position across the whole grid, not
          from a handler on each cell: the header strip is nine pixels tall, and
          a reader who wants to know what a lit stripe is will point at the
          stripe. Thousands of per-cell handlers would do the same job for the
          cost of thousands of handlers. */}
      <div
        className="cp-scan-grid"
        style={{ ["--cp-scan-cols" as string]: columns.length }}
        onMouseMove={(event) => {
          const box = event.currentTarget.getBoundingClientRect();
          const index = Math.floor(((event.clientX - box.left) / box.width) * columns.length);
          setHovered(index >= 0 && index < columns.length ? index : null);
        }}
        onMouseLeave={() => setHovered(null)}
      >
        <div className="cp-scan-row cp-scan-header">
          {columns.map((column, index) => (
            <button
              key={column.name}
              type="button"
              className={cellClass(isProjected(column), hovered === index, true)}
              style={{ transitionDelay: `${index * 8}ms` }}
              title={`${column.name} — ${column.type}`}
              aria-label={`${column.name}, ${column.type}, ${
                isProjected(column) ? "read by this query" : "not read"
              }`}
              onMouseEnter={() => setHovered(index)}
              onFocus={() => setHovered(index)}
              onBlur={() => setHovered(null)}
            />
          ))}
        </div>

        {bands.map((band) => (
          <div className="cp-scan-row" key={band.from} title={bandLabel(band.from, band.to)}>
            {columns.map((column, index) => (
              <span
                key={column.name}
                className={cellClass(isProjected(column), false, false)}
                style={{ transitionDelay: `${index * 8 + band.from * 6}ms` }}
              />
            ))}
          </div>
        ))}
      </div>

      <div className="cp-scan-metrics">
        <Metric
          label="columns read"
          value={`${projected.length.toLocaleString()} / ${columns.length.toLocaleString()}`}
        />
        <Metric
          label={rowGroups === 1 ? "row group" : "row groups"}
          value={rowGroups.toLocaleString()}
          note={bands.length < rowGroups ? `${bands.length} drawn rows` : undefined}
        />
        <Metric
          label="bytes read"
          value={readBytes === null ? "—" : formatBytes(readBytes)}
          note={
            share === null
              ? undefined
              : `${share < 0.01 ? "<0.01" : share.toFixed(share < 1 ? 2 : 1)}% of the file`
          }
        />
      </div>

      <p className="cp-scan-note">
        {running
          ? "Reading. DuckDB fetches the chunks it needs as HTTP range requests; the sweep marks work in progress, not measured progress."
          : "Lit cells are the column chunks this statement can read — its projection against every row group. Which row groups survive the filter is decided inside the engine, so none is claimed here as skipped."}
      </p>
    </figure>
  );
}

function Metric({ label, value, note }: { label: string; value: string; note?: string }) {
  return (
    <div className="cp-scan-metric">
      <span className="cp-scan-metric-label">{label}</span>
      <strong className="cp-scan-metric-value">{value}</strong>
      {note && <span className="cp-scan-metric-note">{note}</span>}
    </div>
  );
}

function cellClass(projected: boolean, hovered: boolean, header: boolean): string {
  const parts = [header ? "cp-scan-hd" : "cp-scan-cell"];
  if (projected) parts.push("cp-scan-on");
  if (hovered) parts.push("cp-scan-hover");
  return parts.join(" ");
}

function bandLabel(from: number, to: number): string {
  return to - from === 1 ? `Row group ${from}` : `Row groups ${from}–${to - 1}`;
}
