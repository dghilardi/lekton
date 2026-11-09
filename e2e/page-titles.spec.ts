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

  test('an existing editor page has its title in SSR and after hydration', async ({ page }, info) => {
    // Ingested fixtures are managed pages and intentionally cannot be edited.
    // Save through the editor API to create a hand-authored page instead.
    const slug = `e2e-editor-title-${Date.now()}-${info.retry}`;
    const saved = await page.request.post('/api/save_doc_content', {
      form: { slug, title: 'Editor title fixture', html_content: '<p>Title test</p>', access_level: 'public', order: '0' },
    });
    expect(saved.ok(), await saved.text()).toBeTruthy();
    const path = `/edit/${slug}`;
    const title = `Edit ${slug} · Editor · Lekton`;
    const response = await page.request.get(path);
    expect(response.ok()).toBeTruthy();
    expect(await response.text()).toContain(`<title>${title}</title>`);
    await page.goto(path);
    await expect(page.locator('.tiptap')).toBeVisible({ timeout: 30_000 });
    await expect(page).toHaveTitle(title);
  });

  test('the sign-in page has its own title', async ({ browser }) => {
    const anonymous = await browser.newPage();
    await anonymous.goto('/login');
    await expect(anonymous).toHaveTitle('Sign in · Lekton');
    await anonymous.close();
  });
});
