import { createSignal, onMount } from "solid-js";
import init, { parses_empty_ruleset } from "./wasm-pkg/shears_wasm";

function App() {
  const [status, setStatus] = createSignal("loading wasm...");

  onMount(async () => {
    await init();
    setStatus(
      `shears-wasm says: parses_empty_ruleset() = ${parses_empty_ruleset()}`,
    );
  });

  return (
    <main>
      <h1>shears</h1>
      <p>{status()}</p>
    </main>
  );
}

export default App;
