import { test, expect } from '@playwright/test';

test.describe('Theme', () => {
  test('toggle switches theme', async ({ page }) => {
    await page.goto('/');
    // Find the theme toggle button
    const themeToggle = page.locator('button[aria-label="Toggle theme"]');
    await expect(themeToggle).toBeVisible();

    // Get current theme
    const initialTheme = await page.locator('html').getAttribute('data-theme');

    // Click toggle to change theme
    await themeToggle.click();
    await page.waitForTimeout(300);

    const newTheme = await page.locator('html').getAttribute('data-theme');
    expect(newTheme).not.toEqual(initialTheme);
  });

  test('theme persists across page reload', async ({ page }) => {
    await page.goto('/');
    const themeToggle = page.locator('button[aria-label="Toggle theme"]');

    // Click to set a specific theme
    await themeToggle.click();
    await page.waitForTimeout(300);

    const themeAfterToggle = await page.locator('html').getAttribute('data-theme');

    // Reload the page
    await page.reload();
    await page.waitForTimeout(500);

    const themeAfterReload = await page.locator('html').getAttribute('data-theme');
    expect(themeAfterReload).toEqual(themeAfterToggle);
  });
});
