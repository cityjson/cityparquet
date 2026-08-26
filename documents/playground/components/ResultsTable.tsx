import { useEffect, useMemo, useState } from "react";

import { ROWS_PER_PAGE } from "../config";
import type { QueryResult } from "../lib/query";

interface ResultsTableProps {
  result: QueryResult;
}

/**
 * The results grid. Deliberately plain: the interesting object on this page is
 * the data, not the table widget.
 *
 * Wide results scroll inside their own container so the page body never scrolls
 * sideways — CityParquet object tables are 88 columns wide in the 3DBAG case,
 * so this is the normal case rather than an edge one. Tall results are held to
 * one page for the same reason in the other axis: the grid keeps a fixed
 * height, the header stays put while the rows move under it, and anything past
 * `ROWS_PER_PAGE` is a page turn rather than a longer document.
 */
export default function ResultsTable({ result }: ResultsTableProps) {
  const pages = Math.max(1, Math.ceil(result.rows.length / ROWS_PER_PAGE));
  const [page, setPage] = useState(0);

  // Every run produces a fresh result object, and the reader expects to be
  // looking at its first rows rather than wherever the previous answer was left.
  // The effect runs after the render that first sees the new result, so the page
  // is also clamped here: without it a shorter answer renders one empty frame.
  useEffect(() => setPage(0), [result]);
  const current = Math.min(page, pages - 1);

  const start = current * ROWS_PER_PAGE;
  const visible = useMemo(
    () => result.rows.slice(start, start + ROWS_PER_PAGE),
    [result, start],
  );

  if (result.columns.length === 0) {
    return <p className="cp-empty">The statement returned no columns.</p>;
  }

  if (result.rows.length === 0) {
    return <p className="cp-empty">No rows.</p>;
  }

  return (
    <div className="cp-results">
      <div className="cp-table-scroll">
        <table className="cp-table">
          <thead>
            <tr>
              <th className="cp-rownum" scope="col">
                <span className="cp-sr">Row</span>
              </th>
              {result.columns.map((column) => (
                <th key={column} scope="col" title={column}>
                  {column}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {visible.map((row, index) => (
              // Row order is the result's own; nothing reorders, so the index is stable.
              <tr key={start + index}>
                <td className="cp-rownum">{start + index + 1}</td>
                {row.map((cell, cellIndex) => (
                  <td
                    key={result.columns[cellIndex] ?? cellIndex}
                    className={cell === null ? "cp-null" : undefined}
                    title={cell === null ? "NULL" : String(cell)}
                  >
                    {cell === null ? "NULL" : String(cell)}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      {pages > 1 && (
        <nav className="cp-pager" aria-label="Result pages">
          <button
            type="button"
            className="cp-page-button"
            onClick={() => setPage((current) => Math.max(0, current - 1))}
            disabled={current === 0}
          >
            ‹ Previous
          </button>
          {/* One template literal: JSX drops the spaces around a line break,
              and this line is mostly spaces between numbers. */}
          <span className="cp-page-count">
            {`Rows ${(start + 1).toLocaleString()}–${(start + visible.length).toLocaleString()} of ${result.rows.length.toLocaleString()}`}
          </span>
          <button
            type="button"
            className="cp-page-button"
            onClick={() => setPage((current) => Math.min(pages - 1, current + 1))}
            disabled={current >= pages - 1}
          >
            Next ›
          </button>
        </nav>
      )}

      {result.truncated && (
        /* Each run of prose is its own expression: JSX eats the spaces around
           a line break, and every gap here sits next to a number or an
           element. */
        <p className="cp-truncated">
          {`Showing the first ${result.rows.length.toLocaleString()} of ${result.rowCount.toLocaleString()} rows. Add a `}
          <code>LIMIT</code>
          {`, or aggregate, to see the rest.`}
        </p>
      )}
    </div>
  );
}
