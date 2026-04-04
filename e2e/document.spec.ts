import { test, expect } from '@playwright/test';

test.describe('Document viewer', () => {
  test('renders markdown content', async ({ page }) => {
    await page.goto('/docs/getting-started');
    // Document content is streamed via Leptos SSR (Resource + Suspense).
    // Wait directly for the article element rather than networkidle, which can
    // fire before the SSR stream finishes in slow CI environments.
    await expect(page.locator('article h1', { hasText: 'Getting Started' })).toBeVisible({ timeout: 30_000 });
    await expect(page.locator('text=Welcome to')).toBeVisible();
  });

  test('shows tags', async ({ page }) => {
    await page.goto('/docs/getting-started');
    // Tags are part of the SSR-streamed document metadata
    await expect(page.locator('text=guide')).toBeVisible({ timeout: 30_000 });
  });

  test('shows last updated date', async ({ page }) => {
    await page.goto('/docs/getting-started');
    // The document metadata section should show a last updated date
    await expect(page.locator('text=/Last updated/')).toBeVisible({ timeout: 30_000 });
  });

  test('navigating to document via URL shows content', async ({ page }) => {
    // Navigate directly to a document page (avoids reliance on WASM-rendered navbar links)
    await page.goto('/docs/architecture-overview');
    await expect(page.locator('article h1', { hasText: 'Architecture Overview' })).toBeVisible({ timeout: 30_000 });
  });
});
