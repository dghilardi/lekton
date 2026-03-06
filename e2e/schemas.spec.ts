import { test, expect } from '@playwright/test';

test.describe('Schema Registry', () => {
  test('schema list page shows schemas', async ({ page }) => {
    await page.goto('/schemas');
    await expect(page.locator('h1', { hasText: 'Schema Registry' })).toBeVisible();
    // Should show the seeded schema
    await expect(page.locator('text=user-api')).toBeVisible();
  });

  test('clicking schema shows versions', async ({ page }) => {
    await page.goto('/schemas');
    await page.click('text=user-api');
    await expect(page).toHaveURL(/\/schemas\/user-api/);
    // Should show version information
    await expect(page.locator('text=1.0.0')).toBeVisible();
    await expect(page.locator('text=1.1.0')).toBeVisible();
  });

  test('selecting version shows schema content', async ({ page }) => {
    await page.goto('/schemas/user-api');
    // Version selector should be present
    const versionSelect = page.locator('select');
    if (await versionSelect.isVisible()) {
      await versionSelect.selectOption({ label: /1\.0\.0/ });
      await page.waitForTimeout(500);
    }
    // Should display schema content (OpenAPI spec)
    await expect(page.locator('text=User API').first()).toBeVisible();
  });
});
