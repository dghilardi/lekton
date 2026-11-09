import { test, expect, type Page } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';

// Form fields used to have visual labels only, the editor body and toolbar had
// no usable names, and an empty save surfaced a raw 500 from the server.
const formRules = ['label', 'select-name', 'aria-input-field-name', 'button-name'];

async function expectNamedFields(page: Page, context: string) {
  const { violations } = await new AxeBuilder({ page }).withRules(formRules).analyze();
  expect(violations.map(v => `${v.id}: ${v.nodes.map(n => n.target.join(' ')).join(', ')}`), context).toEqual([]);
}

test.describe('form labels and validation', () => {
  test.beforeEach(async ({ page }) => {
    expect((await page.request.post('/api/auth/login', { data: { username: 'admin', password: 'admin' } })).ok()).toBeTruthy();
  });

  test('editor fields, body and toolbar have accessible names', async ({ page }) => {
    await page.goto('/edit/e2e-labels-page');
    await expect(page.locator('.tiptap')).toBeVisible({ timeout: 30_000 });
    for (const label of ['Slug (page path)', 'Document Title', 'Access level', 'Parent (optional)', 'Order']) {
      await expect(page.getByLabel(label, { exact: true })).toBeVisible();
    }
    await expect(page.getByRole('textbox', { name: 'Page content' })).toBeVisible();
    const toolbar = page.getByRole('toolbar', { name: 'Formatting' });
    for (const name of ['Bold', 'Italic', 'Strikethrough', 'Heading 1 (H1)', 'Bulleted list', 'Numbered list', 'Quote', 'Highlight', 'Insert image']) {
      await expect(toolbar.getByRole('button', { name, exact: true })).toBeVisible();
    }
    await expectNamedFields(page, 'editor');
  });

  test('an empty editor save marks the fields and focuses the first one without calling the server', async ({ page }) => {
    let saves = 0;
    page.on('request', request => { if (request.url().includes('/api/save_doc_content')) saves++; });
    await page.goto('/edit/e2e-labels-empty');
    await expect(page.locator('.tiptap')).toBeVisible({ timeout: 30_000 });
    const slug = page.getByLabel('Slug (page path)', { exact: true });
    await slug.fill('');
    await page.getByRole('button', { name: 'Create Page' }).click();
    await expect(slug).toBeFocused();
    await expect(slug).toHaveAttribute('aria-invalid', 'true');
    await expect(slug).toHaveAccessibleDescription(/Enter the page path/);
    await expect(page.getByLabel('Document Title', { exact: true })).toHaveAccessibleDescription('Enter a title.');
    await expect(page.getByLabel('Access level', { exact: true })).toHaveAttribute('aria-invalid', 'true');
    await slug.fill('e2e-labels-fixed');
    await expect(slug).not.toHaveAttribute('aria-invalid', 'true');
    expect(saves).toBe(0);
  });

  test('a server-side validation error answers 400 and is shown on its field', async ({ page }) => {
    await page.goto('/edit/e2e-labels-server');
    await expect(page.locator('.tiptap')).toBeVisible({ timeout: 30_000 });
    await page.getByLabel('Document Title', { exact: true }).fill('Server validation');
    await page.getByLabel('Access level', { exact: true }).selectOption({ index: 1 });
    // Bypass the client-side check so the server's own validation answers.
    await page.route('**/api/save_doc_content*', route => {
      const form = new URLSearchParams(route.request().postData() ?? '');
      form.set('slug', '');
      return route.continue({ postData: form.toString() });
    });
    const response = page.waitForResponse(r => r.url().includes('/api/save_doc_content'));
    await page.getByRole('button', { name: 'Create Page' }).click();
    const answer = await response;
    expect(answer.status()).toBe(400);
    expect(await answer.text()).not.toContain('Bad request:');
    const slug = page.getByLabel('Slug (page path)', { exact: true });
    await expect(slug).toHaveAccessibleDescription('Slug is required');
    await expect(slug).toBeFocused();
  });

  test('document upload fields are labelled and validated in place', async ({ page }) => {
    await page.goto('/admin/upload');
    const create = page.getByRole('button', { name: 'Create document' });
    test.skip(!(await create.isVisible({ timeout: 15_000 }).catch(() => false)), 'document upload is disabled');
    for (const label of ['Title', 'Description', 'Access level', 'Parent (optional)', 'Order']) {
      await expect(page.getByLabel(label, { exact: true })).toBeVisible();
    }
    await create.click();
    const pdf = page.getByRole('button', { name: /Choose PDF/ });
    await expect(pdf).toBeFocused();
    await expect(pdf).toHaveAttribute('aria-invalid', 'true');
    await expect(page.getByLabel('Title', { exact: true })).toHaveAccessibleDescription('Enter a title.');
    await expectNamedFields(page, 'document upload');
  });

  test('learn controls are labelled', async ({ page }) => {
    await page.goto('/learn');
    const heading = page.getByRole('heading', { name: 'Learn', exact: true });
    test.skip(!(await heading.isVisible({ timeout: 15_000 }).catch(() => false)), 'learn mode is disabled');
    await expect(page.getByRole('switch', { name: 'Save my progress' })).toBeVisible();
    await expect(page.getByLabel('Scope type', { exact: true })).toBeVisible();
    await expect(page.getByLabel('Tag, topic or document slug', { exact: true })).toBeVisible();
    await expectNamedFields(page, 'learn');
  });
});
