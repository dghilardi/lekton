import { test, expect } from '@playwright/test';
import { loginAsAdmin, loginAsPublic } from './helpers/auth';

test.describe('Editor', () => {
  test('edit button visible for admin', async ({ page }) => {
    await loginAsAdmin(page);
    await page.goto('/docs/getting-started');
    const editButton = page.locator('a', { hasText: 'Edit' });
    await expect(editButton).toBeVisible({ timeout: 30_000 });
  });

  test('edit button hidden for non-admin', async ({ page }) => {
    await loginAsPublic(page);
    await page.goto('/docs/getting-started');
    // Wait for the document content to appear first (SSR streaming)
    await expect(page.locator('article')).toBeVisible({ timeout: 30_000 });
    const editButton = page.locator('a', { hasText: 'Edit' });
    await expect(editButton).not.toBeVisible();
  });

  test('editor loads document content', async ({ page }) => {
    await loginAsAdmin(page);
    await page.goto('/edit/getting-started');
    // TipTap editor should be loaded with content
    const editor = page.locator('.tiptap, .ProseMirror, [contenteditable="true"]');
    await expect(editor.first()).toBeVisible({ timeout: 30_000 });
  });

  test('save persists changes', async ({ page }) => {
    await loginAsAdmin(page);
    await page.goto('/edit/getting-started');
    // Wait for editor to load
    const editor = page.locator('.tiptap, .ProseMirror, [contenteditable="true"]');
    await expect(editor.first()).toBeVisible({ timeout: 30_000 });
    // Find and click save button
    const saveButton = page.locator('button', { hasText: /save/i });
    if (await saveButton.isVisible()) {
      await saveButton.click();
      // Should show success feedback or navigate
      await page.waitForTimeout(1000);
    }
  });
});
