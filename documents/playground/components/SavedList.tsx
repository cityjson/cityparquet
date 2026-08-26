import { useState } from "react";

import type { SavedQuery } from "../lib/saved";

interface SavedListProps {
  queries: readonly SavedQuery[];
  activeSql: string;
  available: boolean;
  onSelect: (query: SavedQuery) => void;
  onRename: (id: string, name: string) => void;
  onRemove: (id: string) => void;
}

/**
 * The reader's own queries, newest first.
 *
 * Renaming is here rather than at save time because a save should cost one
 * click: the name is derived from the query, and corrected later by whoever
 * cares. Most never will.
 */
export default function SavedList({
  queries,
  activeSql,
  available,
  onSelect,
  onRename,
  onRemove,
}: SavedListProps) {
  const [editing, setEditing] = useState<string | null>(null);
  const [draft, setDraft] = useState("");

  const commit = () => {
    if (editing && draft.trim()) onRename(editing, draft.trim());
    setEditing(null);
  };

  if (!available) {
    return (
      <p className="cp-dim-text">
        This browser is not storing data — private mode, or storage turned off — so queries cannot
        be kept here. A share link still carries one: the address bar is always a link to what the
        editor holds.
      </p>
    );
  }

  if (queries.length === 0) {
    return (
      <p className="cp-dim-text">
        Nothing saved yet. <strong>Save in browser</strong> keeps the current query here, in this
        browser only — nothing is uploaded, and clearing site data clears it.
      </p>
    );
  }

  return (
    <ul className="cp-saved-list">
      {queries.map((query) => (
        <li
          key={query.id}
          className={query.sql === activeSql ? "cp-saved cp-saved-active" : "cp-saved"}
        >
          {editing === query.id ? (
            <input
              className="cp-saved-input"
              value={draft}
              autoFocus
              onChange={(event) => setDraft(event.target.value)}
              onBlur={commit}
              onKeyDown={(event) => {
                if (event.key === "Enter") commit();
                if (event.key === "Escape") setEditing(null);
              }}
              aria-label="Query name"
            />
          ) : (
            <button
              type="button"
              className="cp-saved-open"
              onClick={() => onSelect(query)}
              title={query.sql}
            >
              <span className="cp-saved-name">{query.name}</span>
              <span className="cp-saved-date">{when(query.savedAt)}</span>
            </button>
          )}
          <span className="cp-saved-actions">
            <button
              type="button"
              className="cp-saved-action"
              onClick={() => {
                setDraft(query.name);
                setEditing(query.id);
              }}
              aria-label={`Rename ${query.name}`}
              title="Rename"
            >
              ✎
            </button>
            <button
              type="button"
              className="cp-saved-action"
              onClick={() => onRemove(query.id)}
              aria-label={`Delete ${query.name}`}
              title="Delete"
            >
              ×
            </button>
          </span>
        </li>
      ))}
    </ul>
  );
}

/** Recent saves are the ones being worked with, so they get the finer grain. */
function when(savedAt: number): string {
  const seconds = Math.max(0, Math.round((Date.now() - savedAt) / 1000));
  if (seconds < 60) return "just now";
  if (seconds < 3_600) return `${Math.floor(seconds / 60)} min ago`;
  if (seconds < 86_400) return `${Math.floor(seconds / 3_600)} h ago`;
  const days = Math.floor(seconds / 86_400);
  if (days < 7) return `${days} d ago`;
  return new Date(savedAt).toLocaleDateString();
}
