import { For, Show } from "solid-js";
import type { Analysis } from "./session";

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

export function AnalysisView(props: {
  result: AnalysisResult | undefined;
  itemCount: number;
  recipeCount: number;
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
          </>
        )}
      </Show>
    </section>
  );
}
