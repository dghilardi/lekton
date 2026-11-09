import { test, expect, type APIRequestContext } from '@playwright/test';

// Pages that create browser-only actions used to crash the server when loaded
// directly: the response owner's cleanup dropped those actions on another
// worker thread, truncating the HTML and, on a second panic, aborting the
// process. Concurrent loads make different worker threads finish responses.
const paths = ['/learn', '/learn/unknown-path', '/admin/rag', '/prompts', '/profile'];

async function loadConcurrently(request: APIRequestContext, path: string, times: number) {
  const responses = await Promise.all(Array.from({ length: times }, () => request.get(path)));
  for (const response of responses) {
    // 404 when the page's feature is disabled (its not-found page still renders).
    expect([200, 404], path).toContain(response.status());
    expect(await response.text(), `${path} returned truncated HTML`).toContain('</html>');
  }
}

test.describe('direct loads of pages with browser-only actions', () => {
  test.beforeEach(async ({ page }) => {
    expect((await page.request.post('/api/auth/login', { data: { username: 'admin', password: 'admin' } })).ok()).toBeTruthy();
  });

  for (const path of paths) {
    test(`${path} renders completely on repeated and concurrent loads`, async ({ page, request }) => {
      await loadConcurrently(page.request, path, 16);
      await loadConcurrently(request, path, 16);
      for (let i = 0; i < 2; i++) {
        await page.goto(path);
        await expect(page.locator('main')).toBeVisible();
      }
      expect((await request.get('/login')).ok(), 'server stopped responding').toBeTruthy();
    });
  }
});
