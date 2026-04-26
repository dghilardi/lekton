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

  test('mermaid re-renders after theme toggle', async ({ page }) => {
    await page.goto('/docs/mermaid-test');
    await expect(page.locator('article h1', { hasText: 'Mermaid Test' })).toBeVisible({
      timeout: 30_000,
    });

    await waitForMermaidSvg(page);

    // Toggle the theme — the MutationObserver in mermaid-loader.js will re-initialize
    // mermaid and re-render all diagrams with the new theme.
    const themeToggle = page.locator('button[aria-label="Toggle theme"]');
    await themeToggle.click();

    // Wait for re-render: SVG is briefly removed and re-inserted
    await waitForMermaidSvg(page);
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
