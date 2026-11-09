import { test, expect, type Page } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';

async function signInAsAdmin(page: Page) {
  expect((await page.request.post('/api/auth/login', { data: { username: 'admin', password: 'admin' } })).ok()).toBeTruthy();
}

async function focusIsInside(page: Page, selector: string) {
  return page.evaluate(sel => !!document.activeElement?.closest(sel), selector);
}

test.describe('dialogs and keyboard focus', () => {
  test.beforeEach(async ({ page }) => signInAsAdmin(page));

  test('the prompt detail is a modal dialog that keeps and returns focus', async ({ page }) => {
    await page.goto('/prompts');
    const opener = page.getByRole('button', { name: 'Incident Review', exact: true });
    await expect(async () => {
      await opener.click();
      await expect(page.getByRole('dialog', { name: 'Incident Review' })).toBeVisible({ timeout: 1_000 });
    }).toPass({ timeout: 20_000 });
    expect(await focusIsInside(page, 'dialog[open]')).toBe(true);
    for (let i = 0; i < 8; i++) {
      await page.keyboard.press('Tab');
      expect(await focusIsInside(page, 'dialog[open]'), `Tab ${i + 1} left the dialog`).toBe(true);
    }
    await page.keyboard.press('Escape');
    await expect(page.getByRole('dialog')).toHaveCount(0);
    await expect(opener).toBeFocused();
  });

  test('the created-token dialog is named and closes with Escape', async ({ page }) => {
    await page.goto('/admin/tokens');
    const name = page.getByPlaceholder('e.g. github-actions-ci');
    await expect(async () => {
      const unique = `e2e-dialog-${Date.now()}`;
      await name.fill(unique);
      // Scopes may not overlap with another token's.
      await page.getByPlaceholder(/docs\/getting-started/).fill(`${unique}/*`);
      await page.getByRole('button', { name: /Create Service Token/ }).click();
      await expect(page.getByRole('dialog', { name: 'Token Created' })).toBeVisible({ timeout: 2_000 });
    }).toPass({ timeout: 20_000 });
    expect(await focusIsInside(page, 'dialog[open]')).toBe(true);
    await page.keyboard.press('Escape');
    await expect(page.getByRole('dialog')).toHaveCount(0);
  });

  test('wide tables and usage tables are keyboard-scrollable regions', async ({ page }) => {
    await page.goto('/docs/e2e-wide-table');
    const region = page.getByRole('region', { name: 'Scrollable table' });
    await expect(region).toBeVisible();
    await expect(region).toHaveAttribute('tabindex', '0');
    for (const path of ['/docs/e2e-wide-table', '/admin/ai-usage']) {
      await page.goto(path);
      await page.waitForLoadState('networkidle');
      const { violations } = await new AxeBuilder({ page }).withRules(['scrollable-region-focusable']).analyze();
      expect(violations.map(v => v.nodes.map(n => n.target.join(' ')).join(', ')), path).toEqual([]);
    }
  });

  test('desktop readers can skip to the section navigation', async ({ page }) => {
    await page.setViewportSize({ width: 1280, height: 800 });
    await page.goto('/docs/getting-started');
    await expect(page.getByRole('navigation', { name: 'Section navigation' })).toBeVisible();
    const skip = page.getByRole('link', { name: 'Skip to section navigation' });
    await expect(async () => {
      await page.keyboard.press('Tab');
      await expect(skip).toBeFocused({ timeout: 500 });
    }).toPass({ timeout: 10_000 });
    await page.keyboard.press('Enter');
    await expect(page.getByRole('navigation', { name: 'Section navigation' })).toBeFocused();
    await page.keyboard.press('Tab');
    expect(await focusIsInside(page, '#portal-navigation')).toBe(true);
  });
});
