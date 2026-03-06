import { test, expect } from '@playwright/test';

test.describe('Theme', () => {
  test('toggle switches theme', async ({ page }) => {
    await page.goto('/');
    await page.waitForLoadState('networkidle');
    // Find the theme toggle button
    const themeToggle = page.locator('button[aria-label="Toggle theme"]');
    await expect(themeToggle).toBeVisible();

    // Click twice: system→light→dark (in headless, system resolves to light so we need 2 clicks)
    await themeToggle.click();
    await page.waitForTimeout(200);
    await themeToggle.click();
    await page.waitForTimeout(300);

    const newTheme = await page.locator('html').getAttribute('data-theme');
    expect(newTheme).toEqual('dark');
  });

  test('theme persists across page reload', async ({ page }) => {
    await page.goto('/');
    await page.waitForLoadState('networkidle');
    const themeToggle = page.locator('button[aria-label="Toggle theme"]');

    // Click twice to reach 'dark' (system→light→dark; headless resolves system=light)
    await themeToggle.click();
    await page.waitForTimeout(200);
    await themeToggle.click();
    await page.waitForTimeout(300);

    const themeAfterToggle = await page.locator('html').getAttribute('data-theme');

    // Reload the page
    await page.reload();
    await page.waitForLoadState('networkidle');

    const themeAfterReload = await page.locator('html').getAttribute('data-theme');
    expect(themeAfterReload).toEqual(themeAfterToggle);
  });
});
