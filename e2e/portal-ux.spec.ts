import { test, expect } from '@playwright/test';

// These checks exercise the shell without depending on a particular corpus.
test('search isolates focus, closes with Escape and restores its trigger', async ({ page }) => {
  await page.goto('/');
  const trigger = page.getByRole('button', { name: 'Open search', exact: true });
  await expect(trigger).toBeVisible({ timeout: 30_000 });
  // Wait for hydration before opening the client-side dialog.
  await expect(async () => {
    await trigger.click();
    await expect(page.getByRole('dialog', { name: 'Search documentation' })).toBeVisible();
  }).toPass();
  const input = page.getByRole('combobox', { name: 'Search documentation' });
  await expect(input).toBeFocused();
  await page.keyboard.press('Shift+Tab');
  await expect(page.getByRole('button', { name: 'Close search' })).toBeFocused();
  await page.keyboard.press('Tab');
  await expect(input).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(page.getByRole('dialog')).not.toBeVisible();
  await expect(trigger).toBeFocused();
});

test('mobile navigation opens by keyboard and returns focus on Escape', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/schemas');
  const trigger = page.getByRole('button', { name: 'Open navigation menu' });
  await expect(async () => {
    await trigger.focus();
    await page.keyboard.press('Enter');
    await expect(page.locator('#sidebar-trigger')).toHaveAttribute('aria-expanded', 'true');
  }).toPass();
  await expect(page.getByRole('button', { name: 'Close navigation', exact: true })).toBeFocused();
  await expect(page.locator('.drawer-content')).toHaveAttribute('inert', '');
  await page.keyboard.press('Escape');
  await expect(trigger).toHaveAttribute('aria-expanded', 'false');
  await expect(trigger).toBeFocused();
  expect(await page.locator('#sidebar-drawer').getAttribute('tabindex')).toBe('-1');
});

test('skip link is first and targets the main content', async ({ page }) => {
  await page.goto('/');
  await page.keyboard.press('Tab');
  const skip = page.getByRole('link', { name: 'Skip to content' });
  await expect(skip).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(page.locator('main#content')).toBeFocused();
});

test('one arrow press advances one search result', async ({ page }) => {
  await page.route('**/api/search_docs*', route => route.fulfill({
    contentType: 'application/json',
    body: JSON.stringify([0, 1, 2].map(i => ({
      slug: `guide-${i}`, title: `Guide ${i}`, tags: [], content_preview: 'Illustrative test result',
    }))),
  }));
  await page.goto('/');
  await expect(async () => {
    await page.getByRole('button', { name: 'Open search', exact: true }).click();
    await expect(page.getByRole('dialog')).toBeVisible();
  }).toPass();
  const input = page.getByRole('combobox');
  await input.fill('guide');
  await expect(page.getByRole('option')).toHaveCount(3);
  await expect(input).toHaveAttribute('aria-activedescendant', 'global-search-result-0');
  await input.press('ArrowDown');
  await expect(input).toHaveAttribute('aria-activedescendant', 'global-search-result-1');
  await expect(page.locator('#global-search-result-1')).toHaveAttribute('aria-selected', 'true');
  await expect(page.locator('#global-search-result-0')).toHaveAttribute('aria-selected', 'false');
});

test('search failure offers a readable recovery message', async ({ page }) => {
  await page.route('**/api/search_docs*', route => route.fulfill({ status: 500, body: 'Audit injected failure' }));
  await page.goto('/');
  await expect(async () => {
    await page.getByRole('button', { name: 'Open search', exact: true }).click();
    await expect(page.getByRole('dialog')).toBeVisible();
  }).toPass();
  await page.getByRole('combobox').fill('guide');
  await expect(page.getByText('Search is temporarily unavailable')).toBeVisible();
  await page.getByRole('button', { name: 'Close search' }).click();
  await expect(page.getByRole('dialog')).not.toBeVisible();
});

test('anonymous empty portal offers sign-in without duplicate primary destinations', async ({ page }) => {
  await page.route('**/api/get_navigation*', route => route.fulfill({ contentType: 'application/json', body: '[]' }));
  await page.goto('/');
  await expect(page.getByRole('link', { name: 'Log in to explore' })).toHaveAttribute('href', '/login');
  await expect(page.getByRole('link', { name: 'API Schemas', exact: true })).toHaveAttribute('href', '/schemas');
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(page.getByRole('link', { name: 'Lekton home', exact: true })).toBeVisible();
});

test('home stays neutral until auth and navigation resolve', async ({ page, request }) => {
  const html = await (await request.get('/')).text();
  expect(html).not.toContain('Log in to explore');
  await page.route('**/api/get_navigation*', async route => {
    await new Promise(resolve => setTimeout(resolve, 800));
    await route.fulfill({ contentType: 'application/json', body: '[]' });
  });
  await page.goto('/');
  await expect(page.getByRole('link', { name: 'Log in to explore' })).toBeVisible();
});

test('keyboard search selection remains inside the scrolled results viewport', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.route('**/api/search_docs*', route => route.fulfill({
    contentType: 'application/json', body: JSON.stringify(Array.from({ length: 12 }, (_, i) => ({
      slug: `result-${i}`, title: `Result ${i}`, tags: [], content_preview: 'Illustrative result preview with enough text to occupy a result row.',
    }))),
  }));
  await page.goto('/');
  await expect(async () => {
    await page.getByRole('button', { name: 'Open search', exact: true }).click();
    await expect(page.getByRole('dialog')).toBeVisible();
  }).toPass();
  const input = page.getByRole('combobox');
  await input.fill('result');
  await expect(page.getByRole('option')).toHaveCount(12);
  for (let i = 0; i < 9; i++) await input.press('ArrowDown');
  await expect(input).toHaveAttribute('aria-activedescendant', 'global-search-result-9');
  const result = await page.locator('#global-search-result-9').boundingBox();
  const list = await page.locator('#global-search-results').boundingBox();
  expect(result!.y).toBeGreaterThanOrEqual(list!.y - 1);
  expect(result!.y + result!.height).toBeLessThanOrEqual(list!.y + list!.height + 1);
});

test('signed-in readers with no docs or registry get a profile action', async ({ page }) => {
  const login = await page.request.post('/api/auth/login', { data: { username: 'admin', password: 'admin' } });
  expect(login.ok()).toBeTruthy();
  expect(await (await page.request.get('/')).text()).not.toContain('Log in to explore');
  await page.route('**/api/get_navigation*', route => route.fulfill({ contentType: 'application/json', body: '[]' }));
  await page.route('**/api/get_feature_flags*', async route => {
    const response = await route.fetch();
    await route.fulfill({ json: { ...await response.json(), schema_registry: false } });
  });
  await page.goto('/');
  await expect(page.getByRole('link', { name: 'View your profile' })).toHaveAttribute('href', '/profile');
  await expect(page.getByRole('link', { name: 'Log in to explore' })).toHaveCount(0);
});
