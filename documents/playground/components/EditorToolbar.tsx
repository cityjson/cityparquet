import { useEffect, useRef, useState } from "react";

import { EXPORT_FORMATS, type ExportFormat } from "../lib/files";

interface EditorToolbarProps {
  onCopy: () => Promise<boolean>;
  onSave: () => void;
  onExplain: () => void;
  onImport: () => void;
  onExport: (format: ExportFormat) => void;
  /** False before the database is up, or while something is already running. */
  enabled: boolean;
  /** Whether saving will actually outlive this tab. */
  storageAvailable: boolean;
  exporting: string | null;
}

/**
 * The row of things you can do to the query that are not "run it".
 *
 * It sits above the editor rather than beside Run, because Run is the one
 * action with consequences worth a deliberate reach — these are all cheap, and
 * two of them (copy, save) touch nothing but the browser.
 */
export default function EditorToolbar({
  onCopy,
  onSave,
  onExplain,
  onImport,
  onExport,
  enabled,
  storageAvailable,
  exporting,
}: EditorToolbarProps) {
  // "Copied" is the whole feedback for a copy — there is nowhere else to look.
  const [copied, setCopied] = useState<"idle" | "done" | "failed">("idle");
  const [exportOpen, setExportOpen] = useState(false);
  const exportRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (copied === "idle") return;
    const timer = setTimeout(() => setCopied("idle"), 1_800);
    return () => clearTimeout(timer);
  }, [copied]);

  // A menu that stays open after you have clicked elsewhere is a bug people
  // report as "the page is broken", so close on both routes out of it.
  useEffect(() => {
    if (!exportOpen) return;
    const onPointer = (event: MouseEvent) => {
      if (!exportRef.current?.contains(event.target as Node)) setExportOpen(false);
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") setExportOpen(false);
    };
    document.addEventListener("mousedown", onPointer);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onPointer);
      document.removeEventListener("keydown", onKey);
    };
  }, [exportOpen]);

  return (
    <div className="cp-toolbar">
      <button
        type="button"
        className="cp-tool-button"
        onClick={async () => setCopied((await onCopy()) ? "done" : "failed")}
      >
        {copied === "done" ? "Copied" : copied === "failed" ? "Copy failed" : "Copy"}
      </button>

      <button
        type="button"
        className="cp-tool-button"
        onClick={onSave}
        title={
          storageAvailable
            ? "Keep this query in this browser"
            : "This browser is not storing data, so a saved query will not survive a reload"
        }
      >
        Save in browser
      </button>

      <button type="button" className="cp-tool-button" onClick={onExplain} disabled={!enabled}>
        Explain
      </button>

      <span className="cp-toolbar-gap" />

      <button type="button" className="cp-tool-button" onClick={onImport} disabled={!enabled}>
        Import file
      </button>

      <div className="cp-export" ref={exportRef}>
        <button
          type="button"
          className="cp-tool-button"
          onClick={() => setExportOpen((open) => !open)}
          disabled={!enabled}
          aria-expanded={exportOpen}
          aria-haspopup="menu"
        >
          {exporting ? `Exporting ${exporting}…` : "Export"}
          <span aria-hidden="true">▾</span>
        </button>
        {exportOpen && (
          <div className="cp-export-menu" role="menu">
            <p className="cp-export-note">
              Runs the query again and writes every row it matches, not only the ones shown.
            </p>
            {EXPORT_FORMATS.map((format) => (
              <button
                key={format.id}
                type="button"
                role="menuitem"
                className="cp-export-item"
                onClick={() => {
                  setExportOpen(false);
                  onExport(format);
                }}
              >
                <span>{format.label}</span>
                <span className="cp-export-ext">.{format.extension}</span>
              </button>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
