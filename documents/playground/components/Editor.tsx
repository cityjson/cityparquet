import { useEffect, useRef } from "react";
import { EditorState } from "@codemirror/state";
import { EditorView, keymap, lineNumbers, highlightActiveLine } from "@codemirror/view";
import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import { sql, PostgreSQL } from "@codemirror/lang-sql";

export interface EditorHandle {
  /** Insert text at the cursor and keep focus, as a click-to-insert should. */
  insertAtCursor: (text: string) => void;
}

interface EditorProps {
  value: string;
  onChange: (value: string) => void;
  onRun: () => void;
  disabled: boolean;
  handleRef?: { current: EditorHandle | null };
}

/**
 * CodeMirror 6, wired to the site's theme tokens rather than a packaged theme,
 * so the editor follows light/dark with everything else on the page.
 */
export default function Editor({
  value,
  onChange,
  onRun,
  disabled,
  handleRef,
}: EditorProps) {
  const host = useRef<HTMLDivElement | null>(null);
  const view = useRef<EditorView | null>(null);
  // Kept in refs so the CodeMirror extensions never close over a stale render.
  const onChangeRef = useRef(onChange);
  const onRunRef = useRef(onRun);
  onChangeRef.current = onChange;
  onRunRef.current = onRun;

  useEffect(() => {
    if (!host.current) return;

    const state = EditorState.create({
      doc: value,
      extensions: [
        lineNumbers(),
        highlightActiveLine(),
        history(),
        // Run before defaultKeymap so Mod-Enter is ours, not the editor's.
        keymap.of([
          {
            key: "Mod-Enter",
            preventDefault: true,
            run: () => {
              onRunRef.current();
              return true;
            },
          },
        ]),
        keymap.of([...defaultKeymap, ...historyKeymap]),
        sql({ dialect: PostgreSQL, upperCaseKeywords: false }),
        EditorView.lineWrapping,
        EditorView.updateListener.of((update) => {
          if (update.docChanged) onChangeRef.current(update.state.doc.toString());
        }),
        EditorView.theme({
          "&": { fontSize: "0.875rem", backgroundColor: "transparent", height: "100%" },
          "&.cm-focused": { outline: "none" },
          ".cm-content": { fontFamily: "var(--cp-mono)", padding: "0.75rem 0" },
          ".cm-gutters": {
            backgroundColor: "transparent",
            border: "none",
            color: "var(--cp-dim)",
          },
          ".cm-activeLine": { backgroundColor: "var(--cp-hover)" },
          ".cm-activeLineGutter": { backgroundColor: "transparent" },
          ".cm-scroller": { fontFamily: "var(--cp-mono)", lineHeight: "1.6" },
        }),
      ],
    });

    const instance = new EditorView({ state, parent: host.current });
    view.current = instance;
    return () => {
      instance.destroy();
      view.current = null;
    };
    // Created once: subsequent value changes are reconciled below.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Push external changes (a preset click, a shared link) into the editor
  // without disturbing the cursor when the text already matches.
  useEffect(() => {
    const instance = view.current;
    if (!instance) return;
    const current = instance.state.doc.toString();
    if (current === value) return;
    instance.dispatch({
      changes: { from: 0, to: current.length, insert: value },
    });
  }, [value]);

  useEffect(() => {
    view.current?.contentDOM.setAttribute("aria-disabled", String(disabled));
  }, [disabled]);

  // Expose cursor insertion to the schema panel. Appending to the end of the
  // document would nearly always produce SQL that does not parse.
  useEffect(() => {
    if (!handleRef) return;
    handleRef.current = {
      insertAtCursor(text: string) {
        const instance = view.current;
        if (!instance) return;
        const { from, to } = instance.state.selection.main;
        instance.dispatch({
          changes: { from, to, insert: text },
          selection: { anchor: from + text.length },
        });
        instance.focus();
      },
    };
    return () => {
      handleRef.current = null;
    };
  }, [handleRef]);

  return <div className="cp-editor" ref={host} />;
}
