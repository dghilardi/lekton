import { test, expect } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';

// Warning badges, faint metadata, the AsyncAPI viewer's palette and small
// uppercase labels fell below WCAG AA in one or both themes.
const pages = ['/docs/getting-started', '/schemas/user-api', '/schemas/event-api', '/prompts', '/admin/tokens', '/profile'];

test.describe('text contrast and size', () => {
  test.beforeEach(async ({ page }) => {
    expect((await page.request.post('/api/auth/login', { data: { username: 'admin', password: 'admin' } })).ok()).toBeTruthy();
  });

  for (const theme of ['light', 'dark'] as const) {
    test(`text meets AA contrast in the ${theme} theme`, async ({ page }) => {
      test.setTimeout(120_000);
      await page.addInitScript(value => localStorage.setItem('lekton-theme', value), theme);
      const failures: string[] = [];
      for (const path of pages) {
        await page.goto(path);
        await page.waitForLoadState('networkidle');
        if (path === '/schemas/event-api') {
          await expect(page.locator('#asyncapi-viewer')).toContainText('Event API', { timeout: 20_000 });
        }
        const { violations } = await new AxeBuilder({ page }).withRules(['color-contrast']).analyze();
        for (const node of violations.flatMap(v => v.nodes)) {
          failures.push(`${path}: ${(node.any[0]?.data as { contrastRatio?: number })?.contrastRatio} ${node.target.join(' ')}`);
        }
      }
      expect(failures).toEqual([]);
    });
  }

  test('badges and header links stay legible in size and spacing', async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.goto('/schemas/user-api');
    const badge = page.locator('.badge').first();
    await expect(badge).toBeVisible();
    expect(parseFloat(await badge.evaluate(el => getComputedStyle(el).fontSize))).toBeGreaterThanOrEqual(11);
    const link = page.locator('.lekton-header-link:visible').first();
    expect(parseFloat(await link.evaluate(el => getComputedStyle(el).fontSize))).toBeGreaterThanOrEqual(14);
    expect(await link.evaluate(el => getComputedStyle(el).letterSpacing)).toMatch(/^(normal|0px)$/);
  });
});
