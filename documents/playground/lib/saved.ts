// Queries the reader has kept, in this browser and nowhere else.
//
// A share link already carries a query between people. This is the other half:
// keeping one for yourself, across a reload, without an account and without
// anything leaving the tab.
//
// The store is handed its `Storage` rather than reaching for `localStorage`,
// which keeps it testable outside a browser and makes the failures explicit.
// They are real: a page in private mode can throw from the accessor itself, and
// a full quota throws on write.

/** Bumped only if the stored shape changes in a way older data cannot satisfy. */
export const SAVED_VERSION = 1;

/**
 * The key is prefixed, and that matters more than it looks: GitHub project
 * pages share one origin, so `cityjson.github.io` is also every other cityjson
 * project's `localStorage`.
 */
export const SAVED_KEY = "cityparquet:playground:saved";

/** Enough to be useful, few enough to stay inside a modest storage quota. */
export const SAVED_LIMIT = 50;

export interface SavedQuery {
  readonly id: string;
  readonly name: string;
  readonly sql: string;
  /** Epoch milliseconds, so the list can be shown newest first. */
  readonly savedAt: number;
}

interface StoredShape {
  version: number;
  queries: SavedQuery[];
}

/**
 * A name for a query the reader did not name themselves.
 *
 * The presets open with a comment saying what they do, which is a better label
 * than the first line of SQL — so that is preferred where it exists.
 */
export function deriveName(sql: string, fallback = "Untitled query"): string {
  for (const line of sql.split("\n")) {
    const text = line.trim();
    if (!text) continue;
    const comment = text.match(/^--+\s*(.+)$/);
    const candidate = comment ? comment[1] : text;
    const cleaned = candidate
      .replace(/\s+/g, " ")
      .replace(/[;,]\s*$/, "")
      .trim();
    if (cleaned) return cleaned.length > 60 ? `${cleaned.slice(0, 59)}…` : cleaned;
  }
  return fallback;
}

/** Distinct without needing `crypto.randomUUID`, which is not everywhere. */
function nextId(existing: readonly SavedQuery[]): string {
  let highest = 0;
  for (const query of existing) {
    const number = Number.parseInt(query.id.replace(/^q/, ""), 10);
    if (Number.isFinite(number) && number > highest) highest = number;
  }
  return `q${highest + 1}`;
}

/**
 * The saved list, and the four things the interface does to it.
 *
 * Every method returns the resulting list, so a caller can hold it in state
 * without reading storage again — and a read that fails returns an empty list
 * rather than throwing, because a browser that will not store queries should
 * still run them.
 */
export class SavedQueries {
  private readonly storage: Storage | null;

  constructor(storage: Storage | null) {
    this.storage = storage;
  }

  /**
   * The store for the current browser, or one backed by nothing at all when
   * storage is unavailable — private mode, or a browser set to refuse it.
   */
  static forBrowser(): SavedQueries {
    try {
      // Touching `localStorage` is itself what throws when it is blocked, so
      // the access has to be inside the guard, not just the write.
      const storage = typeof localStorage === "undefined" ? null : localStorage;
      storage?.getItem(SAVED_KEY);
      return new SavedQueries(storage);
    } catch {
      return new SavedQueries(null);
    }
  }

  /** Whether anything saved here will actually survive the reader leaving. */
  get available(): boolean {
    return this.storage !== null;
  }

  list(): SavedQuery[] {
    const raw = this.read();
    // Newest first: the thing you just saved is the thing you want to see.
    return [...raw].sort((a, b) => b.savedAt - a.savedAt);
  }

  /** Keep a query. Saving the same SQL twice updates the one already there. */
  save(sql: string, name: string, now: number): SavedQuery[] {
    const queries = this.read();
    const existing = queries.find((query) => query.sql === sql);
    if (existing) {
      return this.write(
        queries.map((query) =>
          query.id === existing.id ? { ...query, name, savedAt: now } : query,
        ),
      );
    }
    const saved: SavedQuery = { id: nextId(queries), name, sql, savedAt: now };
    // Oldest out at the limit, rather than refusing to save.
    const kept = [...queries, saved].sort((a, b) => b.savedAt - a.savedAt).slice(0, SAVED_LIMIT);
    return this.write(kept);
  }

  rename(id: string, name: string): SavedQuery[] {
    return this.write(this.read().map((query) => (query.id === id ? { ...query, name } : query)));
  }

  remove(id: string): SavedQuery[] {
    return this.write(this.read().filter((query) => query.id !== id));
  }

  private read(): SavedQuery[] {
    if (!this.storage) return [];
    try {
      const raw = this.storage.getItem(SAVED_KEY);
      if (!raw) return [];
      const parsed = JSON.parse(raw) as StoredShape | null;
      if (!parsed || parsed.version !== SAVED_VERSION || !Array.isArray(parsed.queries)) return [];
      // Anything malformed is dropped rather than trusted: this is data from a
      // previous version of the page, or from another tab.
      return parsed.queries.filter(
        (query): query is SavedQuery =>
          typeof query?.id === "string" &&
          typeof query?.name === "string" &&
          typeof query?.sql === "string" &&
          typeof query?.savedAt === "number",
      );
    } catch {
      return [];
    }
  }

  private write(queries: SavedQuery[]): SavedQuery[] {
    const sorted = [...queries].sort((a, b) => b.savedAt - a.savedAt);
    if (!this.storage) return sorted;
    try {
      const shape: StoredShape = { version: SAVED_VERSION, queries: sorted };
      this.storage.setItem(SAVED_KEY, JSON.stringify(shape));
    } catch {
      // A full quota, or storage disabled between construction and now. The
      // list the caller renders is still correct for this session.
    }
    return sorted;
  }
}
