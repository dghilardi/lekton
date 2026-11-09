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
    // Create a hand-authored page through the UI (server-function URLs are hashed).
    const slug = `e2e-editor-title-${Date.now()}-${info.retry}`;
    const path = `/edit/${slug}`;
    await page.goto(path);
    await expect(page.locator('.tiptap')).toBeVisible({ timeout: 30_000 });
    await page.getByLabel('Document Title', { exact: true }).fill('Editor title fixture');
    await page.getByLabel('Access level', { exact: true }).selectOption({ index: 1 });
    await page.locator('.tiptap').fill('Title test');
    await page.getByRole('button', { name: 'Create Page' }).click();
    await expect(page.getByText(/saved successfully/)).toBeVisible({ timeout: 30_000 });
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
