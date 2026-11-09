import { test, expect } from '@playwright/test';

test.describe('unknown routes', () => {
  test('an unknown page path returns the not-found page with status 404', async ({ request }) => {
    const response = await request.get('/definitely-not-a-page/really');
    expect(response.status()).toBe(404);
    expect(response.headers()['content-type']).toContain('text/html');
    const html = await response.text();
    expect(html).toMatch(/<html[^>]* lang="/);
    expect(html).toContain('There is nothing at this address');
    expect(html).toContain('href="/"');
  });

  test('the not-found page is navigable in the browser', async ({ page }) => {
    const response = await page.goto('/definitely-not-a-page');
    expect(response?.status()).toBe(404);
    await expect(page.getByRole('heading', { level: 1, name: 'Page not found' })).toBeVisible();
    await expect(page.getByRole('main').getByRole('link', { name: 'Go to home' })).toBeVisible();
    await page.getByRole('main').getByRole('link', { name: 'Go to home' }).click();
    await expect(page).toHaveURL(/\/$/);
  });

  for (const path of ['/js/missing-asset.js', '/pkg/missing.wasm', '/missing-image.png']) {
    test(`a missing asset ${path} stays a bare 404`, async ({ request }) => {
      const response = await request.get(path);
      expect(response.status()).toBe(404);
      expect(await response.text()).not.toContain('<html');
    });
  }
});
