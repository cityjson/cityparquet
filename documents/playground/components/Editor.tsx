import { useEffect, useRef } from "react";
import { EditorState } from "@codemirror/state";
import { EditorView, keymap, lineNumbers, highlightActiveLine, tooltips } from "@codemirror/view";
import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import {
  autocompletion,
  type CompletionContext,
  type CompletionSource,
} from "@codemirror/autocomplete";
import { sql, PostgreSQL } from "@codemirror/lang-sql";
import { tags } from "@lezer/highlight";

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
  /** Columns, struct fields and functions. Keywords come from `sql()` itself. */
  completionSource?: CompletionSource;
}

/**
 * The colours, against the tags `@codemirror/lang-sql` actually emits — it has
 * seventeen, and the ones left out inherit the surrounding text rather than
 * being given a colour that means nothing. `lang-sql` supplies the parse tree;
 * CodeMirror renders it monochrome until a highlight style like this one names
 * what each tag should look like.
 *
 * The values are the site's own, so a SELECT here reads as a SELECT does in the
 * specification: `--cp-syntax-*` resolves to GitHub Light or GitHub Dark, which
 * is the pair Shiki renders the documentation's code blocks with.
 */
const highlight = HighlightStyle.define([
  { tag: [tags.keyword, tags.operator, tags.typeName], color: "var(--cp-syntax-keyword)" },
  // Quoted identifiers are `special(string)`: "my column" is not a string, but
  // it is quoted, and colouring it as one is what every SQL editor does.
  { tag: [tags.string, tags.special(tags.string)], color: "var(--cp-syntax-string)" },
  { tag: [tags.number, tags.bool, tags.null], color: "var(--cp-syntax-number)" },
  { tag: [tags.standard(tags.name), tags.special(tags.name)], color: "var(--cp-syntax-builtin)" },
  {
    tag: [tags.lineComment, tags.blockComment],
    color: "var(--cp-syntax-comment)",
    fontStyle: "italic",
  },
]);

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
  completionSource,
}: EditorProps) {
  const host = useRef<HTMLDivElement | null>(null);
  const view = useRef<EditorView | null>(null);
  // Kept in refs so the CodeMirror extensions never close over a stale render.
  const onChangeRef = useRef(onChange);
  const onRunRef = useRef(onRun);
  const completionRef = useRef(completionSource);
  onChangeRef.current = onChange;
  onRunRef.current = onRun;
  // The editor is built once, but the completion source only exists after the
  // database is up. Reading it through a ref lets it arrive late without
  // reconfiguring the editor.
  completionRef.current = completionSource;

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
        // Upper-case, because that is how every preset — and the specification
        // — writes a keyword, and a completion should match the house style.
        sql({ dialect: PostgreSQL, upperCaseKeywords: true }),
        // Registered as language data rather than as an `override`, so the
        // keyword source `sql()` installs above keeps working alongside it.
        PostgreSQL.language.data.of({
          autocomplete: (context: CompletionContext) =>
            completionRef.current ? completionRef.current(context) : null,
        }),
        // The class is the hook the page's own styles hang on, so the popup
        // follows light/dark with everything else rather than shipping
        // CodeMirror's defaults.
        autocompletion({ tooltipClass: () => "cp-completion" }),
        // The editor shell scrolls and is capped at 28rem, so a tooltip laid
        // out inside it is clipped at the bottom of the box — exactly where a
        // completion popup wants to appear. Fixed positioning escapes it.
        tooltips({ position: "fixed" }),
        syntaxHighlighting(highlight),
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
