import { createSignal, For, onMount, Show } from "solid-js";
import { ParseReportView } from "./ParseReportView";
import {
  fetchPreset,
  PRESETS,
  type FetchedPreset,
  type Preset,
} from "./presets";
import { SessionClient, type SessionInfo } from "./session";
import { ThreePanes } from "./ThreePanes";

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
    <>
      <h3>
        <label for="parsing-toml">parsing.toml</label>
      </h3>
      <p class="hint">
        How to read recipe types beyond vanilla's. Paste yours here; empty means
        vanilla recipe types only.
      </p>
      <textarea
        id="parsing-toml"
        class="toml-input"
        spellcheck={false}
        value={shapesToml()}
        onInput={(event) => setShapesToml(event.currentTarget.value)}
      />
      <p>
        <button
          type="button"
          disabled={busy() || !shapesChanged()}
          onClick={onApplyShapes}
        >
          Re-ingest
        </button>{" "}
        <Show when={shapesChanged()}>
          <span class="hint">changed since the last ingest</span>
        </Show>
      </p>
      <p class="placeholder">The rules.toml editor WIP.</p>
    </>
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
        <h1>shears</h1>
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
        <h2>Open an Export</h2>
        <p class="hint">Everything's processed locally, nothing's uploaded.</p>
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

      <ThreePanes
        panes={[
          { title: "Rules", body: rulesPane },
          { title: "Report", body: reportPane },
          { title: "Explorer", body: explorerPane },
        ]}
      />
    </div>
  );
}

export default App;
