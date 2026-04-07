import { test, expect } from '@playwright/test';

test.describe('Home page', () => {
  test('loads and shows document list in navigation', async ({ page }) => {
    await page.goto('/');
    await page.waitForLoadState('networkidle');
    // Top-level documents should be visible in the navbar (SSR-streamed)
    // .first() because the navbar renders 3 responsive tiers in the DOM simultaneously
    await expect(page.locator('text=Getting Started').first()).toBeVisible({ timeout: 15_000 });
    await expect(page.locator('text=Architecture Overview').first()).toBeVisible();
  });

  test('search button with Ctrl+K hint is visible', async ({ page }) => {
    await page.goto('/');
    await page.waitForLoadState('networkidle');
    // The search trigger button should be visible in the navbar
    const searchButton = page.locator('button', { hasText: /search|ctrl.*k/i });
    await expect(searchButton.first()).toBeVisible();
  });

  test('anonymous user sees only public documents', async ({ page }) => {
    await page.goto('/');
    await page.waitForLoadState('networkidle');
    // Public docs should be visible
    // .first() because the navbar renders 3 responsive tiers in the DOM simultaneously
    await expect(page.locator('text=Getting Started').first()).toBeVisible();
    // Internal docs should NOT be visible (requires auth)
    await expect(page.locator('text=Internal Processes')).not.toBeVisible();
  });
});
