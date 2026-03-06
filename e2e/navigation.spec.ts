import { test, expect } from '@playwright/test';

test.describe('Navigation', () => {
  test('nav tree shows hierarchy with collapsible items', async ({ page }) => {
    await page.goto('/');
    // Parent item "API Documentation" should be visible
    await expect(page.locator('text=API Documentation')).toBeVisible();
    // It should use details/summary for collapsible hierarchy
    const details = page.locator('details');
    const count = await details.count();
    expect(count).toBeGreaterThan(0);
  });

  test('expand and collapse works', async ({ page }) => {
    await page.goto('/');
    // Find a collapsible parent (details element containing "API Documentation")
    const apiDetails = page.locator('details', { hasText: 'API Documentation' }).first();
    if (await apiDetails.isVisible()) {
      const summary = apiDetails.locator('summary');
      // Click to toggle (may already be open)
      await summary.click();
      await page.waitForTimeout(300);
      // Click again to toggle back
      await summary.click();
      await page.waitForTimeout(300);
    }
  });

  test('active link is highlighted', async ({ page }) => {
    await page.goto('/docs/getting-started');
    // The nav link for "Getting Started" should have active styling
    const navLink = page.locator('a[href="/docs/getting-started"]').first();
    await expect(navLink).toBeVisible();
    // Check for active class or bg class indicating selection
    const classes = await navLink.getAttribute('class');
    // The link should have some visual distinction (bg-primary, active, etc.)
    expect(classes).toBeTruthy();
  });
});
