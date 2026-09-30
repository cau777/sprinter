import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import { defineConfig, devices } from "@playwright/test";

const dataDir = mkdtempSync(resolve(tmpdir(), "sprinter-e2e-"));
const appPort = process.env.E2E_APP_PORT ?? "8080";
const appBaseUrl = `http://127.0.0.1:${appPort}`;
process.env.SPRINTER_E2E_DATA_DIR = dataDir;
process.env.SPRINTER_E2E_BASE_URL = appBaseUrl;

export default defineConfig({
  testDir: ".",
  testMatch: "**/*.spec.ts",
  fullyParallel: true,
  forbidOnly: Boolean(process.env.CI),
  retries: 0,
  workers: process.env.CI ? 2 : undefined,
  reporter: "list",
  timeout: 30_000,
  expect: { timeout: 5_000 },
  globalSetup: "./global-setup.ts",
  globalTeardown: "./global-teardown.ts",
  use: {
    baseURL: appBaseUrl,
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
    video: "retain-on-failure",
    storageState: ".auth/user.json",
  },
  projects: [
    { name: "desktop", use: { ...devices["Desktop Chrome"] } },
    { name: "mobile", grep: /@mobile/, use: { ...devices["Pixel 7"] } },
  ],
  webServer: [
    {
      command: "cargo run --quiet -p fake-openrouter --bin fake-openrouter",
      cwd: "..",
      url: "http://127.0.0.1:4010/api/v1/models",
      reuseExistingServer: false,
      timeout: 180_000,
      env: { FAKE_OPENROUTER_ADDR: "127.0.0.1:4010", RUST_LOG: "warn" },
    },
    {
      command: "cargo run --quiet -p sprinter -- serve",
      cwd: "..",
      url: `${appBaseUrl}/healthz`,
      reuseExistingServer: false,
      timeout: 180_000,
      env: {
        DATA_DIR: dataDir,
        PORT: appPort,
        BIND: "127.0.0.1",
        SPRINTER_PASSWORD: "test",
        SPRINTER_INSECURE_COOKIES: "true",
        OPENROUTER_BASE_URL: "http://127.0.0.1:4010/api/v1",
        RUST_LOG: "warn",
      },
    },
  ],
});
