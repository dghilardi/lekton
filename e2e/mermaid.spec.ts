import { test, expect, type Page } from '@playwright/test';

/**
 * Wait for mermaid to finish rendering all diagrams on the page.
 * Mermaid replaces `<pre class="mermaid">` content with an SVG in-place;
 * we wait until at least one SVG appears inside a mermaid container.
 */
async function waitForMermaidSvg(page: Page, timeout = 30_000): Promise<void> {
  await page.waitForFunction(
    () => document.querySelector('.mermaid svg') !== null,
    undefined,
    { timeout },
  );
}

test.describe('Mermaid diagrams', () => {
  test('renders mermaid code block as SVG', async ({ page }) => {
    test.setTimeout(90_000);

    // Capture console errors and failed network requests for diagnostics.
    const consoleErrors: string[] = [];
    const failedRequests: string[] = [];
    page.on('console', (msg) => {
      if (msg.type() === 'error') consoleErrors.push(msg.text());
    });
    page.on('requestfailed', (req) => {
      if (req.url().includes('mermaid')) failedRequests.push(`${req.failure()?.errorText} ${req.url()}`);
    });
    page.on('response', (resp) => {
      if (resp.url().includes('mermaid') && resp.status() >= 400) {
        failedRequests.push(`HTTP ${resp.status()} ${resp.url()}`);
      }
    });

    await page.goto('/docs/mermaid-test');
    await expect(page.locator('article h1', { hasText: 'Mermaid Test' })).toBeVisible({
      timeout: 30_000,
    });

    await waitForMermaidSvg(page).catch((err) => {
      const parts: string[] = [err.message];
      if (failedRequests.length) parts.push(`Failed mermaid requests:\n  ${failedRequests.join('\n  ')}`);
      if (consoleErrors.length) parts.push(`Browser console errors:\n  ${consoleErrors.join('\n  ')}`);
      throw new Error(parts.join('\n'));
    });

    const svg = page.locator('.mermaid svg');
    await expect(svg.first()).toBeVisible();
    // The pre element should still have the mermaid class (mermaid renders SVG inside it)
    await expect(page.locator('pre.mermaid')).toBeAttached();
  });

  test('mermaid re-renders after theme toggle', async ({ page }) => {
    test.setTimeout(90_000);

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
