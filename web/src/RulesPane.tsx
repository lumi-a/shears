import { createSignal, For, Show } from "solid-js";
import { TomlEditor } from "./TomlEditor";

type Tab = "rules.toml" | "parsing.toml";
const TABS: Tab[] = ["rules.toml", "parsing.toml"];

const RULES_PLACEHOLDER = `# Which items are needed. For example:
#
# [[item-rule]]
# reason = "the pack's goal"
# filter = { id = "gtceu:creative_chest" }
# action = { required = true }`;

// Saves `text` as a file called `name` through the browser's download.
function download(name: string, text: string) {
  const url = URL.createObjectURL(
    new Blob([text], { type: "application/toml" }),
  );
  // What?
  const link = document.createElement("a");
  link.href = url;
  link.download = name;
  link.click();
  setTimeout(() => URL.revokeObjectURL(url), 0);
}

// The editors for rules.toml and parsing.toml.
export function RulesPane(props: {
  rulesToml: string;
  setRulesToml: (value: string) => void;
  shapesToml: string;
  setShapesToml: (value: string) => void;
  // parsing.toml differs from what the current report was ingested with.
  shapesChanged: boolean;
  // rules.toml differs from what the current analysis ran with.
  rulesChanged: boolean;
  busy: boolean;
  // Whether there's an export to analyze.
  canAnalyze: boolean;
  onReingest: () => void;
  onAnalyze: () => void;
}) {
  const [tab, setTab] = createSignal<Tab>("rules.toml");
  const textOf = (name: Tab) =>
    name === "rules.toml" ? props.rulesToml : props.shapesToml;

  return (
    <div class="rules-pane">
      <div class="tabs" role="tablist" aria-label="Files">
        <For each={TABS}>
          {(name) => (
            <button
              type="button"
              role="tab"
              id={`tab-${name}`}
              aria-selected={tab() === name}
              aria-controls={`panel-${name}`}
              onClick={() => setTab(name)}
            >
              {name}
            </button>
          )}
        </For>
        <button
          type="button"
          class="download"
          onClick={() => download(tab(), textOf(tab()))}
        >
          Download
        </button>
      </div>

      <div class="tab-panels">
        <div
          class="tab-panel"
          role="tabpanel"
          id="panel-rules.toml"
          aria-labelledby="tab-rules.toml"
          hidden={tab() !== "rules.toml"}
        >
          <TomlEditor
            label="rules.toml"
            placeholder={RULES_PLACEHOLDER}
            value={props.rulesToml}
            onChange={(value) => props.setRulesToml(value)}
          />
        </div>
        <div
          class="tab-panel"
          role="tabpanel"
          id="panel-parsing.toml"
          aria-labelledby="tab-parsing.toml"
          hidden={tab() !== "parsing.toml"}
        >
          <TomlEditor
            label="parsing.toml"
            placeholder="# How to parse recipe types that aren't vanilla. Empty: vanilla recipe types only."
            value={props.shapesToml}
            onChange={(value) => props.setShapesToml(value)}
          />
        </div>
      </div>

      <div class="rules-actions">
        <Show
          when={tab() === "parsing.toml"}
          fallback={
            <>
              <button
                type="button"
                disabled={props.busy || !props.canAnalyze}
                onClick={() => props.onAnalyze()}
              >
                Analyze
              </button>
              <Show when={props.rulesChanged}>
                <span class="hint">changed since the last analysis</span>
              </Show>
            </>
          }
        >
          <button
            type="button"
            disabled={props.busy || !props.shapesChanged}
            onClick={() => props.onReingest()}
          >
            Re-ingest
          </button>
          <Show when={props.shapesChanged}>
            <span class="hint">changed since the last ingest</span>
          </Show>
        </Show>
      </div>
    </div>
  );
}
