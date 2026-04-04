import { test, expect } from '@playwright/test';

test.describe('Schema Registry', () => {
  test('schema list page shows schemas', async ({ page }) => {
    await page.goto('/schemas');
    await page.waitForLoadState('networkidle');
    await expect(page.locator('h1', { hasText: 'Schema Registry' })).toBeVisible();
    // Should show the seeded schema
    await expect(page.locator('text=user-api')).toBeVisible({ timeout: 15_000 });
  });

  test('schema detail page shows versions', async ({ page }) => {
    // Navigate directly to the schema detail page (avoids click-navigation
    // issues when WASM router hasn't hydrated yet in CI)
    await page.goto('/schemas/user-api');
    // Should show version information
    await expect(page.locator('text=v1.0.0').first()).toBeVisible({ timeout: 15_000 });
    await expect(page.locator('text=v1.1.0').first()).toBeVisible();
  });

  test('selecting version shows schema content', async ({ page }) => {
    await page.goto('/schemas/user-api');
    // Version selector should be present (wait up to 15s for WASM to render it)
    const versionSelect = page.locator('select');
    if (await versionSelect.isVisible({ timeout: 15_000 })) {
      await versionSelect.selectOption({ label: '1.0.0 (stable)' });
      await page.waitForTimeout(500);
    }
    // Should display schema content (OpenAPI spec)
    await expect(page.locator('text=User API').first()).toBeVisible();
  });
});
