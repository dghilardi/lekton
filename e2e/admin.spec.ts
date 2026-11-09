import { test, expect, type Page } from '@playwright/test';

async function signInAsAdmin(page: Page) {
  expect((await page.request.post('/api/auth/login', { data: { username: 'admin', password: 'admin' } })).ok()).toBeTruthy();
}

test.describe('Administration', () => {
  test.beforeEach(async ({ page }) => {
    await signInAsAdmin(page);
    // Destructive actions ask in the page, never through the browser.
    page.on('dialog', dialog => { throw new Error(`unexpected browser ${dialog.type()} dialog`); });
  });

  test('a new service token is read-only unless write access is chosen', async ({ page }) => {
    await page.goto('/admin/tokens');
    const write = page.getByRole('checkbox', { name: /Allow Write Access/ });
    await expect(write).not.toBeChecked();
    await expect(page.getByText('Read only: this token cannot change content.')).toBeVisible();
    await expect(page.getByLabel('Token Name')).toBeVisible();
    await expect(page.getByLabel('Allowed Scopes')).toBeVisible();
  });

  test('deactivating a token asks in a named dialog that can be cancelled', async ({ page }) => {
    await page.goto('/admin/tokens');
    const unique = `e2e-confirm-${Date.now()}`;
    await expect(async () => {
      await page.getByLabel('Token Name').fill(unique);
      // Scopes may not overlap with another token's.
      await page.getByLabel('Allowed Scopes').fill(`${unique}/*`);
      await page.getByRole('button', { name: /Create Service Token/ }).click();
      await expect(page.getByRole('dialog', { name: 'Token Created' })).toBeVisible({ timeout: 2_000 });
    }).toPass({ timeout: 20_000 });
    await page.keyboard.press('Escape');

    const row = page.getByRole('row').filter({ hasText: unique });
    await row.getByRole('button', { name: 'Deactivate' }).click();
    const confirm = page.getByRole('dialog', { name: `Deactivate “${unique}”?` });
    await expect(confirm).toBeVisible();
    await expect(confirm.getByRole('button', { name: 'Cancel' })).toBeFocused();
    await page.keyboard.press('Escape');
    await expect(confirm).toBeHidden();
    await expect(row.getByText('active', { exact: true })).toBeVisible();

    await row.getByRole('button', { name: 'Deactivate' }).click();
    await confirm.getByRole('button', { name: 'Deactivate' }).click();
    await expect(row.getByText('deactivated', { exact: true })).toBeVisible();
  });

  test('a failed list says so instead of leaving an empty panel', async ({ page }) => {
    await page.route('**/api/list_admin_users*', route => route.fulfill({ status: 500, contentType: 'text/plain', body: 'ServerError|Database unavailable' }));
    await page.goto('/admin/users');
    await expect(page.getByRole('alert')).toContainText('Users could not be loaded. Database unavailable');
  });
});
