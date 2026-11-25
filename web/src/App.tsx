import { createSignal, For, onMount, Show } from "solid-js";
import { AnalysisView, type AnalysisResult } from "./AnalysisView";
import type { Target } from "./Details";
import { Explorer } from "./Explorer";
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
  // rules.toml's contents.
  const [rulesToml, setRulesToml] = createSignal("");
  // The latest Analyze. undefined until the first one.
  const [analysis, setAnalysis] = createSignal<AnalysisResult>();
  const rulesChanged = () => {
    const latest = analysis();
    return latest !== undefined && rulesToml() !== latest.rulesToml;
  };
  let openExport!: HTMLDivElement;

  // Nothing to show until an export is open. We offer one instead.
  onMount(() => openExport.showPopover());

  // Analyzes with `toml`. A rules.toml that doesn't parse shows up in the
  // Report pane.
  const analyze = async (toml: string) => {
    setWorking("analyzing...");
    try {
      setAnalysis({ rulesToml: toml, analysis: await session.analyze(toml) });
    } catch (caught) {
      const message = caught instanceof Error ? caught.message : String(caught);
      setAnalysis({ rulesToml: toml, error: message });
    } finally {
      setWorking("");
    }
  };

  // Runs one session request that ingests with `toml`, showing its result or
  // its error. If error, the previous result stays up.
  const run = async (
    toml: string,
    request: (toml: string) => Promise<SessionInfo>,
  ) => {
    setWorking("ingesting...");
    setError("");
    let ingested = false;
    try {
      setInfo(await request(toml));
      setAppliedShapesToml(toml);
      ingested = true;
    } catch (caught) {
      setError(String(caught));
    } finally {
      setWorking("");
    }
    // A new graph makes an earlier analysis stale, so redo it.
    if (ingested && analysis() !== undefined) await analyze(rulesToml());
  };

  const onAnalyze = () => analyze(rulesToml());

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
      rulesChanged={rulesChanged()}
      busy={busy()}
      canAnalyze={info() !== undefined}
      onReingest={onApplyShapes}
      onAnalyze={onAnalyze}
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
      {(current) => (
        <div class="report">
          <AnalysisView
            result={analysis()}
            itemCount={current().itemCount}
            recipeCount={current().recipeCount}
            open={openInExplorer}
          />
          <ParseReportView report={current().report} open={openInExplorer} />
        </div>
      )}
    </Show>
  );

  // What the Report pane asked the Explorer to open.
  const [explorerRequest, setExplorerRequest] = createSignal<{
    target: Target;
  }>();
  const openInExplorer = (target: Target) => setExplorerRequest({ target });

  const explorerPane = (
    <Show
      when={info() !== undefined}
      fallback={<p class="placeholder">No export open yet.</p>}
    >
      <Explorer
        session={session}
        revision={[info(), analysis()]}
        request={explorerRequest()}
      />
    </Show>
  );

  // TODO: Would probably be useful to have a "what is this?" or "?" button popup for more
  // information about the site. Perhaps it should just mirror README.md, idk. Probably
  // needs more "How, specifically, to use this site". Would then also include a link
  // to the github.
  // Do we need this header at all, actually? And while we're at it:
  // TODO: Do the panes need headers? they seem kinda silly and take up space best used
  // otherwise. Can they be replaced with something smaller, or perhaps dropped entirely?
  return (
    <div class="app">
      <header class="app-header">
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
