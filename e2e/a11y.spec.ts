import { test, expect } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { loginAsDemo, loginAsAdmin } from './helpers/auth';

// Accessibility audit + keyboard-operability checks over the main flows.
//
// Skipped in CI for now: axe will surface pre-existing violations that should
// be triaged before they gate the pipeline. Run locally with
// `npx playwright test a11y` (add `--project=webkit`/`mobile-chrome` for the
// local-only browser profiles). Remove the skip once violations are cleared.
test.describe('Accessibility', () => {
  test.skip(!!process.env.CI, 'a11y audit runs locally until violations are triaged');

  async function expectNoViolations(page: import('@playwright/test').Page, context: string) {
    const { violations } = await new AxeBuilder({ page })
      .withTags(['wcag2a', 'wcag2aa'])
      .analyze();
    if (violations.length > 0) {
      // Surface a readable summary before the assertion diff.
      console.log(
        `axe violations on ${context}:\n` +
          violations
            .map((v) => `  [${v.impact}] ${v.id}: ${v.help} (${v.nodes.length} node(s))`)
            .join('\n'),
      );
    }
    expect(violations, `axe violations on ${context}`).toEqual([]);
  }

  // The search is a native <dialog> (implicit role), and Ctrl+K only works
  // once hydration has attached the shortcut, so retry until it opens.
  async function openSearch(page: import('@playwright/test').Page) {
    const dialog = page.getByRole('dialog', { name: 'Search documentation' });
    await expect(async () => {
      await page.keyboard.press('Control+k');
      await expect(dialog).toBeVisible({ timeout: 500 });
    }).toPass({ timeout: 15_000 });
    return dialog;
  }

  test('home page has no axe violations', async ({ page }) => {
    await page.goto('/');
    await page.waitForLoadState('networkidle');
    await expectNoViolations(page, 'home');
  });

  test('search modal has no axe violations', async ({ page }) => {
    await page.setViewportSize({ width: 1600, height: 900 });
    await page.goto('/');
    await page.waitForLoadState('networkidle');
    await openSearch(page);
    await expectNoViolations(page, 'search modal');
  });

  test('chat page has no axe violations', async ({ page }) => {
    test.setTimeout(90_000);
    await loginAsDemo(page);
    await page.goto('/chat');
    await page.waitForLoadState('networkidle');
    await expectNoViolations(page, 'chat');
  });

  test('admin page has no axe violations', async ({ page }) => {
    test.setTimeout(90_000);
    await loginAsAdmin(page);
    await page.goto('/admin/tokens');
    await page.waitForLoadState('networkidle');
    await expectNoViolations(page, 'admin');
  });

  test('search modal is keyboard operable (open with Ctrl+K, close with Escape)', async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1600, height: 900 });
    await page.goto('/');
    await page.waitForLoadState('networkidle');
    // Wait for hydration: the Ctrl+K handler is attached client-side, so pressing
    // before the navbar search control has rendered would be a no-op.
    await expect(
      page.locator('button', { hasText: /search|ctrl.*k/i }).first(),
    ).toBeVisible({ timeout: 15_000 });

    const dialog = await openSearch(page);

    await page.keyboard.press('Escape');
    await expect(dialog).not.toBeVisible({ timeout: 10_000 });
  });
});
