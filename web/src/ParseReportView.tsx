import {
  createMemo,
  createSignal,
  For,
  Show,
  type JSX,
  type ParentProps,
} from "solid-js";
import type { Diagnostic, ParseReport } from "./session";

// How many entries a list renders before "and N more". A big export has tens
// of thousands of diagnostics, let's not bloat the poor DOM.
const SHOWN_PER_LIST = 200;

// A <details> whose contents are only rendered while it's open.
function LazyDetails(props: ParentProps<{ summary: JSX.Element }>) {
  const [open, setOpen] = createSignal(false);
  return (
    <details onToggle={(event) => setOpen(event.currentTarget.open)}>
      <summary>{props.summary}</summary>
      <Show when={open()}>{props.children}</Show>
    </details>
  );
}

function CappedList(props: { items: string[] }) {
  const hidden = () => props.items.length - SHOWN_PER_LIST;
  return (
    <ul>
      <For each={props.items.slice(0, SHOWN_PER_LIST)}>
        {(item) => <li>{item}</li>}
      </For>
      <Show when={hidden() > 0}>
        <li class="more">and {hidden()} more</li>
      </Show>
    </ul>
  );
}

interface DiagnosticGroup {
  message: string;
  abouts: string[];
}

// Most diagnostics repeat one of a few hundred messages ("no adapter for
// recipe type X yet"). So we group by message and sort by group size.
function groupByMessage(diagnostics: Diagnostic[]): DiagnosticGroup[] {
  const groups = new Map<string, string[]>();
  for (const { about, message } of diagnostics) {
    const abouts = groups.get(message);
    if (abouts === undefined) {
      groups.set(message, [about]);
    } else {
      abouts.push(about);
    }
  }
  return [...groups]
    .map(([message, abouts]) => ({ message, abouts }))
    .sort(
      (a, b) =>
        b.abouts.length - a.abouts.length || a.message.localeCompare(b.message),
    );
}

export function ParseReportView(props: { report: ParseReport }) {
  const groups = createMemo(() => groupByMessage(props.report.diagnostics));
  const recipeTypes = createMemo(() =>
    Object.entries(props.report.recipe_type_counts).sort(
      ([a, aCount], [b, bCount]) => bCount - aCount || a.localeCompare(b),
    ),
  );

  return (
    <section class="parse-report">
      <h2>
        Diagnostics ({props.report.diagnostics.length}, {groups().length} kinds)
      </h2>
      <For each={groups()}>
        {(group) => (
          <LazyDetails
            summary={
              <>
                <span class="count">{group.abouts.length}</span> {group.message}
              </>
            }
          >
            <CappedList items={group.abouts} />
          </LazyDetails>
        )}
      </For>

      <h2>Recipe files</h2>
      <LazyDetails summary={<>{recipeTypes().length} recipe types</>}>
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
            {props.report.registered_but_unreferenced.length} registered ids no
            recipe or tag references
          </>
        }
      >
        <CappedList items={props.report.registered_but_unreferenced} />
      </LazyDetails>
    </section>
  );
}
