import { test, expect } from '@playwright/test';

test.describe('Schema Registry', () => {
  test('schema list page shows schemas', async ({ page }) => {
    await page.goto('/schemas');
    await page.waitForLoadState('networkidle');
    await expect(page.locator('h1', { hasText: 'Schema Registry' })).toBeVisible();
    // Should show the seeded schema
    await expect(page.locator('text=user-api')).toBeVisible();
  });

  test('clicking schema shows versions', async ({ page }) => {
    await page.goto('/schemas');
    await page.waitForLoadState('networkidle');
    await page.click('text=user-api');
    await expect(page).toHaveURL(/\/schemas\/user-api/);
    // Should show version information (visible elements, not hidden select options)
    await expect(page.locator('text=v1.0.0').first()).toBeVisible({ timeout: 10_000 });
    await expect(page.locator('text=v1.1.0').first()).toBeVisible();
  });

  test('selecting version shows schema content', async ({ page }) => {
    await page.goto('/schemas/user-api');
    // Version selector should be present (wait up to 10s for WASM to render it)
    const versionSelect = page.locator('select');
    if (await versionSelect.isVisible({ timeout: 10_000 })) {
      await versionSelect.selectOption({ label: '1.0.0 (stable)' });
      await page.waitForTimeout(500);
    }
    // Should display schema content (OpenAPI spec)
    await expect(page.locator('text=User API').first()).toBeVisible();
  });
});
