import { formatBytes } from "../lib/bytes";
import { starterSql, type ImportedFile } from "../lib/files";

interface ImportedFilesProps {
  files: readonly ImportedFile[];
  onUse: (sql: string) => void;
  onForget: (name: string) => void;
}

/**
 * The local files this tab has been given.
 *
 * They are worth showing rather than silently registering: a reader who has
 * imported one needs to know what to call it, and that it is theirs alone —
 * DuckDB reads it out of the browser's own file handle, so nothing was
 * uploaded and nothing survives the tab closing.
 */
export default function ImportedFiles({ files, onUse, onForget }: ImportedFilesProps) {
  if (files.length === 0) {
    return (
      <p className="cp-dim-text">
        Nothing imported. <strong>Import file</strong> opens a Parquet, CityJSON, CityJSONSeq or
        FlatCityBuf file from this machine. It is read in place, in ranges, exactly as a remote
        package is — nothing is uploaded.
      </p>
    );
  }

  return (
    <ul className="cp-imported-list">
      {files.map((file) => (
        <li key={file.name} className="cp-imported">
          <div className="cp-imported-head">
            <span className="cp-imported-name">{file.name}</span>
            <button
              type="button"
              className="cp-saved-action"
              onClick={() => onForget(file.name)}
              aria-label={`Forget ${file.name}`}
              title="Forget this file"
            >
              ×
            </button>
          </div>
          <p className="cp-imported-meta">
            {file.format.label} · {formatBytes(file.size)}
            {file.columns && ` · ${file.columns.length} columns`}
          </p>
          {file.error ? (
            <p className="cp-imported-error">{file.error}</p>
          ) : (
            <button
              type="button"
              className="cp-imported-use"
              onClick={() => onUse(starterSql(file))}
            >
              Query it
            </button>
          )}
        </li>
      ))}
    </ul>
  );
}
