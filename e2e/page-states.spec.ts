import { test, expect, type Page } from '@playwright/test';

// Missing, restricted and failed pages share one presentation, and signing in
// from one of them comes back to it.

async function signIn(page: Page, username: string) {
  await page.waitForLoadState('load');
  await page.fill('#login-username', username);
  await page.fill('#login-password', username);
  await page.click('button[type="submit"]');
}

test.describe('Page states', () => {
  test('a restricted document reads as not found and sign-in returns to it', async ({ page }) => {
    const response = await page.goto('/docs/internal-processes');
    expect(response?.status()).toBe(404);
    await expect(page.getByRole('heading', { level: 1, name: 'Page not found' })).toBeVisible();
    await expect(page.locator('main')).toContainText('only visible after signing in');

    const signInLink = page.getByRole('main').getByRole('link', { name: 'Sign in' });
    await expect(signInLink).toHaveAttribute('href', '/login?next=/docs/internal-processes');
    await signInLink.click();
    await signIn(page, 'admin');
    await expect(page).toHaveURL(/\/docs\/internal-processes$/, { timeout: 30_000 });
    await expect(page.getByRole('heading', { level: 1, name: 'Internal Processes' })).toBeVisible();
  });

  test('a signed-in reader is not offered sign-in on a missing page', async ({ page }) => {
    expect((await page.request.post('/api/auth/login', { data: { username: 'admin', password: 'admin' } })).ok()).toBeTruthy();
    const response = await page.goto('/docs/no-such-page');
    expect(response?.status()).toBe(404);
    await expect(page.locator('main')).toContainText('your account cannot see it');
    await expect(page.getByRole('main').getByRole('link', { name: 'Sign in' })).toHaveCount(0);
  });

  test('the editor asks anonymous readers to sign in, without framework errors', async ({ page }) => {
    await page.goto('/edit/some-page');
    await expect(page.getByRole('heading', { level: 1, name: 'Sign in to continue' })).toBeVisible({ timeout: 30_000 });
    await expect(page.getByRole('main').getByRole('link', { name: 'Sign in' })).toHaveAttribute('href', '/login?next=/edit/some-page');
    await expect(page.locator('main')).not.toContainText('server function');
  });

  test('non-admin users are refused the editor and administration plainly', async ({ page }) => {
    expect((await page.request.post('/api/auth/login', { data: { username: 'demo', password: 'demo' } })).ok()).toBeTruthy();

    const editor = await page.goto('/edit/some-page');
    expect(editor?.status()).toBe(403);
    await expect(page.getByRole('heading', { level: 1, name: "You don't have access to this page" })).toBeVisible();
    await expect(page.locator('main')).toContainText('Editing pages is limited to administrators');

    await page.goto('/admin/users');
    await expect(page.getByRole('heading', { level: 1, name: "You don't have access to this page" })).toBeVisible();
    // The administration menu is not shown to people who cannot use it.
    await expect(page.getByRole('link', { name: 'Service Tokens' })).toHaveCount(0);
  });

  test('administration asks anonymous readers to sign in and returns there', async ({ page }) => {
    await page.goto('/admin/users');
    await expect(page.getByRole('heading', { level: 1, name: 'Sign in to continue' })).toBeVisible();
    await expect(page.getByRole('link', { name: 'Service Tokens' })).toHaveCount(0);
    await page.getByRole('main').getByRole('link', { name: 'Sign in' }).click();
    await signIn(page, 'admin');
    await expect(page).toHaveURL(/\/admin\/users$/, { timeout: 30_000 });
  });

  test('pages that need an account redirect to sign-in with the way back', async ({ page }) => {
    await page.goto('/profile');
    await expect(page).toHaveURL(/\/login\?next=\/profile$/, { timeout: 30_000 });
    await signIn(page, 'demo');
    await expect(page).toHaveURL(/\/profile$/, { timeout: 30_000 });
  });

  test('sign-in never returns to another site', async ({ page }) => {
    for (const next of ['//evil.example/', 'https://evil.example/', '/\\evil.example']) {
      await page.context().clearCookies();
      await page.goto(`/login?next=${encodeURIComponent(next)}`);
      await signIn(page, 'demo');
      await expect(page).toHaveURL('/', { timeout: 30_000 });
    }
  });
});

test('a missing schema shows the not-found state with status 404', async ({ page }) => {
  const response = await page.goto('/schemas/no-such-schema');
  expect(response?.status()).toBe(404);
  await expect(page.getByRole('heading', { level: 1, name: 'Page not found' })).toBeVisible();
  await expect(page.locator('main#content')).not.toContainText('Not found:');
});
