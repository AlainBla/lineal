import { tanstackRouter } from "@tanstack/router-plugin/vite";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import path from "node:path";
import { defineConfig } from "vitest/config";

// Backend-Adresse; per LINEAL_BACKEND überschreibbar (z. B. wenn :8080 belegt ist).
const backend = process.env.LINEAL_BACKEND ?? "http://localhost:8080";

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
    proxy: {
      "/graphql": backend,
      "/auth": backend,
      "/mcp": backend,
      "/sync": { target: backend, ws: true },
    },
  },
  test: {
    environment: "jsdom",
    setupFiles: ["./src/test-setup.ts"],
    css: false,
  },
});
