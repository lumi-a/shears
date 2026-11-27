import { For, Show } from "solid-js";
import { CappedList, LazyDetails } from "./ParseReportView";
import type { Target } from "./Details";
import type { Analysis, AnalysisDiff } from "./session";

// What Analysis found or why rules.toml didn't parse.
export interface AnalysisResult {
  // The rules.toml this ran with.
  rulesToml: string;
  // Exactly one of these is set.
  analysis?: Analysis;
  error?: string;
}

const count = (value: number) => value.toLocaleString();

const percent = (part: number, whole: number) =>
  whole === 0 ? "" : ` (${((100 * part) / whole).toFixed(1)}%)`;

// What changed since the previous analysis, one expandable list per kind of change.
// Kinds that didn't change are left out.
function DiffView(props: {
  diff: AnalysisDiff;
  open: (target: Target) => void;
}) {
  const changes = () =>
    [
      {
        sign: "+",
        label: "items newly required",
        kind: "item" as const,
        ids: props.diff.items_added,
      },
      {
        sign: "−",
        label: "items no longer required",
        kind: "item" as const,
        ids: props.diff.items_removed,
      },
      {
        sign: "+",
        label: "recipes newly useful",
        kind: "recipe" as const,
        ids: props.diff.recipes_added,
      },
      {
        sign: "−",
        label: "recipes no longer useful",
        kind: "recipe" as const,
        ids: props.diff.recipes_removed,
      },
    ].filter((change) => change.ids.length > 0);

  return (
    <>
      <h4>Since the previous analysis</h4>
      <Show
        when={changes().length > 0}
        fallback={<p class="hint">No change.</p>}
      >
        <For each={changes()}>
          {(change) => (
            <LazyDetails
              summary={
                <>
                  <span class="count">
                    {change.sign}
                    {count(change.ids.length)}
                  </span>
                  <span class="label">{change.label}</span>
                </>
              }
            >
              <CappedList
                items={change.ids}
                links={{ kind: change.kind, open: props.open }}
              />
            </LazyDetails>
          )}
        </For>
      </Show>
    </>
  );
}

export function AnalysisView(props: {
  result: AnalysisResult | undefined;
  itemCount: number;
  recipeCount: number;
  // Opens an item or recipe in the Explorer.
  open: (target: Target) => void;
}) {
  return (
    <section class="analysis">
      <h3>Analysis</h3>
      <Show when={props.result === undefined}>
        <p class="hint">Write rules in rules.toml, then press Analyze.</p>
      </Show>
      <Show when={props.result?.error}>
        {(error) => <pre class="error">{error()}</pre>}
      </Show>
      <Show when={props.result?.analysis}>
        {(analysis) => (
          <>
            <ul>
              <li>
                {count(analysis().required_count)} items (directly) required
              </li>
              <li>
                {count(analysis().transitively_required_item_count)} of{" "}
                {count(props.itemCount)} items transitively required
                {percent(
                  analysis().transitively_required_item_count,
                  props.itemCount,
                )}
              </li>
              <li>
                {count(analysis().transitively_required_recipe_count)} of{" "}
                {count(props.recipeCount)} recipes useful
                {percent(
                  analysis().transitively_required_recipe_count,
                  props.recipeCount,
                )}
              </li>
            </ul>
            <Show when={analysis().warnings.length > 0}>
              <h4>Warnings</h4>
              <ul>
                <For each={analysis().warnings}>
                  {(warning) => <li>{warning}</li>}
                </For>
              </ul>
            </Show>
            <Show when={analysis().diff}>
              {(diff) => <DiffView diff={diff()} open={props.open} />}
            </Show>
          </>
        )}
      </Show>
    </section>
  );
}
