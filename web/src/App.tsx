import { createSignal, onMount } from "solid-js";
import init, { unpack_archive } from "./wasm-pkg/shears_wasm";

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

    setStatus("unpacking...");
    try {
      const bytes = new Uint8Array(await file.arrayBuffer());
      const summary = unpack_archive(bytes);
      const root =
        summary.root_directory === ""
          ? "the archive's top level"
          : summary.root_directory;
      setStatus(
        `export found at ${root}: ${summary.file_count} files, ${summary.total_bytes} bytes total`,
      );
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
