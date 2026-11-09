import { test, expect, type Page } from '@playwright/test';

// Leaving the editor used to drop every unsaved change without a word.
test.describe('Editor unsaved changes', () => {
  let editPath: string;

  test.beforeEach(async ({ page }, info) => {
    expect((await page.request.post('/api/auth/login', { data: { username: 'admin', password: 'admin' } })).ok()).toBeTruthy();
    editPath = `/edit/e2e-unsaved-${Date.now()}-${info.retry}`;
  });

  async function openEditor(page: Page) {
    await page.goto(editPath);
    // TipTap mounts after hydration, so field input reaches the app from here.
    await expect(page.locator('.tiptap')).toBeVisible({ timeout: 30_000 });
  }

  const titleInput = (page: Page) =>
    page.locator('.form-control', { hasText: 'Document Title' }).locator('input');

  test('an in-app link asks before leaving and staying keeps the edits', async ({ page }) => {
    await openEditor(page);
    await titleInput(page).fill('Unsaved title');
    const dialog = page.waitForEvent('dialog');
    const click = page.getByRole('link', { name: 'Lekton home' }).click();
    const prompt = await dialog;
    expect(prompt.type()).toBe('confirm');
    expect(prompt.message()).toContain('unsaved changes');
    await prompt.dismiss();
    await click;
    await expect(page).toHaveURL(new RegExp(`${editPath}$`));
    await expect(titleInput(page)).toHaveValue('Unsaved title');
  });

  test('leaving anyway keeps a local draft that can be restored or discarded', async ({ page }) => {
    await openEditor(page);
    await titleInput(page).fill('Draft title');
    await page.locator('.tiptap').click();
    await page.keyboard.type('Draft body');
    page.once('dialog', prompt => prompt.accept());
    await page.getByRole('link', { name: 'Lekton home' }).click();
    await expect(page).toHaveURL(/\/$/);

    await openEditor(page);
    await expect(titleInput(page)).toHaveValue('');
    await expect(page.getByText('unsaved changes from an earlier visit')).toBeVisible();
    await page.getByRole('button', { name: 'Restore draft' }).click();
    await expect(titleInput(page)).toHaveValue('Draft title');
    await expect(page.locator('.tiptap')).toContainText('Draft body');
    await expect(page.getByText('Unsaved changes', { exact: true })).toBeVisible();
  });

  test('a pending draft must be resolved before editing or saving', async ({ page }) => {
    await openEditor(page);
    await titleInput(page).fill('Earlier draft');
    page.once('dialog', prompt => prompt.accept());
    await page.getByRole('link', { name: 'Lekton home' }).click();
    await openEditor(page);

    await expect(page.getByRole('button', { name: 'Restore draft' })).toBeVisible();
    await expect(titleInput(page)).toBeDisabled();
    await expect(page.locator('#editor-slug')).toBeDisabled();
    for (const select of await page.getByRole('group', { name: 'Page editor' }).locator('select').all()) {
      await expect(select).toBeDisabled();
    }
    await expect(page.getByRole('button', { name: 'Bold', exact: true })).toBeDisabled();
    await expect(page.getByRole('button', { name: 'Create Page' })).toBeDisabled();
    await expect(page.locator('.tiptap')).toHaveAttribute('contenteditable', 'false');

    await page.getByRole('button', { name: 'Discard draft' }).click();
    await expect(titleInput(page)).toBeEnabled();
    await expect(page.locator('.tiptap')).toHaveAttribute('contenteditable', 'true');
    await titleInput(page).fill('New draft after choosing');
    page.once('dialog', prompt => prompt.accept());
    await page.getByRole('link', { name: 'Lekton home' }).click();
    await openEditor(page);
    await page.getByRole('button', { name: 'Restore draft' }).click();
    await expect(titleInput(page)).toHaveValue('New draft after choosing');
    await expect(titleInput(page)).toBeEnabled();
    await expect(page.locator('.tiptap')).toHaveAttribute('contenteditable', 'true');
  });

  test('reverting field edits drops the local draft', async ({ page }) => {
    await openEditor(page);
    await titleInput(page).fill('Temporary title');
    await titleInput(page).fill('');
    page.on('dialog', prompt => { throw new Error(`unexpected ${prompt.type()} dialog`); });
    await page.reload();
    await expect(page.locator('.tiptap')).toBeVisible({ timeout: 30_000 });
    await expect(page.getByText('unsaved changes from an earlier visit')).toHaveCount(0);
  });

  test('a discarded draft is not offered again', async ({ page }) => {
    await openEditor(page);
    await titleInput(page).fill('Draft to discard');
    page.once('dialog', prompt => prompt.accept());
    await page.getByRole('link', { name: 'Lekton home' }).click();
    await openEditor(page);
    await page.getByRole('button', { name: 'Discard draft' }).click();
    await page.reload();
    await expect(page.locator('.tiptap')).toBeVisible({ timeout: 30_000 });
    await expect(page.getByText('unsaved changes from an earlier visit')).toHaveCount(0);
  });

  test('saving clears the unsaved state', async ({ page }) => {
    await openEditor(page);
    await titleInput(page).fill('Saved title');
    await page.locator('.form-control', { hasText: 'Access level' }).locator('select').selectOption({ index: 1 });
    await page.locator('.tiptap').click();
    await page.keyboard.type('Saved body');
    await expect(page.getByText('Unsaved changes', { exact: true })).toBeVisible();
    await page.getByRole('button', { name: 'Create Page' }).click();
    // Saving also indexes the page for search and retrieval.
    await expect(page.getByText('Unsaved changes', { exact: true })).toHaveCount(0, { timeout: 30_000 });
    await expect(page.getByText(/saved successfully/)).toBeVisible();
    page.on('dialog', prompt => { throw new Error(`unexpected ${prompt.type()} dialog`); });
    await page.getByRole('link', { name: 'Lekton home' }).click();
    await expect(page).toHaveURL(/\/$/);
  });

  test('an expired session is refreshed once and the save retried', async ({ page }) => {
    await openEditor(page);
    let saves = 0, refreshes = 0;
    await page.route('**/api/save_doc_content*', async route => {
      saves++;
      if (saves === 1) {
        await route.fulfill({ status: 500, contentType: 'text/plain', body: 'ServerError|unauthorized' });
      } else {
        await route.continue();
      }
    });
    await page.route('**/auth/refresh', async route => { refreshes++; await route.fulfill({ status: 200, json: {} }); });
    await titleInput(page).fill('Retried title');
    await page.locator('.form-control', { hasText: 'Access level' }).locator('select').selectOption({ index: 1 });
    await page.getByRole('button', { name: 'Create Page' }).click();
    await expect(page.getByText(/saved successfully/)).toBeVisible({ timeout: 30_000 });
    expect(saves).toBe(2);
    expect(refreshes).toBe(1);
  });

  test('closing the page with unsaved changes asks first', async ({ page }) => {
    await openEditor(page);
    await titleInput(page).fill('Unsaved before close');
    const dialog = page.waitForEvent('dialog');
    await page.close({ runBeforeUnload: true });
    const prompt = await dialog;
    expect(prompt.type()).toBe('beforeunload');
    await prompt.dismiss();
    expect(page.isClosed()).toBe(false);
  });

  test('an unchanged editor leaves without asking', async ({ page }) => {
    await openEditor(page);
    page.on('dialog', prompt => { throw new Error(`unexpected ${prompt.type()} dialog`); });
    await page.getByRole('link', { name: 'Lekton home' }).click();
    await expect(page).toHaveURL(/\/$/);
  });
});
