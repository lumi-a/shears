import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import {
  bracketMatching,
  HighlightStyle,
  StreamLanguage,
  syntaxHighlighting,
} from "@codemirror/language";
import { toml } from "@codemirror/legacy-modes/mode/toml";
import { EditorState } from "@codemirror/state";
import {
  drawSelection,
  dropCursor,
  EditorView,
  highlightActiveLine,
  highlightSpecialChars,
  keymap,
  placeholder,
} from "@codemirror/view";
import { tags } from "@lezer/highlight";
import { createEffect, onCleanup, onMount } from "solid-js";

const setup = [
  highlightSpecialChars(),
  history(),
  drawSelection(),
  dropCursor(),
  bracketMatching(),
  highlightActiveLine(),
  keymap.of([...defaultKeymap, ...historyKeymap]),
];

// Colours come from CSS variables (see index.css).
const theme = EditorView.theme({
  "&": {
    height: "100%",
    color: "var(--fg)",
    backgroundColor: "var(--bg)",
    fontSize: "var(--font-size-small)",
  },
  "&.cm-focused": { outline: "none" },
  ".cm-scroller": { fontFamily: "var(--font-mono)" },
  ".cm-content": { caretColor: "var(--fg)" },
  ".cm-cursor, .cm-dropCursor": { borderLeftColor: "var(--fg)" },
  ".cm-gutters": {
    backgroundColor: "var(--bg-subtle)",
    color: "var(--fg-muted)",
    borderRight: "1px solid var(--border)",
  },
  ".cm-activeLine": { backgroundColor: "var(--editor-active-line)" },
  "&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground, .cm-selectionBackground":
    { backgroundColor: "var(--editor-selection)" },
  ".cm-placeholder": { color: "var(--fg-muted)" },
});

// Token names are what the legacy TOML mode emits: `property` (keys),
// `atom` (table headers, booleans, dates), `string`, `number`, `comment`.
const highlighting = HighlightStyle.define([
  { tag: tags.comment, color: "var(--syntax-comment)" },
  { tag: tags.propertyName, color: "var(--syntax-key)" },
  { tag: tags.atom, color: "var(--syntax-header)", fontWeight: "600" },
  { tag: tags.string, color: "var(--syntax-string)" },
  { tag: tags.number, color: "var(--syntax-number)" },
]);

const tomlLanguage = StreamLanguage.define(toml);

// A CodeMirror editor for one TOML document. `value` coming from outside
// (e.g. a preset's parsing.toml) replaces the document. Edits flow back
// through `onChange`.
export function TomlEditor(props: {
  value: string;
  onChange: (value: string) => void;
  // The editor's accessible name, e.g. "parsing.toml".
  label: string;
  // Shown while the document is empty.
  placeholder: string;
}) {
  let parent!: HTMLDivElement;
  let view: EditorView | undefined;

  onMount(() => {
    view = new EditorView({
      parent,
      state: EditorState.create({
        doc: props.value,
        extensions: [
          setup,
          EditorView.lineWrapping,
          tomlLanguage,
          syntaxHighlighting(highlighting),
          theme,
          placeholder(props.placeholder),
          EditorView.contentAttributes.of({ "aria-label": props.label }),
          // Reading props.onChange when an edit happens, not tracking it, is
          // the point: it always calls whichever handler is current.
          // eslint-disable-next-line solid/reactivity
          EditorView.updateListener.of((update) => {
            if (update.docChanged) props.onChange(update.state.doc.toString());
          }),
        ],
      }),
    });
  });

  createEffect(() => {
    const value = props.value;
    // Our own edits come back through here too, those already match.
    if (view !== undefined && value !== view.state.doc.toString()) {
      view.dispatch({
        changes: { from: 0, to: view.state.doc.length, insert: value },
      });
    }
  });

  onCleanup(() => view?.destroy());

  return <div class="toml-editor" ref={(element) => (parent = element)} />;
}
