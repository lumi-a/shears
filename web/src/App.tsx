import { createSignal, onMount } from "solid-js";
import init, { Session } from "./wasm-pkg/shears_wasm";

function App() {
  const [ready, setReady] = createSignal(false);
  const [status, setStatus] = createSignal("");

  onMount(async () => {
    await init();
    setReady(true);
  });

  const onFileChange = async (event: Event) => {
    const input = event.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    if (!file) return;

    setStatus("ingesting...");
    try {
      const bytes = new Uint8Array(await file.arrayBuffer());
      const session = Session.open(bytes, "");
      const root =
        session.root_directory === ""
          ? "the archive's top level"
          : session.root_directory;
      setStatus(
        `export found at ${root}: ${session.file_count} files, ` +
          `${session.item_count} items, ${session.recipe_count} recipes`,
      );
      session.free();
    } catch (error) {
      setStatus(`error: ${String(error)}`);
    }
  };

  return (
    <main>
      <h1>shears</h1>
      <p>{ready() ? "wasm ready" : "loading wasm..."}</p>
      <input
        type="file"
        accept=".zip,.tar.zst,.zst"
        disabled={!ready()}
        onChange={onFileChange}
      />
      <p>{status()}</p>
    </main>
  );
}

export default App;
