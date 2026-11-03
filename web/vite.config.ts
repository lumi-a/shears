import { defineConfig } from "vite";
import solid from "vite-plugin-solid";

export default defineConfig(({ command }) => ({
  // GitHub Pages serves this as a project site, under /shears/, not at the domain root.
  base: command === "serve" ? "/" : "/shears/",
  plugins: [solid()],
}));
