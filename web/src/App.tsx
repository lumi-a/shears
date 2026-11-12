import { createSignal } from "solid-js";
import { SessionClient } from "./session";

const session = new SessionClient();

function App() {
  const [busy, setBusy] = createSignal(false);
  const [status, setStatus] = createSignal("");

  const onFileChange = async (event: Event) => {
    const input = event.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    if (!file) return;

    setBusy(true);
    setStatus("ingesting...");
    try {
      const info = await session.open(file, "");
      const root =
        info.rootDirectory === ""
          ? "the archive's top level"
          : info.rootDirectory;
      setStatus(
        `export found at ${root}: ${info.fileCount} files, ` +
          `${info.itemCount} items, ${info.recipeCount} recipes, ` +
          `${info.report.diagnostics.length} diagnostics`,
      );
    } catch (error) {
      setStatus(`error: ${String(error)}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <main>
      <h1>shears</h1>
      <input
        type="file"
        accept=".zip,.tar.zst,.zst"
        disabled={busy()}
        onChange={onFileChange}
      />
      <p>{status()}</p>
    </main>
  );
}

export default App;
