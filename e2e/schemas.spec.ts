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

  test('openapi viewer does not emit HTTP errors, panic, or break hydrated navigation', async ({ page }) => {
    const failures: string[] = [];
    page.on('pageerror', error => failures.push(`pageerror: ${error.message}`));
    page.on('console', message => {
      if (message.type() === 'error') {
        failures.push(`console error: ${message.text()}`);
      }
    });
    page.on('response', response => {
      const url = response.url();
      if (response.status() >= 400 && !url.endsWith('/favicon.ico')) {
        failures.push(`${response.status()}: ${url}`);
      }
    });

    const response = await page.goto('/schemas/user-api');
    expect(response?.status()).toBe(200);

    await expect(page.locator('#scalar-api-reference')).toBeVisible({ timeout: 15_000 });
    await expect(page.locator('text=User API').first()).toBeVisible({ timeout: 20_000 });
    await expect(page).toHaveURL(/\/schemas\/user-api(?:#.*)?$/);

    await page.evaluate(() => {
      window.dispatchEvent(new KeyboardEvent('keydown', { key: 'k', ctrlKey: true, bubbles: true }));
    });
    await expect(page.getByPlaceholder('Search documentation...')).toBeVisible({ timeout: 5_000 });

    expect(failures).toEqual([]);
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
    // Use toContainText instead of a nested toBeVisible because the AsyncAPI
    // viewer may render the title inside a CSS-clipped or overflow-hidden section.
    const viewer = page.locator('#asyncapi-viewer');
    await expect(viewer).toBeVisible({ timeout: 15_000 });
    await expect(viewer).toContainText('Event API', { timeout: 20_000 });
  });
});

test.describe('JSON Schema', () => {
  const SERVICE_TOKEN = process.env.LKN__AUTH__SERVICE_TOKEN || 'test-token';
  const schema = (version: string) => JSON.stringify({
    title: 'E2E Customer',
    type: 'object',
    required: ['id'],
    properties: {
      id: { type: 'string', format: 'uuid', description: 'Stable identifier.' },
      address: { $ref: '#/$defs/Address' },
      ...(version === '2.0.0' ? { tier: { enum: ['free', 'pro'] } } : {}),
    },
    $defs: { Address: { type: 'object', properties: { city: { type: 'string' } } } },
  });

  test.beforeAll(async ({ request }) => {
    for (const [version, status] of [['1.0.0', 'stable'], ['2.0.0', 'beta']]) {
      const response = await request.post('/api/v1/schemas', {
        data: { service_token: SERVICE_TOKEN, name: 'e2e-json-customer', schema_type: 'jsonschema', version, content: schema(version), status },
      });
      expect(response.ok(), await response.text()).toBeTruthy();
    }
  });

  test('opens on the stable version and outlines its properties', async ({ page }) => {
    await page.goto('/schemas');
    await expect(page.locator('main a[href="/schemas/e2e-json-customer"]')).toContainText('Current version1.0.0');
    await page.goto('/schemas/e2e-json-customer');
    await expect(page.getByRole('combobox', { name: 'Version:', exact: true })).toHaveValue('1.0.0');
    await expect(page.getByRole('heading', { level: 2, name: 'Properties' })).toBeVisible();
    const id = page.locator('.lekton-json-prop').filter({ hasText: 'id' }).first();
    await expect(id).toContainText('string');
    await expect(id).toContainText('required');
    await expect(page.locator('.lekton-json-outline').getByText('Stable identifier.')).toBeVisible();
    // A local $ref links to its definition.
    await page.getByRole('link', { name: 'Address', exact: true }).click();
    await expect(page).toHaveURL(/#json-def-Address$/);
    // The source stays one click away.
    await expect(page.getByText('Schema source')).toBeVisible();
    await expect(page.locator('.lekton-json-outline').getByText('tier')).toHaveCount(0);
  });
});
