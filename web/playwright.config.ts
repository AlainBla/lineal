import { defineConfig, devices } from "@playwright/test";

/**
 * End-to-End-Tests gegen einen echten Server (`scripts/e2e.sh`).
 *
 * Der Aufrufer (Skript) legt die Datenbank `lineal_e2e` neu an, migriert und seedet
 * (`lineal seed --demo`); hier werden nur `lineal serve`, `lineal worker` und `vite preview`
 * (mit Proxy auf den Server) gestartet. Ports kommen aus der Umgebung, damit belegte Ports
 * (8080, 5432 auf dieser Maschine) keine Rolle spielen.
 */
const apiPort = process.env.LINEAL_E2E_API_PORT ?? "18741";
const webPort = process.env.LINEAL_E2E_WEB_PORT ?? "4173";
const databaseUrl =
  process.env.LINEAL_E2E_DATABASE_URL ?? "postgres://lineal:lineal@127.0.0.1:5440/lineal_e2e";
const bin = process.env.LINEAL_BIN ?? "../target/debug/lineal";
// Mock-Linear (erfundene Daten, nur lokal): der Worker liest den Endpunkt aus der Umgebung.
const mockPort = process.env.LINEAL_E2E_MOCK_PORT ?? "18950";
const mockBin = process.env.LINEAL_E2E_MOCK_BIN ?? "../target/debug/examples/mock_linear";
const webOrigin = `http://localhost:${webPort}`;
const backend = `http://127.0.0.1:${apiPort}`;

const serverEnv = {
  DATABASE_URL: databaseUrl,
  LINEAL_DEV: "1",
  LINEAL_BASE_URL: webOrigin,
  LINEAL_ALLOWED_ORIGINS: webOrigin,
  LINEAL_BLOB_DIR: process.env.LINEAL_E2E_BLOB_DIR ?? "/tmp/lineal-e2e-blobs",
  RUST_LOG: "warn",
};

export default defineConfig({
  testDir: "./e2e",
  testMatch: "**/*.e2e.ts",
  // Die Flows teilen sich eine Datenbank: seriell und ohne Wiederholung.
  fullyParallel: false,
  workers: 1,
  retries: 0,
  timeout: 45_000,
  expect: { timeout: 10_000 },
  reporter: [["list"]],
  use: {
    baseURL: webOrigin,
    trace: "retain-on-failure",
    // Auf dieser Maschine verbietet AppArmor den User-Namespace-Sandbox von Chromium.
    launchOptions: { chromiumSandbox: false },
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
  webServer: [
    {
      command: `${bin} serve`,
      url: `${backend}/healthz`,
      env: { ...serverEnv, LINEAL_BIND: `127.0.0.1:${apiPort}` },
      reuseExistingServer: false,
      timeout: 60_000,
    },
    {
      command: `${bin} worker`,
      // Der Worker hat keinen HTTP-Port; Bereitschaft = Startmeldung im Log.
      wait: { stdout: /worker started/ },
      env: {
        ...serverEnv,
        RUST_LOG: "info",
        LINEAL_IMPORT_LINEAR_ENDPOINT: `http://127.0.0.1:${mockPort}/graphql`,
      },
      reuseExistingServer: false,
      timeout: 60_000,
    },
    {
      command: mockBin,
      wait: { stdout: /mock linear listening/ },
      env: { MOCK_LINEAR_PORT: mockPort },
      reuseExistingServer: false,
      timeout: 60_000,
    },
    {
      command: `npx vite preview --host localhost --port ${webPort} --strictPort`,
      url: webOrigin,
      env: { LINEAL_BACKEND: backend },
      reuseExistingServer: false,
      timeout: 60_000,
    },
  ],
});
