import { test, expect, type Page } from '@playwright/test';

/**
 * Wait for mermaid to finish rendering all diagrams on the page.
 * Mermaid replaces `<pre class="mermaid">` content with an SVG in-place;
 * we wait until at least one SVG appears inside a mermaid container.
 */
async function waitForMermaidSvg(page: Page, timeout = 20_000): Promise<void> {
  await page.waitForFunction(
    () => document.querySelector('.mermaid svg') !== null,
    { timeout },
  );
}

test.describe('Mermaid diagrams', () => {
  test('renders mermaid code block as SVG', async ({ page }) => {
    await page.goto('/docs/mermaid-test');
    await expect(page.locator('article h1', { hasText: 'Mermaid Test' })).toBeVisible({
      timeout: 30_000,
    });

    await waitForMermaidSvg(page);

    const svg = page.locator('.mermaid svg');
    await expect(svg.first()).toBeVisible();
    // The pre element should still have the mermaid class (mermaid renders SVG inside it)
    await expect(page.locator('pre.mermaid')).toBeAttached();
  });

  test('mermaid diagram remains visible after theme toggle', async ({ page }) => {
    await page.goto('/docs/mermaid-test');
    await expect(page.locator('article h1', { hasText: 'Mermaid Test' })).toBeVisible({
      timeout: 30_000,
    });

    await waitForMermaidSvg(page);
    await expect(page.locator('.mermaid svg').first()).toBeVisible();

    // Toggle theme and verify the SVG is still present
    const themeToggle = page.locator('button[aria-label="Toggle theme"]');
    await themeToggle.click();
    await page.waitForTimeout(500);

    await expect(page.locator('.mermaid svg').first()).toBeVisible();
  });

  test('mermaid does not render raw HTML as code block', async ({ page }) => {
    await page.goto('/docs/mermaid-test');
    await expect(page.locator('article h1', { hasText: 'Mermaid Test' })).toBeVisible({
      timeout: 30_000,
    });

    // The surrounding text should be rendered normally
    await expect(page.locator('text=And some text after')).toBeVisible({ timeout: 10_000 });
  });
});
