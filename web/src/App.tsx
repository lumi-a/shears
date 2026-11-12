import { createSignal, Show } from "solid-js";
import { ParseReportView } from "./ParseReportView";
import { SessionClient, type SessionInfo } from "./session";

const session = new SessionClient();

function App() {
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal("");
  const [info, setInfo] = createSignal<SessionInfo>();
  // parsing.toml's contents.
  const [shapesToml, setShapesToml] = createSignal("");
  const [shapesName, setShapesName] = createSignal("");

  // Runs one session request, showing its result or its error.
  // If error, the previous result stays up.
  const run = async (request: () => Promise<SessionInfo>) => {
    setBusy(true);
    setError("");
    try {
      setInfo(await request());
    } catch (caught) {
      setError(String(caught));
    } finally {
      setBusy(false);
    }
  };

  const onExportChange = async (event: Event) => {
    const file = (event.currentTarget as HTMLInputElement).files?.[0];
    if (!file) return;
    const toml = shapesToml();
    await run(() => session.open(file, toml));
  };

  const onShapesChange = async (event: Event) => {
    const file = (event.currentTarget as HTMLInputElement).files?.[0];
    if (!file) return;
    const toml = await file.text();
    setShapesToml(toml);
    setShapesName(file.name);
    if (info() !== undefined) await run(() => session.reingest(toml));
  };

  const shownRoot = (current: SessionInfo) =>
    current.rootDirectory === ""
      ? "the archive's top level"
      : current.rootDirectory;

  return (
    <main>
      <h1>shears</h1>
      <p>
        <label>
          export (.zip or .tar.zst){" "}
          <input
            type="file"
            accept=".zip,.tar.zst,.zst"
            disabled={busy()}
            onChange={onExportChange}
          />
        </label>
      </p>
      <p>
        <label>
          parsing.toml{" "}
          <input
            type="file"
            accept=".toml"
            disabled={busy()}
            onChange={onShapesChange}
          />
        </label>{" "}
        {shapesName() === ""
          ? "(none: vanilla recipe types only)"
          : `(using ${shapesName()})`}
      </p>
      <Show when={busy()}>
        <p>ingesting...</p>
      </Show>
      <Show when={error() !== ""}>
        <p class="error">error: {error()}</p>
      </Show>
      <Show when={info()}>
        {(current) => (
          <>
            <p>
              export found at {shownRoot(current())}: {current().fileCount}{" "}
              files, {current().itemCount} items, {current().recipeCount}{" "}
              recipes
            </p>
            <ParseReportView report={current().report} />
          </>
        )}
      </Show>
    </main>
  );
}

export default App;
