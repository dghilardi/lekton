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
    command: process.env.CI
      ? 'cargo leptos serve --release'
      : 'cargo leptos serve',
    url: 'http://localhost:3000',
    reuseExistingServer: !process.env.CI,
    timeout: 180_000,
    env: {
      LKN_AUTH__DEMO_MODE: 'true',
      LKN_AUTH__SERVICE_TOKEN: 'test-token',
      LKN_SERVER__RATE_LIMIT_BURST: '1000',
      LKN_DATABASE__URI: process.env.LKN_DATABASE__URI || 'mongodb://localhost:27017',
      LKN_DATABASE__NAME: process.env.LKN_DATABASE__NAME || 'lekton_e2e',
      LKN_STORAGE__BUCKET: process.env.LKN_STORAGE__BUCKET || 'lekton-e2e',
      LKN_STORAGE__ENDPOINT: process.env.LKN_STORAGE__ENDPOINT || 'http://localhost:9000',
      LKN_SEARCH__URL: process.env.LKN_SEARCH__URL || 'http://localhost:7700',
      LKN_SEARCH__API_KEY: process.env.LKN_SEARCH__API_KEY || '',
      AWS_ACCESS_KEY_ID: process.env.AWS_ACCESS_KEY_ID || 'minioadmin',
      AWS_SECRET_ACCESS_KEY: process.env.AWS_SECRET_ACCESS_KEY || 'minioadmin',
      AWS_REGION: process.env.AWS_REGION || 'us-east-1',
    },
  },
});
