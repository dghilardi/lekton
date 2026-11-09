import { test, expect, type Page } from '@playwright/test';

const errors = new WeakMap<Page, string[]>();
test.afterEach(async ({ page }) => { expect(errors.get(page) || []).toEqual([]); });

const doc = process.env.LEKTON_READER_DOC || 'getting-started';
const openapi = process.env.LEKTON_OPENAPI_SCHEMA || 'user-api';
const asyncapi = process.env.LEKTON_ASYNCAPI_SCHEMA || 'event-api';
const multiVersionSchema = process.env.LEKTON_MULTI_VERSION_SCHEMA || openapi;

async function establishHydration(page: Page) {
  const toggle = page.getByRole('button', { name: /theme; switch to/ });
  await expect(async () => {
    const before = await toggle.getAttribute('aria-label');
    await toggle.click();
    await expect(toggle).not.toHaveAttribute('aria-label', before!, { timeout: 500 });
  }).toPass({ timeout: 20_000 });
}

async function selectTheme(page: Page, theme: 'light' | 'dark') {
  const toggle = page.getByRole('button', { name: /theme; switch to/ });
  for (let i = 0; i < 3 && !(await toggle.getAttribute('aria-label'))!.toLowerCase().startsWith(`${theme} theme`); i++) await toggle.click();
  await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
}

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
  await establishHydration(page);
  expect(calls).toEqual([]);
});

test('reader pages do not load the editor; entering the editor loads it once', async ({ page }) => {
  const bundles: string[] = [];
  page.on('request', request => { if (request.url().includes('tiptap-bundle.min.js')) bundles.push(request.url()); });
  await page.goto('/');
  await establishHydration(page);
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
  await expect(page.getByRole('combobox', { name: 'Version:', exact: true })).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(390);
  expect(schemaReads).toEqual([]);
  // Switch version and theme only after hydration, which would otherwise
  // restore the stored theme over an attribute set too early.
  await establishHydration(page);
  // The seeded user-api's latest version is a beta without operations, and
  // Scalar marks themed content only on operations: use the stable version.
  if (openapi === 'user-api') await page.getByRole('combobox', { name: 'Version:', exact: true }).selectOption('1.0.0');
  await selectTheme(page, 'dark');
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

test('changing a schema version fetches only that source and keeps one themed viewer', async ({ page }) => {
  const loads: string[] = [], reads: string[] = [];
  page.on('request', request => {
    if (request.url().includes('scalar-standalone.js')) loads.push(request.url());
    if (request.url().includes('/api/get_schema_content')) reads.push(request.url());
  });
  await page.goto(`/schemas/${multiVersionSchema}`);
  await expect(page.locator('#scalar-api-reference .scalar-api-reference')).toBeVisible({ timeout: 20_000 });
  await selectTheme(page, 'dark');
  expect(reads).toHaveLength(0);
  const selector = page.getByRole('combobox', { name: 'Version:', exact: true });
  const initial = await selector.inputValue();
  const versions = await selector.locator('option').evaluateAll(options => options.map(o => (o as HTMLOptionElement).value));
  expect(versions.length, 'Use a schema with at least two versions').toBeGreaterThan(1);
  const next = versions.find(version => version !== initial)!;
  const response = page.waitForResponse(r => r.url().includes('/api/get_schema_content') && r.ok());
  await selector.selectOption(next);
  const content = await (await response).json();
  await expect(page.getByLabel('Schema source', { exact: true })).toHaveText(content);
  await expect(page.locator('#scalar-api-reference .dark-mode').first()).toBeVisible();
  await expect(page.locator('#scalar-api-reference .scalar-api-reference')).toHaveCount(1);
  expect(reads).toHaveLength(1);
  expect(loads).toHaveLength(1);
});

test('schema list is hydrated from one shared SSR read', async ({ page }) => {
  const calls: string[] = [];
  page.on('request', request => { if (request.url().includes('/api/list_schemas')) calls.push(request.url()); });
  await page.goto('/schemas');
  await expect(page.locator(`main a[href="/schemas/${openapi}"]`)).toBeVisible();
  await establishHydration(page);
  expect(calls).toEqual([]);
});

test('AsyncAPI source stays available independently of validation', async ({ page }) => {
  await page.goto(`/schemas/${asyncapi}`);
  const source = page.getByLabel('Schema source', { exact: true });
  // An invalid specification opens the source asynchronously; a valid one
  // leaves it collapsed. Either way the reader can reach it.
  await expect(async () => {
    if (!await page.locator('.lekton-spec > details').evaluate(e => (e as HTMLDetailsElement).open)) {
      await page.getByText('Schema source', { exact: true }).first().click();
    }
    await expect(source).toBeVisible({ timeout: 1_000 });
  }).toPass({ timeout: 15_000 });
  await expect(source).not.toBeEmpty();
  await expect(page.locator('#asyncapi-viewer')).toBeVisible();
});

test('authorized reading works without JavaScript', async ({ browser, page }) => {
  const context = await browser.newContext({ javaScriptEnabled: false, storageState: await page.context().storageState() });
  const reader = await context.newPage();
  await reader.goto(`/docs/${doc}`);
  await expect(reader.locator('[data-lekton-markdown]')).toBeVisible();
  await reader.goto(`/schemas/${openapi}`);
  await expect(reader.getByRole('status')).toContainText('requires JavaScript');
  await reader.getByText('Schema source', { exact: true }).first().click();
  await expect(reader.getByLabel('Schema source', { exact: true })).toBeVisible();
  await context.close();
});

test('a failed viewer module import exposes the original source', async ({ page }) => {
  await page.route('**/js/schema-viewers.js*', route => route.abort());
  await page.goto(`/schemas/${openapi}`);
  await expect(page.getByRole('alert')).toContainText('interactive viewer is unavailable');
  await expect(page.getByLabel('Schema source', { exact: true })).toBeVisible();
  await expect(page.getByLabel('Schema source', { exact: true })).not.toBeEmpty();
});

test('JSON Schema keeps its heading and formats minified JSON without changing its value', async ({ page }) => {
  await page.goto('/schemas');
  await establishHydration(page);
  await page.route('**/api/get_schema_detail*', async route => {
    const response = await route.fetch();
    const detail = await response.json(); detail.schema_type = 'jsonschema';
    await route.fulfill({ response, json: detail });
  });
  const fixture = { title: 'Formatted fixture', type: 'object', properties: { id: { type: 'string' } } };
  await page.route('**/api/get_schema_content*', route => route.fulfill({ json: JSON.stringify(fixture) }));
  await page.locator(`main a[href="/schemas/${openapi}"]`).click();
  await expect(page.getByRole('heading', { name: 'JSON Schema', exact: true })).toBeVisible();
  const source = page.getByLabel('Schema source', { exact: true });
  await expect(source).toBeVisible();
  const text = (await source.textContent())!;
  expect(text).toContain('\n  "');
  expect(JSON.parse(text)).toEqual(fixture);
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
  const toggle = viewer.getByRole('button', { name: 'Schema navigation', exact: true });
  await expect(toggle).toHaveAttribute('aria-expanded', 'false');
  await toggle.focus();
  await toggle.press('Enter');
  await expect(toggle).toHaveAttribute('aria-expanded', 'true');
  const channel = viewer.locator('a[href^="#"]').filter({ hasText: 'audit.events' }).first();
  await expect(channel).toBeVisible();
  const target = (await channel.getAttribute('href'))!;
  await channel.focus();
  await channel.press('Enter');
  await expect.poll(() => new URL(page.url()).hash).toBe(target);
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

test('expired-session SSR offers sign-in recovery before JavaScript runs', async ({ browser, page }) => {
  const state = await page.context().storageState();
  state.cookies = state.cookies.filter(c => c.name !== 'lekton_demo_user');
  const context = await browser.newContext({ javaScriptEnabled: false, storageState: state });
  try {
    const reader = await context.newPage();
    for (const path of [`/docs/${doc}`, '/schemas', `/schemas/${openapi}`]) {
      await reader.goto(path);
      // Page-level reads show the sign-in state; region reads an inline notice.
      const signIn = reader.getByRole('link', { name: /^(Log in again|Sign in)$/ });
      await expect(signIn).toBeVisible();
      await expect(signIn).toHaveAttribute('href', `/login?next=${path}`);
      await expect(reader.locator('main')).not.toContainText('error running server function');
      await expect(reader.getByRole('navigation', { name: 'Section navigation' })).not.toContainText('error running server function');
    }
  } finally { await context.close(); }
});

for (const scenario of ['recovered', 'still unauthorized', 'refresh rejected', 'unrelated error'] as const) {
  test(`client reader retry is bounded: ${scenario}`, async ({ page }) => {
    await page.goto('/schemas');
    await establishHydration(page);
    let reads = 0, refreshes = 0;
    await page.route('**/api/get_doc_html*', async route => {
      reads++;
      if (scenario === 'recovered' && reads > 1) { await route.continue(); return; }
      await route.fulfill({ status: 500, contentType: 'text/plain', body: scenario === 'unrelated error' ? 'ServerError|audit unavailable' : 'ServerError|unauthorized' });
    });
    await page.route('**/auth/refresh', async route => {
      refreshes++;
      await route.fulfill({ status: scenario === 'refresh rejected' ? 401 : 200, json: {} });
    });
    // A fixture link exercises Leptos client navigation without modifying any
    // persisted document or depending on the selected corpus's sidebar depth.
    await page.evaluate(href => {
      const link = document.createElement('a'); link.href = href;
      link.textContent = 'Read fixture'; document.querySelector('main')!.append(link);
    }, `/docs/${doc}`);
    await page.getByRole('link', { name: 'Read fixture', exact: true }).click();
    if (scenario === 'recovered') {
      await expect(page.locator('[data-lekton-markdown]')).toBeVisible();
    } else if (scenario === 'refresh rejected') {
      await expect(page).toHaveURL(new RegExp(`/login\\?next=/docs/${doc}$`));
    } else if (scenario === 'still unauthorized') {
      await expect(page.getByRole('main').getByRole('link', { name: 'Sign in', exact: true })).toBeVisible();
    } else {
      await expect(page.locator('main')).toContainText('audit unavailable');
    }
    expect(reads).toBe(scenario === 'recovered' || scenario === 'still unauthorized' ? 2 : 1);
    expect(refreshes).toBe(scenario === 'unrelated error' ? 0 : 1);
  });
}

for (const width of [1440, 1024, 768, 390]) {
  for (const theme of ['light', 'dark'] as const) {
    test(`section navigation stays within the viewport: ${width}px, ${theme}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 844 });
      await page.goto(`/docs/${doc}`);
      await establishHydration(page);
      await selectTheme(page, theme);
      if (width < 1024) {
        await page.getByRole('button', { name: 'Open navigation menu', exact: true }).click();
      } else {
        await page.evaluate(() => window.scrollTo(0, 700));
      }
      const nav = page.getByRole('navigation', { name: 'Section navigation' });
      await expect(nav).toBeVisible();
      await nav.evaluate(element => { element.scrollTop = element.scrollHeight; });
      const box = (await nav.boundingBox())!;
      expect(box.y).toBeGreaterThanOrEqual(64);
      expect(box.y + box.height).toBeLessThanOrEqual(845);
      const last = (await nav.locator('a:visible').last().boundingBox())!;
      expect(last.y).toBeGreaterThanOrEqual(box.y);
      expect(last.y + last.height).toBeLessThanOrEqual(845);
    });
  }
}
