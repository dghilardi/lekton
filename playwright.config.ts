import { defineConfig, devices } from '@playwright/test';

export default defineConfig({
  testDir: './e2e',
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  workers: process.env.CI ? 1 : undefined,
  reporter: process.env.CI ? [['github'], ['html']] : 'html',

  use: {
    baseURL: 'http://localhost:3000',
    trace: 'on-first-retry',
    screenshot: 'only-on-failure',
  },

  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] },
    },
    // Enable these for local multi-browser testing:
    // {
    //   name: 'firefox',
    //   use: { ...devices['Desktop Firefox'] },
    // },
    // {
    //   name: 'webkit',
    //   use: { ...devices['Desktop Safari'] },
    // },
  ],

  globalSetup: './e2e/global-setup.ts',

  webServer: {
    // In CI the binary is already compiled by `cargo leptos build --release`,
    // so run it directly to avoid a redundant recompilation that exceeds the
    // Playwright timeout.
    command: process.env.CI
      ? './target/release/lekton'
      : 'cargo leptos serve',
    url: 'http://localhost:3000',
    reuseExistingServer: !process.env.CI,
    stdout: 'pipe',
    stderr: 'pipe',
    timeout: process.env.CI ? 30_000 : 180_000,
    env: {
      LKN__AUTH__DEMO_MODE: 'true',
      LKN__AUTH__SERVICE_TOKEN: 'test-token',
      LKN__SERVER__RATE_LIMIT_BURST: '1000',
      LKN__DATABASE__URI: process.env.LKN__DATABASE__URI || 'mongodb://localhost:27017',
      LKN__DATABASE__NAME: process.env.LKN__DATABASE__NAME || 'lekton_e2e',
      LKN__STORAGE__BUCKET: process.env.LKN__STORAGE__BUCKET || 'lekton-e2e',
      LKN__STORAGE__ENDPOINT: process.env.LKN__STORAGE__ENDPOINT || 'http://localhost:9000',
      LKN__SEARCH__URL: process.env.LKN__SEARCH__URL || 'http://localhost:7700',
      LKN__SEARCH__API_KEY: process.env.LKN__SEARCH__API_KEY || '',
      AWS_ACCESS_KEY_ID: process.env.AWS_ACCESS_KEY_ID || 'minioadmin',
      AWS_SECRET_ACCESS_KEY: process.env.AWS_SECRET_ACCESS_KEY || 'minioadmin',
      AWS_REGION: process.env.AWS_REGION || 'us-east-1',
      // Leptos configuration for the pre-built binary (reads from env when
      // not launched via cargo-leptos).
      ...(process.env.CI ? {
        LEPTOS_OUTPUT_NAME: 'lekton',
        LEPTOS_SITE_ROOT: 'target/site',
        LEPTOS_SITE_ADDR: '127.0.0.1:3000',
        LEPTOS_SITE_PKG_DIR: 'pkg',
      } : {}),
    },
  },
});
