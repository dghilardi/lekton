import { test, expect } from '@playwright/test';

test.describe('Document viewer', () => {
  test('renders markdown content', async ({ page }) => {
    await page.goto('/docs/getting-started');
    // Should render the markdown heading
    await expect(page.locator('h1', { hasText: 'Getting Started' })).toBeVisible();
    // Should render paragraph content
    await expect(page.locator('text=Welcome to')).toBeVisible();
  });

  test('shows tags', async ({ page }) => {
    await page.goto('/docs/getting-started');
    // Tags should be displayed (seeded with 'guide' and 'onboarding')
    await expect(page.locator('text=guide')).toBeVisible();
  });

  test('shows last updated date', async ({ page }) => {
    await page.goto('/docs/getting-started');
    // The document metadata section should show a date
    const dateElement = page.locator('time, [datetime], text=/\\d{4}/');
    await expect(dateElement.first()).toBeVisible();
  });

  test('clicking nav link navigates to document', async ({ page }) => {
    await page.goto('/');
    // Click on a document in the navigation
    await page.click('text=Architecture Overview');
    // Should navigate to the document page
    await expect(page).toHaveURL(/\/docs\/architecture-overview/);
    await expect(page.locator('h1', { hasText: 'Architecture Overview' })).toBeVisible();
  });
});
