import { test, expect } from '@playwright/test';

test.describe('Home page', () => {
  test('loads and shows document list in navigation', async ({ page }) => {
    await page.goto('/');
    // Navigation sidebar should contain seeded documents
    await expect(page.locator('text=Getting Started')).toBeVisible();
    await expect(page.locator('text=Architecture Overview')).toBeVisible();
  });

  test('search button with Ctrl+K hint is visible', async ({ page }) => {
    await page.goto('/');
    // The search trigger button should be visible in the navbar
    const searchButton = page.locator('button', { hasText: /search|ctrl.*k/i });
    await expect(searchButton.first()).toBeVisible();
  });

  test('anonymous user sees only public documents', async ({ page }) => {
    await page.goto('/');
    // Public docs should be visible
    await expect(page.locator('text=Getting Started')).toBeVisible();
    // Internal docs should NOT be visible (requires auth)
    await expect(page.locator('text=Internal Processes')).not.toBeVisible();
  });
});
