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
 * so this is the normal case rather than an edge one.
 */
export default function ResultsTable({ result }: ResultsTableProps) {
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
            {result.rows.map((row, index) => (
              // Row order is the result's own; nothing reorders, so the index is stable.
              <tr key={index}>
                <td className="cp-rownum">{index + 1}</td>
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
      {result.truncated && (
        <p className="cp-truncated">
          Showing the first {result.rows.length.toLocaleString()} of{" "}
          {result.rowCount.toLocaleString()} rows. Add a <code>LIMIT</code>, or aggregate, to see
          the rest.
        </p>
      )}
    </div>
  );
}
