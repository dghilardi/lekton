import { test, expect } from '@playwright/test';

test.describe('Schema Registry', () => {
  test('schema list page shows schemas', async ({ page }) => {
    await page.goto('/schemas');
    await page.waitForLoadState('networkidle');
    await expect(page.locator('h1', { hasText: 'Schema Registry' })).toBeVisible();
    // Should show the seeded schema
    // .first() because schema name appears in both the main list and the sidebar
    await expect(page.locator('text=user-api').first()).toBeVisible({ timeout: 15_000 });
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
    // Should display schema content (OpenAPI spec).
    // Allow extra time for the local Scalar bundle (3.8 MB) to load and render.
    await expect(page.locator('text=User API').first()).toBeVisible({ timeout: 15_000 });
  });
});

test.describe('Schema viewer static assets', () => {
  test('scalar bundle is served', async ({ request }) => {
    const resp = await request.get('/js/scalar-standalone.js');
    expect(resp.status()).toBe(200);
    expect(resp.headers()['content-type']).toMatch(/javascript/);
  });

  test('asyncapi bundle is served', async ({ request }) => {
    const resp = await request.get('/js/asyncapi-standalone.js');
    expect(resp.status()).toBe(200);
    expect(resp.headers()['content-type']).toMatch(/javascript/);
  });
});

test.describe('AsyncAPI viewer', () => {
  test('asyncapi schema appears in schema list', async ({ page }) => {
    await page.goto('/schemas');
    await page.waitForLoadState('networkidle');
    await expect(page.locator('text=event-api').first()).toBeVisible({ timeout: 15_000 });
  });

  test('asyncapi schema detail page shows version', async ({ page }) => {
    await page.goto('/schemas/event-api');
    await expect(page.locator('text=v1.0.0').first()).toBeVisible({ timeout: 15_000 });
  });

  test('asyncapi viewer renders spec content', async ({ page }) => {
    await page.goto('/schemas/event-api');
    // Wait for version selector and select the only version
    const versionSelect = page.locator('select');
    if (await versionSelect.isVisible({ timeout: 15_000 })) {
      await versionSelect.selectOption({ label: '1.0.0 (stable)' });
    }
    // Wait for the AsyncAPI viewer container to receive rendered content.
    // The loading spinner is replaced once AsyncApiStandalone.render() completes.
    const viewer = page.locator('#asyncapi-viewer');
    await expect(viewer).toBeVisible({ timeout: 15_000 });
    await expect(viewer.locator('text=Event API').first()).toBeVisible({ timeout: 20_000 });
  });
});
