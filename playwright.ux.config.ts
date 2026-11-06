import { defineConfig } from '@playwright/test';

// Runs against an already-built isolated demo instance. Deliberately has no
// globalSetup or webServer: it must not seed a corpus or reuse port 3000.
export default defineConfig({
  testDir: './e2e',
  testMatch: ['portal-ux.spec.ts', 'theme.spec.ts'],
  workers: 1,
  reporter: 'list',
  outputDir: 'test-results/portal-ux',
  use: {
    baseURL: process.env.LEKTON_UX_BASE_URL || 'http://localhost:3100',
    screenshot: 'only-on-failure',
  },
});
