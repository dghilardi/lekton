import { test, expect } from '@playwright/test';
import { loginAsAdmin, loginAsPublic } from './helpers/auth';

// Seeded documents are created through the ingest API (they carry a source_id),
// i.e. they are managed by lekton-sync. Such pages are read-only in the portal:
// editing them in the WYSIWYG editor would be overwritten on the next sync, so
// no edit affordance is offered and the editor refuses to open them.
test.describe('Editor', () => {
  test('managed (synced) pages show no edit button for admin', async ({ page }) => {
    await loginAsAdmin(page);
    await page.goto('/docs/getting-started');
    // Wait for the document content first (SSR streaming).
    await expect(page.locator('article')).toBeVisible({ timeout: 30_000 });
    const editButton = page.locator('a', { hasText: 'Edit' });
    await expect(editButton).not.toBeVisible();
  });

  test('opening the editor on a managed page is blocked', async ({ page }) => {
    await loginAsAdmin(page);
    await page.goto('/edit/getting-started');
    await expect(
      page.getByRole('heading', { level: 1, name: 'This page is published from its source' }),
    ).toBeVisible({ timeout: 30_000 });
    await expect(page.getByRole('link', { name: 'Back to the page' })).toHaveAttribute('href', '/docs/getting-started');
    await expect(page.locator('main')).not.toContainText('Bad request');
  });

  test('edit button hidden for non-admin', async ({ page }) => {
    await loginAsPublic(page);
    await page.goto('/docs/getting-started');
    await expect(page.locator('article')).toBeVisible({ timeout: 30_000 });
    const editButton = page.locator('a', { hasText: 'Edit' });
    await expect(editButton).not.toBeVisible();
  });
});
