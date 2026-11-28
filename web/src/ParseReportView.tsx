import {
  createMemo,
  createSignal,
  For,
  Match,
  Show,
  Switch,
  type JSX,
  type ParentProps,
} from "solid-js";
import { LinkText, ShowAll, TagText, type Target } from "./Details";
import type { About, Diagnostic, ParseReport } from "./session";

// How many entries a list renders before "and N more". A big export has tens
// of thousands of diagnostics, let's not bloat the poor DOM.
const SHOWN_PER_LIST = 200;

// A <details> whose contents are only rendered while it's open.
export function LazyDetails(props: ParentProps<{ summary: JSX.Element }>) {
  const [open, setOpen] = createSignal(false);
  return (
    <details onToggle={(event) => setOpen(event.currentTarget.open)}>
      <summary>{props.summary}</summary>
      <Show when={open()}>{props.children}</Show>
    </details>
  );
}

// A list that renders its first entries, each with `children`, and a button
// to render the rest.
export function CappedList<T>(props: {
  items: T[];
  children: (item: T) => JSX.Element;
}) {
  const [all, setAll] = createSignal(false);
  const shown = () =>
    all() ? props.items : props.items.slice(0, SHOWN_PER_LIST);
  return (
    <ul>
      <For each={shown()}>{(item) => <li>{props.children(item)}</li>}</For>
      <ShowAll
        hidden={props.items.length - shown().length}
        onClick={() => setAll(true)}
      />
    </ul>
  );
}

// An item or recipe id that opens in the Explorer.
export function OpenButton(props: {
  target: Target;
  open: (target: Target) => void;
}) {
  return (
    <button
      type="button"
      class="link"
      title="Open in the Explorer"
      onClick={() => props.open(props.target)}
    >
      <LinkText link={{ ...props.target, live: null }} />
    </button>
  );
}

// What a diagnostic is about. An item opens in the Explorer, a tag searches
// for its items there, and a file is only shown.
function AboutLink(props: {
  about: About;
  open: (target: Target) => void;
  search: (query: string) => void;
  showFile: (path: string) => void;
}) {
  const of = <K extends About["kind"]>(kind: K) =>
    props.about.kind === kind
      ? (props.about as Extract<About, { kind: K }>)
      : undefined;
  return (
    <Switch>
      <Match when={of("item")}>
        {(item) => <OpenButton target={item()} open={props.open} />}
      </Match>
      <Match when={of("tag")}>
        {(tag) => (
          <button
            type="button"
            class="link"
            title="Search for items with this tag"
            onClick={() => props.search(`{ tag = "${tag().id}" }`)}
          >
            <TagText id={tag().id} />
          </button>
        )}
      </Match>
      <Match when={of("file")}>
        {(file) => (
          <button
            type="button"
            class="link"
            title="Show this file"
            onClick={() => props.showFile(file().path)}
          >
            <span class="file">{file().path}</span>
          </button>
        )}
      </Match>
    </Switch>
  );
}

interface DiagnosticGroup {
  message: string;
  abouts: Diagnostic[];
}

// Most diagnostics repeat one of a few hundred messages ("no adapter for
// recipe type X yet"). So we group by message and sort by group size.
function groupByMessage(diagnostics: Diagnostic[]): DiagnosticGroup[] {
  const groups = new Map<string, Diagnostic[]>();
  for (const diagnostic of diagnostics) {
    const abouts = groups.get(diagnostic.message);
    if (abouts === undefined) {
      groups.set(diagnostic.message, [diagnostic]);
    } else {
      abouts.push(diagnostic);
    }
  }
  return [...groups]
    .map(([message, abouts]) => ({ message, abouts }))
    .sort(
      (a, b) =>
        b.abouts.length - a.abouts.length || a.message.localeCompare(b.message),
    );
}

export function ParseReportView(props: {
  report: ParseReport;
  // Opens an item in the Explorer.
  open: (target: Target) => void;
  // Searches for `query` in the Explorer.
  search: (query: string) => void;
  // Shows the export's file at `path`.
  showFile: (path: string) => void;
}) {
  const groups = createMemo(() => groupByMessage(props.report.diagnostics));
  const recipeTypes = createMemo(() =>
    Object.entries(props.report.recipe_type_counts).sort(
      ([a, aCount], [b, bCount]) => bCount - aCount || a.localeCompare(b),
    ),
  );

  return (
    <section class="parse-report">
      <h3>
        Diagnostics ({props.report.diagnostics.length} across {groups().length})
      </h3>
      <For each={groups()}>
        {(group) => (
          <LazyDetails
            summary={
              <>
                <span class="count">{group.abouts.length}</span>
                <span class="label">{group.message}</span>
              </>
            }
          >
            <CappedList items={group.abouts}>
              {(diagnostic) => (
                <AboutLink
                  about={diagnostic.about}
                  open={props.open}
                  search={props.search}
                  showFile={props.showFile}
                />
              )}
            </CappedList>
          </LazyDetails>
        )}
      </For>

      <h3>Recipe files</h3>
      <LazyDetails
        summary={
          <>
            <span class="count">{recipeTypes().length}</span>
            <span class="label">recipe types</span>
          </>
        }
      >
        <table>
          <thead>
            <tr>
              <th>type</th>
              <th>files</th>
            </tr>
          </thead>
          <tbody>
            <For each={recipeTypes()}>
              {([type, count]) => (
                <tr>
                  <td>{type}</td>
                  <td class="count">{count}</td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </LazyDetails>
      <LazyDetails
        summary={
          <>
            <span class="count">
              {props.report.registered_but_unreferenced.length}
            </span>
            <span class="label">
              registered ids no recipe or tag references
            </span>
          </>
        }
      >
        <CappedList items={props.report.registered_but_unreferenced}>
          {(id) => (
            <OpenButton target={{ kind: "item", id }} open={props.open} />
          )}
        </CappedList>
      </LazyDetails>
    </section>
  );
}
