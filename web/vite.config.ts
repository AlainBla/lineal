import { tanstackRouter } from "@tanstack/router-plugin/vite";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import path from "node:path";
import { configDefaults, defineConfig } from "vitest/config";

// Backend-Adresse; per LINEAL_BACKEND überschreibbar (z. B. wenn :8080 belegt ist).
const backend = process.env.LINEAL_BACKEND ?? "http://localhost:8080";

const proxy = {
  "/graphql": backend,
  "/auth": backend,
  // Nur der REST-Endpunkt; die Seite `/invite/<token>` liefert die SPA.
  "/invite/accept": backend,
  "/mcp": backend,
  "/sync": { target: backend, ws: true },
};

export default defineConfig({
  plugins: [
    // Muss vor dem React-Plugin stehen; erzeugt src/routeTree.gen.ts (nicht eingecheckt).
    tanstackRouter({ target: "react", autoCodeSplitting: true }),
    react(),
    tailwindcss(),
  ],
  resolve: {
    alias: { "@": path.resolve(import.meta.dirname, "src") },
  },
  server: {
    port: 5173,
    proxy,
  },
  // `vite preview` (E2E-Tests) nutzt denselben Proxy auf das Backend.
  preview: { proxy },
  test: {
    environment: "jsdom",
    setupFiles: ["./src/test-setup.ts"],
    css: false,
    // Playwright-Tests (`e2e/*.e2e.ts`) laufen über `npm run e2e`.
    exclude: [...configDefaults.exclude, "e2e/**"],
  },
});
