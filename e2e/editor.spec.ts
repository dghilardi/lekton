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
      page.locator('text=/managed outside the editor/i'),
    ).toBeVisible({ timeout: 30_000 });
  });

  test('edit button hidden for non-admin', async ({ page }) => {
    await loginAsPublic(page);
    await page.goto('/docs/getting-started');
    await expect(page.locator('article')).toBeVisible({ timeout: 30_000 });
    const editButton = page.locator('a', { hasText: 'Edit' });
    await expect(editButton).not.toBeVisible();
  });
});
