import { test, expect, type Page } from '@playwright/test';

/**
 * Wait for Mermaid to finish rendering every diagram on the page.
 * Mermaid also returns an SVG for parse failures, so the error marker and
 * fallback text must be absent before a render counts as successful.
 */
async function waitForMermaidSvg(page: Page, timeout = 30_000): Promise<void> {
  await page.waitForFunction(
    () => {
      const diagrams = Array.from(document.querySelectorAll('pre.mermaid'));
      return diagrams.length > 0 && diagrams.every((diagram) => (
        diagram.querySelector('svg') !== null
        && diagram.querySelector('.error-icon') === null
        && !diagram.textContent?.includes('Syntax error in text')
      ));
    },
    undefined,
    { timeout },
  );
}

/** Open the Mermaid test page with the given theme already applied. */
async function openMermaidPage(page: Page, theme: 'light' | 'dark'): Promise<void> {
  await page.addInitScript((t) => localStorage.setItem('lekton-theme', t), theme);
  await page.goto('/docs/mermaid-test');
  await expect(page.locator('article h1', { hasText: 'Mermaid Test' })).toBeVisible({
    timeout: 30_000,
  });
  await waitForMermaidSvg(page);
}

/**
 * Relative luminance (0 = black, 1 = white) of an element's computed colour
 * (background by default). The colour is resolved through a canvas so
 * oklch() values compare like rgb().
 */
async function colourLuminance(
  page: Page,
  selector: string,
  property: 'backgroundColor' | 'color' = 'backgroundColor',
): Promise<number> {
  return page.locator(selector).first().evaluate((el, prop) => {
    const ctx = document.createElement('canvas').getContext('2d')!;
    ctx.fillStyle = getComputedStyle(el)[prop];
    ctx.fillRect(0, 0, 1, 1);
    const [r, g, b] = Array.from(ctx.getImageData(0, 0, 1, 1).data.slice(0, 3)).map((v) => {
      const c = v / 255;
      return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
    });
    return 0.2126 * r + 0.7152 * g + 0.0722 * b;
  }, property);
}

test.describe('Mermaid diagrams', () => {
  test('diagrams keep the page surface when custom CSS darkens code blocks', async ({ page }) => {
    await openMermaidPage(page, 'light');
    // Same override as a corporate theme that paints every code block dark.
    await page.addStyleTag({
      content: 'html[data-theme="light"] .prose pre { background-color: #151820; }',
    });

    expect(await colourLuminance(page, 'pre:not(.mermaid)')).toBeLessThan(0.05);
    expect(await colourLuminance(page, 'pre.mermaid')).toBeGreaterThan(0.8);
  });

  test('renders mermaid code block as SVG', async ({ page }) => {
    test.setTimeout(90_000);

    // Keep the dynamic import pending long enough for code-block enhancements
    // to initialize. This deterministically exercises the first-load race that
    // used to inject the Copy button markup into Mermaid's parser input.
    await page.route('**/js/mermaid.esm.min.mjs', async (route) => {
      await new Promise((resolve) => setTimeout(resolve, 250));
      await route.continue();
    });

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
    await expect(page.locator('.mermaid .error-icon')).toHaveCount(0);
    await expect(page.getByText('Syntax error in text')).toHaveCount(0);
    expect(consoleErrors.filter((message) => message.includes('[mermaid] render failed'))).toEqual([]);
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
    await themeToggle.scrollIntoViewIfNeeded();
    await themeToggle.click();

    // Wait for re-render: SVG is briefly removed and re-inserted
    await waitForMermaidSvg(page);
    await expect(page.locator('.mermaid svg').first()).toBeVisible();
    await expect(page.locator('.mermaid .error-icon')).toHaveCount(0);
  });

  test('code block enhancements do not mutate mermaid diagrams', async ({ page }) => {
    await page.goto('/docs/mermaid-test');
    await expect(page.locator('article h1', { hasText: 'Mermaid Test' })).toBeVisible({
      timeout: 30_000,
    });

    await waitForMermaidSvg(page);

    // The surrounding text should be rendered normally
    await expect(page.locator('text=And some text after')).toBeVisible({ timeout: 10_000 });
    await expect(page.locator('pre.mermaid .code-copy-btn')).toHaveCount(0);
    await expect(page.locator('pre:not(.mermaid) .code-copy-btn')).toHaveCount(1);
  });
});
