import { defineConfig } from '@playwright/test';
import { fileURLToPath } from 'node:url';

const port = Number(process.env.WEBSITE_TEST_PORT || 4173);
if (!Number.isInteger(port) || port < 1 || port > 65535) throw new Error('Invalid WEBSITE_TEST_PORT');

export default defineConfig({
  testDir: './tests',
  outputDir: '../test-results/website',
  fullyParallel: false,
  workers: 1,
  use: { baseURL: `http://127.0.0.1:${port}`, browserName: 'chromium', viewport: { width: 1440, height: 1000 } },
  webServer: { command: `node website/serve.mjs --port ${port}`, cwd: fileURLToPath(new URL('../', import.meta.url)), url: `http://127.0.0.1:${port}/lekton/`, reuseExistingServer: !process.env.CI },
});
