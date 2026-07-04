import { test, expect } from '@playwright/test';
import { loginAsAdmin, loginAsPublic } from './helpers/auth';

const PDF_FIXTURE = 'demo/assets/runbook.pdf';

test.describe('Document upload', () => {
  test('admin sees the Upload Document nav entry', async ({ page }) => {
    await loginAsAdmin(page);
    await page.goto('/admin/upload');
    // The admin section header renders once the feature-gated page mounts.
    await expect(
      page.getByRole('heading', { name: /upload document/i }),
    ).toBeVisible({ timeout: 30_000 });
  });

  test('admin can upload a PDF and create a document', async ({ page }) => {
    await loginAsAdmin(page);
    await page.goto('/admin/upload');

    // Unique title per run so the derived slug does not collide on reuse.
    const stamp = Date.now();
    const title = `E2E Upload ${stamp}`;
    const slug = `e2e-upload-${stamp}`;
    const description = 'An end-to-end uploaded document.';

    // Pick the PDF via the dynamically-created file input.
    const [fileChooser] = await Promise.all([
      page.waitForEvent('filechooser'),
      page.getByRole('button', { name: /choose pdf/i }).click(),
    ]);
    await fileChooser.setFiles(PDF_FIXTURE);
    // The selected file name is shown next to the button.
    await expect(page.locator('text=runbook.pdf')).toBeVisible({ timeout: 30_000 });

    await page.getByPlaceholder(/employee handbook/i).fill(title);
    await page.getByPlaceholder(/short description/i).fill(description);
    // Access level select — "public" is seeded.
    await page.locator('select').first().selectOption('public');

    await page.getByRole('button', { name: /create document/i }).click();

    // Success banner with a link to the created page.
    const openLink = page.getByRole('link', { name: /open it/i });
    await expect(openLink).toBeVisible({ timeout: 30_000 });
    await expect(openLink).toHaveAttribute('href', `/docs/${slug}`);

    // The created page shows the upload-specific PDF layout with the summary
    // and at least one affordance to open/download the uploaded file.
    await page.goto(`/docs/${slug}`);
    await expect(
      page.getByRole('heading', { level: 1, name: title }),
    ).toBeVisible({ timeout: 30_000 });
    await expect(page.getByText(description)).toBeVisible({ timeout: 30_000 });
    const download = page.locator('a[href*="/api/v1/assets/"]');
    await expect(download.first()).toBeVisible();

    // An admin gets an Edit affordance that points back to the upload form.
    const editButton = page.locator(`a[href="/admin/upload?edit=${slug}"]`);
    await expect(editButton).toBeVisible();
  });

  test('non-admin cannot reach the upload form', async ({ page }) => {
    await loginAsPublic(page);
    await page.goto('/admin/upload');
    // The admin route guard keeps the form out of reach for non-admins.
    await expect(
      page.getByRole('heading', { name: /upload document/i }),
    ).not.toBeVisible({ timeout: 10_000 });
  });
});
