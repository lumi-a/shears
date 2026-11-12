import { defineConfig } from "vite";
import solid from "vite-plugin-solid";

export default defineConfig(({ command }) => ({
  // GitHub Pages serves this as a project site, under /shears/, not at the domain root.
  base: command === "serve" ? "/" : "/shears/",
  plugins: [solid()],
  // session.worker.ts is a module worker (it imports the wasm-bindgen glue).
  worker: { format: "es" },
}));
