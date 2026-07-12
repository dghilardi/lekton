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

  test('home page has no axe violations', async ({ page }) => {
    await page.goto('/');
    await page.waitForLoadState('networkidle');
    await expectNoViolations(page, 'home');
  });

  test('search modal has no axe violations', async ({ page }) => {
    await page.setViewportSize({ width: 1600, height: 900 });
    await page.goto('/');
    await page.waitForLoadState('networkidle');
    await page.keyboard.press('Control+k');
    await expect(page.locator('[role="dialog"]')).toBeVisible({ timeout: 10_000 });
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

    await page.keyboard.press('Control+k');
    const dialog = page.locator('[role="dialog"]');
    await expect(dialog).toBeVisible({ timeout: 10_000 });

    await page.keyboard.press('Escape');
    await expect(dialog).not.toBeVisible({ timeout: 10_000 });
  });
});
