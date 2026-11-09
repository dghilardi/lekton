import { test, expect } from '@playwright/test';

// Every page used to share one <title>, so tabs, history and screen readers
// could not tell pages apart.
const pages: [string, string][] = [
  ['/', 'Lekton — Internal Developer Portal'],
  ['/docs/getting-started', 'Getting Started · Docs · Lekton'],
  ['/schemas', 'Schema Registry · Lekton'],
  ['/schemas/user-api', 'user-api · Schemas · Lekton'],
  ['/prompts', 'Prompt Library · Lekton'],
  ['/profile', 'Profile · Lekton'],
  ['/admin/tokens', 'Service Tokens · Admin · Lekton'],
  ['/edit/e2e-title-new-page', 'Create page · Editor · Lekton'],
  ['/no-such-page', 'Page not found · Lekton'],
];

test.describe('page titles', () => {
  test.beforeEach(async ({ page }) => {
    expect((await page.request.post('/api/auth/login', { data: { username: 'admin', password: 'admin' } })).ok()).toBeTruthy();
  });

  test('each page names itself, its section and the product', async ({ page }) => {
    for (const [path, title] of pages) {
      await page.goto(path);
      await expect(page, path).toHaveTitle(title);
    }
    expect(new Set(pages.map(([, title]) => title)).size).toBe(pages.length);
  });

  test('server-rendered pages carry their title before JavaScript runs', async ({ page }) => {
    for (const [path, title] of [pages[1], pages[3], pages[8]]) {
      const html = await (await page.request.get(path)).text();
      expect(html, path).toContain(`<title>${title}</title>`);
    }
  });

  test('the sign-in page has its own title', async ({ browser }) => {
    const anonymous = await browser.newPage();
    await anonymous.goto('/login');
    await expect(anonymous).toHaveTitle('Sign in · Lekton');
    await anonymous.close();
  });
});
