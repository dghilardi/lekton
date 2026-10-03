import { defineConfig } from '@playwright/test';
import { fileURLToPath } from 'node:url';

export default defineConfig({
  testDir: './tests',
  outputDir: '../test-results/website',
  fullyParallel: false,
  workers: 1,
  use: { baseURL: 'http://127.0.0.1:4173', browserName: 'chromium', viewport: { width: 1440, height: 1000 } },
  webServer: { command: 'node website/serve.mjs', cwd: fileURLToPath(new URL('../', import.meta.url)), url: 'http://127.0.0.1:4173/lekton/', reuseExistingServer: !process.env.CI },
});
