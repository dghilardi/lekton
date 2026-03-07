import { test, expect, type Page } from '@playwright/test';

/**
 * Open the search modal, retrying Ctrl+K until WASM has hydrated and
 * registered the keydown event listener. Typically 1–3 retries in CI.
 */
async function openSearchModal(page: Page): Promise<void> {
  const searchInput = page.locator('input[placeholder*="Search"]');
  for (let attempt = 0; attempt < 30; attempt++) {
    await page.keyboard.press('Control+k');
    if (await searchInput.isVisible({ timeout: 500 }).catch(() => false)) return;
    await page.waitForTimeout(500);
  }
  // Final assertion with a clear failure message if all retries failed
  await expect(searchInput).toBeVisible({ timeout: 2_000 });
}

test.describe('Search', () => {
  test('Ctrl+K opens search modal', async ({ page }) => {
    await page.goto('/');
    await openSearchModal(page);
    const searchInput = page.locator('input[placeholder*="Search"]');
    await expect(searchInput).toBeVisible();
    await expect(searchInput).toBeFocused();
  });

  test('search returns results', async ({ page }) => {
    await page.goto('/');
    await openSearchModal(page);
    const searchInput = page.locator('input[placeholder*="Search"]');
    await searchInput.fill('Getting Started');
    // Wait for results to appear
    await page.waitForTimeout(500);
    // Results should contain the seeded document
    await expect(page.locator('text=Getting Started').first()).toBeVisible();
  });

  test('clicking search result navigates to document', async ({ page }) => {
    await page.goto('/');
    await openSearchModal(page);
    const searchInput = page.locator('input[placeholder*="Search"]');
    await searchInput.click();
    await searchInput.fill('Architecture');
    // Result titles render as <div class="font-semibold text-lg mb-1"> inside the modal.
    // This class combination only appears in search result items, not the sidebar nav.
    const resultLink = page.locator('.fixed.inset-0 a').filter({ has: page.locator('div.font-semibold') });
    await expect(resultLink.first()).toBeVisible({ timeout: 15_000 });
    await resultLink.first().click();
    await expect(page).toHaveURL(/\/docs\//);
  });

  test('Escape closes search modal', async ({ page }) => {
    await page.goto('/');
    await openSearchModal(page);
    const searchInput = page.locator('input[placeholder*="Search"]');
    await expect(searchInput).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(searchInput).not.toBeVisible();
  });
});
