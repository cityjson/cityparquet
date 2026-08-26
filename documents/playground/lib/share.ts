// Putting the current query in the URL, so it can be linked to.
//
// A preset that has not been edited travels as its id, which keeps a link from
// a tutorial short and legible. Once edited, the SQL itself travels, encoded so
// that it survives a URL without needing anything escaped by hand.

export interface SharedState {
  readonly presetId: string | null;
  readonly sql: string | null;
}

const EMPTY: SharedState = { presetId: null, sql: null };

/** base64url of the UTF-8 bytes: no padding, no characters needing escaping. */
export function encodeSql(sql: string): string {
  const bytes = new TextEncoder().encode(sql);
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

export function decodeSql(encoded: string): string | null {
  try {
    const padded = encoded.replace(/-/g, "+").replace(/_/g, "/");
    const binary = atob(padded + "=".repeat((4 - (padded.length % 4)) % 4));
    const bytes = Uint8Array.from(binary, (character) => character.charCodeAt(0));
    return new TextDecoder().decode(bytes);
  } catch {
    return null;
  }
}

/** Read `#preset=…` or `#sql=…`. Anything unparseable reads as empty. */
export function parseHash(hash: string): SharedState {
  const trimmed = hash.replace(/^#/, "");
  if (!trimmed) return EMPTY;
  const params = new URLSearchParams(trimmed);

  const sqlParam = params.get("sql");
  if (sqlParam) {
    const sql = decodeSql(sqlParam);
    if (sql !== null) return { presetId: null, sql };
  }

  const presetId = params.get("preset");
  if (presetId) return { presetId, sql: null };

  return EMPTY;
}

/** The hash for a state — `""` when there is nothing worth putting in the URL. */
export function buildHash(state: SharedState): string {
  if (state.sql) return `#sql=${encodeSql(state.sql)}`;
  if (state.presetId) return `#preset=${state.presetId}`;
  return "";
}
