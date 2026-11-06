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
