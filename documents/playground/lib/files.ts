// Bringing the reader's own data in, and taking results back out.
//
// Nothing is uploaded. A Parquet file is handed over as a *handle*:
// `registerFileHandle` with `BROWSER_FILEREADER` lets DuckDB pull ranges out of
// the browser's own file reader on demand, the same access pattern the remote
// packages get over HTTPS, which is why a local 16 GB package is no more
// alarming than a remote one. The text readers get the bytes instead — see
// `registration` below for why that distinction is not a detail.

import * as duckdb from "@duckdb/duckdb-wasm";

import { QUERY_TIMEOUT_MS } from "../config";
import { describe, type Session } from "./duckdb";

/** A format the playground knows how to read from a local file. */
export interface ImportFormat {
  readonly id: string;
  readonly label: string;
  /** Extensions offered in the file picker, and used to guess the format. */
  readonly extensions: readonly string[];
  /** The table function a starter query calls. */
  readonly reader: string;
  /** The extension that supplies `reader`, when it is not core DuckDB. */
  readonly extension: "cityjson" | null;
  /**
   * How the file is handed to DuckDB.
   *
   * `handle` is a lazy reader: DuckDB pulls ranges out of the browser's file
   * handle as it needs them, which is what makes a local package no more
   * alarming than a remote one. It only works for a reader that seeks, though —
   * the text readers walk a file start to end and hang on a handle rather than
   * failing — so those get `buffer`, which reads the whole file into memory
   * first. Text city models are read whole anyway.
   */
  readonly registration: "handle" | "buffer";
}

export const IMPORT_FORMATS: readonly ImportFormat[] = [
  {
    id: "cityparquet",
    label: "CityParquet / Parquet",
    extensions: [".parquet"],
    reader: "read_parquet",
    extension: null,
    registration: "handle",
  },
  {
    id: "cityjson",
    label: "CityJSON",
    extensions: [".json", ".city.json"],
    reader: "read_cityjson",
    extension: "cityjson",
    registration: "buffer",
  },
  {
    id: "cityjsonseq",
    label: "CityJSONSeq",
    extensions: [".jsonl", ".city.jsonl", ".ndjson"],
    reader: "read_cityjsonseq",
    extension: "cityjson",
    registration: "buffer",
  },
  {
    id: "flatcitybuf",
    label: "FlatCityBuf",
    extensions: [".fcb"],
    reader: "read_flatcitybuf",
    extension: "cityjson",
    registration: "handle",
  },
];

/**
 * How long to wait for a reader to open an imported file.
 *
 * Short, because this is a local file: there is no network to be slow, so a
 * reader still working after this is one that is not going to finish.
 */
const IMPORT_TIMEOUT_MS = 20_000;

/** Everything the file picker should offer, for its `accept` attribute. */
export const IMPORT_ACCEPT = IMPORT_FORMATS.flatMap((format) => format.extensions).join(",");

/**
 * Which reader a file needs, by extension. Longest suffix first, so
 * `.city.jsonl` is not read as `.json`-anything.
 */
export function formatFor(filename: string): ImportFormat | null {
  const lower = filename.toLowerCase();
  const candidates = IMPORT_FORMATS.flatMap((format) =>
    format.extensions.map((extension) => ({ format, extension })),
  ).sort((a, b) => b.extension.length - a.extension.length);
  return candidates.find(({ extension }) => lower.endsWith(extension))?.format ?? null;
}

/**
 * A name DuckDB can hold and a query can quote.
 *
 * The registered name is pasted into `read_parquet('…')`, so a quote or a
 * newline in it would not merely look wrong — it would end the string literal.
 * Spaces are legal inside the quotes but make the starter query hard to read,
 * so they go too.
 */
export function safeName(filename: string): string {
  const cleaned = filename
    .replace(/^.*[\\/]/, "")
    .replace(/[^A-Za-z0-9._-]+/g, "_")
    .replace(/^_+|_+$/g, "");
  return cleaned || "imported";
}

export interface ImportedFile {
  /** The name DuckDB knows it by, and the one the query quotes. */
  readonly name: string;
  readonly format: ImportFormat;
  readonly size: number;
  /** Columns from the footer, or null when the reader could not open it. */
  readonly columns: readonly { name: string; type: string }[] | null;
  readonly error: string | null;
}

/** The query that reads one imported file — what the reader starts from. */
export function starterSql(file: ImportedFile): string {
  return `SELECT * FROM ${file.format.reader}('${file.name}')\nLIMIT 20;`;
}

/**
 * Register one local file and look at what is in it.
 *
 * Registration and the `DESCRIBE` are one exclusive block: both touch the
 * worker, and a statement of the reader's landing between them would be reading
 * a database that is halfway through changing.
 */
export async function importFile(session: Session, file: File): Promise<ImportedFile> {
  const format = formatFor(file.name);
  if (!format) {
    return {
      name: safeName(file.name),
      format: IMPORT_FORMATS[0],
      size: file.size,
      columns: null,
      error:
        `Nothing here reads "${file.name}". Expected one of ` +
        `${IMPORT_FORMATS.flatMap((f) => f.extensions).join(", ")}.`,
    };
  }

  const name = safeName(file.name);
  return session.exclusive(async () => {
    // Dropping first makes re-importing a changed file do what it looks like it
    // does; registering over a live name otherwise keeps the old handle.
    try {
      await session.db.dropFile(name);
    } catch {
      // Not registered yet, which is the usual case.
    }
    if (format.registration === "buffer") {
      await session.db.registerFileBuffer(name, new Uint8Array(await file.arrayBuffer()));
    } else {
      await session.db.registerFileHandle(
        name,
        file,
        duckdb.DuckDBDataProtocol.BROWSER_FILEREADER,
        true,
      );
    }

    try {
      // Bounded, and that is not belt-and-braces. A reader that cannot cope
      // with how this file was registered does not always fail — it can simply
      // never return, and since every statement shares one queue, an unbounded
      // wait here would take the whole page down with it.
      const table = await Promise.race([
        session.connection.query(`DESCRIBE SELECT * FROM ${format.reader}('${name}')`),
        new Promise<never>((_resolve, reject) =>
          setTimeout(
            () =>
              reject(
                new Error(
                  `${format.reader} did not open ${name} within ` +
                    `${Math.round(IMPORT_TIMEOUT_MS / 1000)}s.`,
                ),
              ),
            IMPORT_TIMEOUT_MS,
          ),
        ),
      ]);
      const columns = table.toArray().map((row) => {
        const record = row.toJSON() as Record<string, unknown>;
        return { name: String(record.column_name ?? ""), type: String(record.column_type ?? "") };
      });
      return { name, format, size: file.size, columns, error: null };
    } catch (cause) {
      // The file stays registered: the reader may load a newer extension build,
      // or query it another way. Only the schema is unavailable.
      return { name, format, size: file.size, columns: null, error: describe(cause) };
    }
  });
}

/** Forget a file, so its name is free and its handle released. */
export async function forgetFile(session: Session, name: string): Promise<void> {
  await session.exclusive(async () => {
    try {
      await session.db.dropFile(name);
    } catch {
      // Already gone.
    }
  });
}

/** A format the playground can write a result out as. */
export interface ExportFormat {
  readonly id: string;
  readonly label: string;
  readonly extension: string;
  /** The `FORMAT` given to `COPY`. */
  readonly copyFormat: string;
  readonly mime: string;
  /**
   * True when the writer needs CityParquet's own columns rather than whatever
   * the query happened to select. An aggregate cannot become a city model.
   */
  readonly needsCityColumns: boolean;
  readonly extensionName: "cityjson" | null;
}

export const EXPORT_FORMATS: readonly ExportFormat[] = [
  {
    id: "csv",
    label: "CSV",
    extension: "csv",
    copyFormat: "csv",
    mime: "text/csv",
    needsCityColumns: false,
    extensionName: null,
  },
  {
    id: "json",
    label: "JSON",
    extension: "json",
    copyFormat: "json",
    mime: "application/json",
    needsCityColumns: false,
    extensionName: null,
  },
  {
    id: "parquet",
    label: "Parquet",
    extension: "parquet",
    copyFormat: "parquet",
    mime: "application/vnd.apache.parquet",
    needsCityColumns: false,
    extensionName: null,
  },
  {
    id: "cityjsonseq",
    label: "CityJSONSeq",
    extension: "city.jsonl",
    copyFormat: "cityjsonseq",
    mime: "application/x-ndjson",
    needsCityColumns: true,
    extensionName: "cityjson",
  },
  {
    id: "flatcitybuf",
    label: "FlatCityBuf",
    extension: "fcb",
    copyFormat: "flatcitybuf",
    mime: "application/octet-stream",
    needsCityColumns: true,
    extensionName: "cityjson",
  },
];

export interface ExportResult {
  readonly bytes: Uint8Array;
  readonly filename: string;
  readonly mime: string;
}

/**
 * Run the query again and write its rows out in `format`.
 *
 * Again, deliberately: `COPY` takes a statement, not a result set, so this is a
 * second execution and exports everything the query matches — not the rows the
 * grid is holding, which are capped. The interface says so.
 *
 * COPY, read-back and cleanup are one exclusive block. Between the write and
 * the read the file exists only inside DuckDB's virtual filesystem, and a
 * statement of the reader's landing in the middle could see it, or replace it.
 */
export async function exportQuery(
  session: Session,
  sql: string,
  format: ExportFormat,
  stem = "cityparquet-result",
): Promise<ExportResult> {
  const statement = sql.trim().replace(/;\s*$/, "");
  const target = `${stem}.${format.extension}`;

  const work = session.exclusive(async () => {
    try {
      const copied = await session.connection.query(
        `COPY (${statement}) TO '${target}' (FORMAT ${format.copyFormat})`,
      );
      const bytes = await session.db.copyFileToBuffer(target);

      // `COPY` answers with the number of rows it wrote, and that number is not
      // the same claim as the file existing. The cityjson extension's city
      // writers report a row count and leave nothing behind in DuckDB-Wasm's
      // virtual filesystem — the write goes somewhere the browser build cannot
      // reach. Downloading the empty result would be the worst outcome of the
      // three, so the mismatch is checked rather than trusted.
      const row = copied.toArray()[0] as { Count?: unknown } | undefined;
      const claimed = Number(row?.Count ?? 0);
      if (claimed > 0 && bytes.length <= 1) {
        throw new Error(
          `The ${format.label} writer reported ${claimed.toLocaleString()} rows but produced ` +
            "an empty file.",
        );
      }
      return bytes;
    } finally {
      try {
        await session.db.dropFile(target);
      } catch {
        // Never written, because the COPY failed.
      }
    }
  });

  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    const bytes = await Promise.race([
      work,
      new Promise<never>((_resolve, reject) => {
        timer = setTimeout(
          () =>
            reject(
              new Error(
                `Timeout after ${Math.round(QUERY_TIMEOUT_MS / 1000)}s. An export runs the ` +
                  "query again over every row it matches, which can be far more than the " +
                  "grid was showing.",
              ),
            ),
          QUERY_TIMEOUT_MS,
        );
      }),
    ]);
    return { bytes, filename: target, mime: format.mime };
  } finally {
    if (timer !== undefined) clearTimeout(timer);
  }
}

/**
 * Explain why a writer refused, when the reason is one the reader can act on.
 *
 * Two are common enough to be worth naming. The city writers need CityParquet's
 * own columns, so an aggregate cannot become CityJSONSeq however well-formed it
 * is; and the published extension builds lag this project's sources, so a
 * writer that exists in the source may simply not be in the loaded build.
 */
export function explainExportFailure(format: ExportFormat, message: string): string | null {
  const text = message.toLowerCase();
  if (text.includes("produced an empty file")) {
    return (
      `${message} That is a limitation of the published ${format.extensionName ?? "extension"} ` +
      "build rather than of the query: its city writers do not write into DuckDB-Wasm's virtual " +
      "filesystem, so the file never arrives even though the rows were converted. The same " +
      "COPY works in the DuckDB CLI. Exporting as Parquet or CSV is unaffected."
    );
  }
  // The phrasing is DuckDB's own, observed rather than guessed: a missing
  // writer reads "Copy Function with name cityjsonseq does not exist", and a
  // missing reader "Catalog Error: Table Function with name … does not exist!".
  if (
    format.extensionName &&
    (text.includes("does not exist") ||
      text.includes("catalog error") ||
      text.includes("not supported"))
  ) {
    return (
      `The loaded ${format.extensionName} build has no ${format.label} writer. The published ` +
      "community builds lag this project's sources, so a format added recently is not in them yet."
    );
  }
  if (format.needsCityColumns) {
    return (
      `${format.label} is a city model, not a table: the writer needs the columns a CityParquet ` +
      "row carries — id, object_type, and a geometry column. A query that aggregates them away " +
      "cannot be written back out as one."
    );
  }
  return null;
}

/** Hand the bytes to the browser as a download. */
export function download(result: ExportResult): void {
  const url = URL.createObjectURL(new Blob([result.bytes as BlobPart], { type: result.mime }));
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = result.filename;
  document.body.append(anchor);
  anchor.click();
  anchor.remove();
  // Revoked on a turn of the event loop rather than at once: Safari has been
  // known to cancel a download whose object URL is released too eagerly.
  setTimeout(() => URL.revokeObjectURL(url), 10_000);
}
