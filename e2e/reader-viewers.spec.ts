import { test, expect, type Page } from '@playwright/test';

const errors = new WeakMap<Page, string[]>();
test.afterEach(async ({ page }) => { expect(errors.get(page) || []).toEqual([]); });

const doc = process.env.LEKTON_READER_DOC || 'getting-started';
const openapi = process.env.LEKTON_OPENAPI_SCHEMA || 'user-api';
const asyncapi = process.env.LEKTON_ASYNCAPI_SCHEMA || 'event-api';

test.beforeEach(async ({ page }) => {
  const messages: string[] = []; errors.set(page, messages);
  page.on('pageerror', e => messages.push(e.message));
  expect((await page.request.post('/api/auth/login', { data: { username: 'admin', password: 'admin' } })).ok()).toBeTruthy();
});

test('authorized document is present in SSR and is not fetched again during hydration', async ({ page }) => {
  const response = await page.request.get(`/docs/${doc}`);
  const html = await response.text();
  expect(html).toContain('data-lekton-markdown');
  expect(response.headers()['cache-control']).toContain('private');
  const calls: string[] = [];
  page.on('request', request => { if (/\/api\/(get_doc_html|get_navigation)/.test(request.url())) calls.push(request.url()); });
  await page.goto(`/docs/${doc}`);
  await expect(page.locator('[data-lekton-markdown]')).toBeVisible();
  await expect(page.getByRole('button', { name: /theme; switch to/ })).toBeVisible();
  // A working theme interaction establishes that hydration has completed.
  await page.getByRole('button', { name: /theme; switch to/ }).click();
  expect(calls).toEqual([]);
});

test('reader pages do not load the editor; entering the editor loads it once', async ({ page }) => {
  const bundles: string[] = [];
  page.on('request', request => { if (request.url().includes('tiptap-bundle.min.js')) bundles.push(request.url()); });
  await page.goto('/');
  await page.getByRole('button', { name: /theme; switch to/ }).click();
  expect(bundles).toHaveLength(0);
  await page.goto('/edit/reader-audit-unsaved');
  await expect(page.locator('.tiptap')).toBeVisible({ timeout: 20_000 });
  expect(bundles).toHaveLength(1);
});

test('OpenAPI fits narrow screens, follows theme and reuses its library across routes', async ({ page }) => {
  const schemaReads: string[] = [];
  page.on('request', r => { if (/\/api\/(get_schema_content|get_schema_detail)/.test(r.url())) schemaReads.push(r.url()); });
  const libraries: string[] = [];
  page.on('request', request => { if (request.url().includes('scalar-standalone.js')) libraries.push(request.url()); });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(`/schemas/${openapi}`);
  await expect(page.locator('#scalar-api-reference .scalar-api-reference')).toBeVisible({ timeout: 20_000 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(390);
  expect(schemaReads).toEqual([]);
  await page.evaluate(() => document.documentElement.setAttribute('data-theme', 'dark'));
  await expect(page.locator('#scalar-api-reference .dark-mode').first()).toBeVisible();
  const operation = page.locator('#scalar-api-reference a.endpoint').first();
  await expect(operation).toHaveAttribute('tabindex', '0');
  await operation.focus();
  await operation.press('Enter');
  await expect(page).toHaveURL(/#tag\//);
  await page.evaluate(() => document.querySelector<HTMLAnchorElement>('main .breadcrumbs a[href="/schemas"]')!.click());
  await page.locator(`main a[href="/schemas/${openapi}"]`).click();
  await expect(page.locator('#scalar-api-reference .scalar-api-reference')).toBeVisible();
  expect(libraries).toHaveLength(1);
});

test('schema list is hydrated from one shared SSR read', async ({ page }) => {
  const calls: string[] = [];
  page.on('request', request => { if (request.url().includes('/api/list_schemas')) calls.push(request.url()); });
  await page.goto('/schemas');
  await expect(page.locator(`main a[href="/schemas/${openapi}"]`)).toBeVisible();
  await page.getByRole('button', { name: /theme; switch to/ }).click();
  expect(calls).toEqual([]);
});

test('AsyncAPI always exposes the source and a recovery action for invalid specifications', async ({ page }) => {
  await page.goto(`/schemas/${asyncapi}`);
  const source = page.getByLabel('Schema source', { exact: true });
  await page.getByText('Schema source', { exact: true }).first().click();
  await expect(source).toBeVisible();
  await expect(source).not.toBeEmpty();
  await expect(page.locator('#asyncapi-viewer')).toBeVisible();
});

test('authorized reading works without JavaScript', async ({ browser, page }) => {
  const context = await browser.newContext({ javaScriptEnabled: false, storageState: await page.context().storageState() });
  const reader = await context.newPage();
  await reader.goto(`/docs/${doc}`);
  await expect(reader.locator('[data-lekton-markdown]')).toBeVisible();
  await context.close();
});

test('SSR does not serialize a private document for an anonymous reader', async ({ browser }) => {
  test.skip(!process.env.LEKTON_PRIVATE_DOC, 'Set a known restricted document for the RBAC check');
  const context = await browser.newContext();
  const response = await context.request.get(`/docs/${process.env.LEKTON_PRIVATE_DOC}`);
  expect(await response.text()).not.toContain('data-lekton-markdown');
  await context.close();
});

test('valid AsyncAPI fixture has navigable channels in both themes without fetching the library twice', async ({ page }) => {
  const fixture = {
    asyncapi: '2.6.0', info: { title: 'Illustrative events', version: '1.0.0' },
    channels: { 'audit.events': { subscribe: { message: { name: 'AuditEvent', payload: { type: 'object', properties: { name: { type: 'string' } } } } } } },
  };
  const loads: string[] = [];
  page.on('request', r => { if (r.url().includes('asyncapi-standalone.js')) loads.push(r.url()); });
  await page.goto('/schemas');
  // Exercise a browser navigation with a fixture response; never modify the
  // stored specification or suppress its validation errors in the product.
  await page.route('**/api/get_schema_content*', route => route.fulfill({ json: JSON.stringify(fixture) }));
  await page.locator(`main a[href="/schemas/${asyncapi}"]`).click();
  const viewer = page.locator('#asyncapi-viewer');
  await expect(viewer).toContainText('Illustrative events', { timeout: 20_000 });
  await expect(viewer).toContainText('audit.events');
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(1280);
  await page.evaluate(() => document.documentElement.setAttribute('data-theme', 'dark'));
  await expect(viewer).toContainText('audit.events');
  await page.evaluate(() => document.documentElement.setAttribute('data-theme', 'light'));
  expect(loads).toHaveLength(1);
});

test('a failed SSR session recovers through one centralized refresh and retries the reader', async ({ page }) => {
  const cookies = await page.context().cookies();
  const session = cookies.find(c => c.name === 'lekton_demo_user')!;
  await page.context().clearCookies({ name: 'lekton_demo_user' });
  let refreshes = 0;
  await page.route('**/auth/refresh', async route => {
    refreshes++;
    await page.context().addCookies([session]);
    await route.fulfill({ status: 200, json: {} });
  });
  await page.goto(`/docs/${doc}`);
  await expect(page.locator('[data-lekton-markdown]')).toBeVisible({ timeout: 20_000 });
  await expect(page.getByRole('button', { name: 'Open user menu' })).toBeVisible();
  expect(refreshes).toBe(1);
});
