import { test, expect } from '@playwright/test';

test.describe('Document viewer', () => {
  test('renders markdown content', async ({ page }) => {
    await page.goto('/docs/getting-started');
    // Document content is loaded via Leptos Resource (requires WASM hydration)
    await page.waitForLoadState('networkidle');
    // Should render the markdown heading (inside article content)
    await expect(page.locator('article h1', { hasText: 'Getting Started' })).toBeVisible();
    // Should render paragraph content
    await expect(page.locator('text=Welcome to')).toBeVisible();
  });

  test('shows tags', async ({ page }) => {
    await page.goto('/docs/getting-started');
    await page.waitForLoadState('networkidle');
    // Tags should be displayed (seeded with 'guide' and 'onboarding')
    await expect(page.locator('text=guide')).toBeVisible();
  });

  test('shows last updated date', async ({ page }) => {
    await page.goto('/docs/getting-started');
    await page.waitForLoadState('networkidle');
    // The document metadata section should show a last updated date
    await expect(page.locator('text=/Last updated/')).toBeVisible();
  });

  test('clicking nav link navigates to document', async ({ page }) => {
    await page.goto('/');
    // Click on a document in the navigation
    await page.click('text=Architecture Overview');
    // Should navigate to the document page
    await expect(page).toHaveURL(/\/docs\/architecture-overview/);
    // Document content is loaded via Leptos Resource (requires WASM hydration)
    await page.waitForLoadState('networkidle');
    await expect(page.locator('article h1', { hasText: 'Architecture Overview' })).toBeVisible();
  });
});
