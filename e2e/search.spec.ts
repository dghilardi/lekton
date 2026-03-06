import { test, expect } from '@playwright/test';

test.describe('Search', () => {
  test('Ctrl+K opens search modal', async ({ page }) => {
    await page.goto('/');
    await page.keyboard.press('Control+k');
    // Search modal should appear with input
    const searchInput = page.locator('input[placeholder*="Search"]');
    await expect(searchInput).toBeVisible();
    await expect(searchInput).toBeFocused();
  });

  test('search returns results', async ({ page }) => {
    await page.goto('/');
    await page.keyboard.press('Control+k');
    const searchInput = page.locator('input[placeholder*="Search"]');
    await searchInput.fill('Getting Started');
    // Wait for results to appear
    await page.waitForTimeout(500);
    // Results should contain the seeded document
    await expect(page.locator('text=Getting Started').first()).toBeVisible();
  });

  test('clicking search result navigates to document', async ({ page }) => {
    await page.goto('/');
    await page.keyboard.press('Control+k');
    const searchInput = page.locator('input[placeholder*="Search"]');
    await searchInput.fill('Architecture');
    await page.waitForTimeout(500);
    // Click the result link
    const result = page.locator('a[href*="/docs/architecture"]');
    await result.first().click();
    await expect(page).toHaveURL(/\/docs\/architecture/);
  });

  test('Escape closes search modal', async ({ page }) => {
    await page.goto('/');
    await page.keyboard.press('Control+k');
    const searchInput = page.locator('input[placeholder*="Search"]');
    await expect(searchInput).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(searchInput).not.toBeVisible();
  });
});
