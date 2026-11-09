import { test, expect, type Page } from '@playwright/test';

// Small screens have one navigation menu on every page: the drawer, with the
// portal destinations first and the page's own navigation below them.

async function openDrawer(page: Page) {
  const trigger = page.getByRole('button', { name: 'Open navigation menu' });
  await expect(async () => {
    await trigger.click();
    await expect(trigger).toHaveAttribute('aria-expanded', 'true');
  }).toPass();
  return page.getByRole('navigation', { name: 'Section navigation' });
}

test.describe('Mobile navigation', () => {
  test.use({ viewport: { width: 390, height: 844 } });

  test('a page without a sidebar uses the same drawer, with the destinations and documentation', async ({ page }) => {
    await page.goto('/');
    // The old second menu is gone: one trigger, one menu.
    await expect(page.getByRole('button', { name: 'Navigation menu', exact: true })).toHaveCount(0);
    const drawer = await openDrawer(page);
    await expect(page.getByRole('button', { name: 'Close navigation', exact: true })).toBeFocused();
    await expect(drawer.getByRole('link', { name: 'Home', exact: true })).toBeVisible();
    await expect(drawer.getByRole('link', { name: 'Registry', exact: true })).toBeVisible();
    await expect(drawer.getByRole('link', { name: 'Getting Started', exact: true })).toBeVisible();

    for (const link of await drawer.getByRole('link').all()) {
      if (!(await link.isVisible())) continue;
      const box = await link.boundingBox();
      expect(box!.height, await link.textContent() ?? '').toBeGreaterThanOrEqual(44);
    }

    await drawer.getByRole('link', { name: 'Registry', exact: true }).click();
    await expect(page).toHaveURL(/\/schemas$/);
    await expect(page.getByRole('button', { name: 'Open navigation menu' })).toHaveAttribute('aria-expanded', 'false');
  });

  test('on a documentation page the drawer adds the page tree without repeating the documentation list', async ({ page }) => {
    await page.goto('/docs/api-docs');
    const drawer = await openDrawer(page);
    await expect(drawer.getByRole('link', { name: 'Registry', exact: true })).toBeVisible();
    await expect(drawer.getByText('Authentication API')).toBeVisible();
    await expect(drawer.getByText('Documentation', { exact: true })).toHaveCount(0);
  });

  test('signed-in destinations appear for the people who can use them', async ({ page }) => {
    expect((await page.request.post('/api/auth/login', { data: { username: 'admin', password: 'admin' } })).ok()).toBeTruthy();
    await page.goto('/');
    const drawer = await openDrawer(page);
    await expect(drawer.getByRole('link', { name: 'Admin', exact: true })).toBeVisible();
  });
});

test('large screens keep the header links and show no drawer without a sidebar', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/');
  await expect(page.getByRole('button', { name: 'Open navigation menu' })).toBeHidden();
  await expect(page.getByRole('navigation', { name: 'Section navigation' })).toBeHidden();
  // First in the DOM is the wide-screen tier; the next one is a dropdown.
  await expect(page.getByRole('banner').getByRole('link', { name: 'Getting Started', exact: true }).first()).toBeVisible();
});
