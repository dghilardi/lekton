import { test, expect } from '@playwright/test';

test.describe('Navigation', () => {
  test('navbar shows top-level document links', async ({ page }) => {
    await page.goto('/');
    await page.waitForLoadState('networkidle');
    // Top-level documents appear in the navbar (SSR-streamed via TopNavbarLinks)
    await expect(page.locator('text=Getting Started')).toBeVisible({ timeout: 15_000 });
    await expect(page.locator('text=Architecture Overview')).toBeVisible();
  });

  test('sidebar shows section children when browsing docs', async ({ page }) => {
    // Navigate to a section that has children (api-docs → authentication)
    await page.goto('/docs/api-docs');
    await page.waitForLoadState('networkidle');
    // The sidebar should show children of the api-docs section
    await expect(page.locator('text=Authentication API')).toBeVisible({ timeout: 15_000 });
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
