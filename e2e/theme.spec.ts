import { test, expect, type Page } from '@playwright/test';

/**
 * Click the theme toggle and wait until `data-theme` actually changes,
 * retrying to handle WASM hydration delay in CI.
 * Returns the new data-theme value.
 */
async function clickThemeToggleUntilChange(page: Page): Promise<string> {
  const themeToggle = page.locator('button[aria-label="Toggle theme"]');
  const before = await page.locator('html').getAttribute('data-theme');

  for (let attempt = 0; attempt < 30; attempt++) {
    await themeToggle.click();
    await page.waitForTimeout(300);
    const after = await page.locator('html').getAttribute('data-theme');
    if (after !== before) return after!;
  }
  // Return current value even if unchanged (test will fail with a clear message)
  return (await page.locator('html').getAttribute('data-theme'))!;
}

test.describe('Theme', () => {
  test('toggle switches theme', async ({ page }) => {
    await page.goto('/');
    await page.waitForLoadState('networkidle');
    const themeToggle = page.locator('button[aria-label="Toggle theme"]');
    await expect(themeToggle).toBeVisible();

    // First click: wait for WASM hydration + theme change (system→light or light→dark)
    const afterFirst = await clickThemeToggleUntilChange(page);

    // Second click should cycle to the next theme
    await themeToggle.click();
    await page.waitForTimeout(300);
    const afterSecond = await page.locator('html').getAttribute('data-theme');

    // After two successful clicks from system default, we should cycle through
    // system→light→dark. The first detected change confirms WASM works.
    expect(afterFirst).not.toEqual(afterSecond);
  });

  test('theme persists across page reload', async ({ page }) => {
    await page.goto('/');
    await page.waitForLoadState('networkidle');

    // Click until theme changes (ensures WASM hydrated)
    const themeAfterToggle = await clickThemeToggleUntilChange(page);

    // Reload the page
    await page.reload();
    await page.waitForLoadState('networkidle');

    const themeAfterReload = await page.locator('html').getAttribute('data-theme');
    expect(themeAfterReload).toEqual(themeAfterToggle);
  });
});
