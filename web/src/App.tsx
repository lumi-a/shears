import { createSignal, For, onMount, Show } from "solid-js";
import { ParseReportView } from "./ParseReportView";
import { RulesPane } from "./RulesPane";
import {
  fetchPreset,
  PRESETS,
  type FetchedPreset,
  type Preset,
} from "./presets";
import { SessionClient, type SessionInfo } from "./session";
import { Pane, Split } from "./Split";

const session = new SessionClient();

function App() {
  // What's in progress, e.g. "ingesting...". Empty when idle.
  const [working, setWorking] = createSignal("");
  const busy = () => working() !== "";
  const [error, setError] = createSignal("");
  const [info, setInfo] = createSignal<SessionInfo>();
  // parsing.toml's contents as edited, and as last successfully ingested with.
  // Empty is valid: only vanilla recipe types then.
  const [shapesToml, setShapesToml] = createSignal("");
  const [appliedShapesToml, setAppliedShapesToml] = createSignal("");
  const shapesChanged = () =>
    info() !== undefined && shapesToml() !== appliedShapesToml();
  // rules.toml's contents, not used by analysis yet.
  const [rulesToml, setRulesToml] = createSignal("");
  let openExport!: HTMLDivElement;

  // Nothing to show until an export is open. We offer one instead.
  onMount(() => openExport.showPopover());

  // Runs one session request that ingests with `toml`, showing its result or
  // its error. If error, the previous result stays up.
  const run = async (
    toml: string,
    request: (toml: string) => Promise<SessionInfo>,
  ) => {
    setWorking("ingesting...");
    setError("");
    try {
      setInfo(await request(toml));
      setAppliedShapesToml(toml);
    } catch (caught) {
      setError(String(caught));
    } finally {
      setWorking("");
    }
  };

  const onExportChange = async (event: Event) => {
    const input = event.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    // Cleared so picking the same file again still counts as a change.
    input.value = "";
    if (!file) return;
    openExport.hidePopover();
    await run(shapesToml(), (toml) => session.open(file, toml));
  };

  const onApplyShapes = () =>
    run(shapesToml(), (toml) => session.reingest(toml));

  const onPreset = async (preset: Preset) => {
    openExport.hidePopover();
    setWorking(`downloading ${preset.name}...`);
    setError("");
    let fetched: FetchedPreset;
    try {
      fetched = await fetchPreset(preset);
    } catch (caught) {
      setError(String(caught));
      setWorking("");
      return;
    }
    // The preset's parsing.toml replaces whatever was picked before.
    const { archive, parsingToml } = fetched;
    setShapesToml(parsingToml);
    await run(parsingToml, (toml) => session.open(archive, toml));
  };

  const summary = (current: SessionInfo) =>
    `${current.rootDirectory === "" ? "export" : current.rootDirectory}: ` +
    `${current.fileCount} files, ${current.itemCount} items, ` +
    `${current.recipeCount} recipes`;

  const rulesPane = (
    <RulesPane
      rulesToml={rulesToml()}
      setRulesToml={setRulesToml}
      shapesToml={shapesToml()}
      setShapesToml={setShapesToml}
      shapesChanged={shapesChanged()}
      busy={busy()}
      onReingest={onApplyShapes}
    />
  );

  const reportPane = (
    <Show
      when={info()}
      fallback={
        <p class="placeholder">
          No export open yet.{" "}
          <button type="button" popovertarget="open-export">
            Open one
          </button>
        </p>
      }
    >
      {(current) => <ParseReportView report={current().report} />}
    </Show>
  );

  const explorerPane = (
    <p class="placeholder">Searching items and recipes WIP.</p>
  );

  return (
    <div class="app">
      <header class="app-header">
        <h1>
          <img
            class="logo"
            src={`${import.meta.env.BASE_URL}favicon.svg`}
            alt=""
          />
          shears
        </h1>
        <button type="button" popovertarget="open-export" disabled={busy()}>
          Open export...
        </button>
        <p class="status" classList={{ error: error() !== "" }} role="status">
          <Show when={!busy()} fallback={working()}>
            <Show when={error() === ""} fallback={`error: ${error()}`}>
              <Show when={info()}>{(current) => summary(current())}</Show>
            </Show>
          </Show>
        </p>
      </header>

      <div
        id="open-export"
        class="open-export"
        popover
        ref={(element) => (openExport = element)}
      >
        <h3>Open a preset</h3>
        <p class="presets">
          <For each={PRESETS}>
            {(preset) => (
              <button
                type="button"
                disabled={busy()}
                onClick={() => onPreset(preset)}
              >
                {preset.name}
              </button>
            )}
          </For>
        </p>
        <h3>Open your own export</h3>
        <p class="hint">
          These won't get sent to a server, they're processed locally in your
          browser.
        </p>
        <p>
          <label>
            <code>/kubejs export</code> as .zip or .tar.zst{" "}
            <input
              type="file"
              accept=".zip,.tar.zst,.zst"
              disabled={busy()}
              onChange={onExportChange}
            />
          </label>
        </p>
      </div>

      <Split
        direction="columns"
        storageKey="shears.split.main"
        initial={0.4}
        first={<Pane title="Rules">{rulesPane}</Pane>}
        second={
          <Split
            direction="rows"
            storageKey="shears.split.right"
            initial={0.5}
            first={<Pane title="Report">{reportPane}</Pane>}
            second={<Pane title="Explorer">{explorerPane}</Pane>}
          />
        }
      />
    </div>
  );
}

export default App;
