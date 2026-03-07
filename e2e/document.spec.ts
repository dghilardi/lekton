import { test, expect } from '@playwright/test';

test.describe('Document viewer', () => {
  test('renders markdown content', async ({ page }) => {
    await page.goto('/docs/getting-started');
    // Document content is streamed via Leptos SSR (Resource + Suspense).
    // Wait directly for the article element rather than networkidle, which can
    // fire before the SSR stream finishes in slow CI environments.
    await expect(page.locator('article h1', { hasText: 'Getting Started' })).toBeVisible({ timeout: 15_000 });
    await expect(page.locator('text=Welcome to')).toBeVisible();
  });

  test('shows tags', async ({ page }) => {
    await page.goto('/docs/getting-started');
    // Tags are part of the SSR-streamed document metadata
    await expect(page.locator('text=guide')).toBeVisible({ timeout: 15_000 });
  });

  test('shows last updated date', async ({ page }) => {
    await page.goto('/docs/getting-started');
    // The document metadata section should show a last updated date
    await expect(page.locator('text=/Last updated/')).toBeVisible({ timeout: 15_000 });
  });

  test('clicking nav link navigates to document', async ({ page }) => {
    await page.goto('/');
    await page.waitForLoadState('networkidle');
    // Click on a document in the navigation
    await page.click('text=Architecture Overview');
    // Should navigate to the document page
    await expect(page).toHaveURL(/\/docs\/architecture-overview/);
    await expect(page.locator('article h1', { hasText: 'Architecture Overview' })).toBeVisible({ timeout: 15_000 });
  });
});
