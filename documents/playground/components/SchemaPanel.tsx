interface Column {
  name: string;
  type: string;
}

interface SchemaPanelProps {
  columns: readonly Column[];
  loading: boolean;
  error: string | null;
  onInsert: (name: string) => void;
}

/**
 * What the current statement selects, from `DESCRIBE` — which reads the Parquet
 * footer and none of the data.
 *
 * This exists because CityParquet's column names are not guessable. Faced with
 * `geometry_lod2_2`, `geometry_properties_lod2_2` and 60-odd `b3_*` attributes,
 * a first-time visitor otherwise has to leave the page to find out what to type.
 */
export default function SchemaPanel({ columns, loading, error, onInsert }: SchemaPanelProps) {
  return (
    <aside className="cp-schema" aria-label="Columns">
      <h2 className="cp-schema-title">Columns</h2>

      {loading && <p className="cp-dim-text">Reading the footer…</p>}

      {error && !loading && <p className="cp-dim-text">{error}</p>}

      {!loading && !error && columns.length === 0 && (
        <p className="cp-dim-text">
          Run a statement to see the columns it selects. Only the file footer is read.
        </p>
      )}

      {!loading && columns.length > 0 && (
        <>
          <p className="cp-dim-text">
            {columns.length.toLocaleString()} column
            {columns.length === 1 ? "" : "s"}. Click one to insert it.
          </p>
          <ul className="cp-schema-list">
            {columns.map((column) => (
              <li key={column.name}>
                <button
                  type="button"
                  className="cp-schema-item"
                  onClick={() => onInsert(column.name)}
                  title={`${column.name} — ${column.type}`}
                >
                  <span className="cp-schema-name">{column.name}</span>
                  <span className="cp-schema-type">{shorten(column.type)}</span>
                </button>
              </li>
            ))}
          </ul>
        </>
      )}
    </aside>
  );
}

/**
 * CityParquet types get long — the 3DBAG `address` column is a STRUCT of nine
 * fields inside a list. The full type stays in the `title`.
 */
function shorten(type: string): string {
  if (type.length <= 18) return type;
  const head = type.match(/^[A-Z_]+/)?.[0];
  return head ? `${head}…` : `${type.slice(0, 17)}…`;
}
