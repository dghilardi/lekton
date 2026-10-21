import { test, expect } from '@playwright/test';

for (const theme of ['light', 'dark'] as const) {
  for (const customHeader of [false, true]) {
    test(`Markdown table headers have readable contrast in ${theme} with ${customHeader ? 'custom' : 'default'} styles`, async ({ page }) => {
      await page.addInitScript((value) => localStorage.setItem('lekton-theme', value), theme);
      await page.goto('/docs/getting-started');
      await expect(page.locator('article h1', { hasText: 'Getting Started' })).toBeVisible({ timeout: 30_000 });
      await expect(page.locator('html')).toHaveAttribute('data-theme', theme);

      // Plain GFM table HTML. Isolate the CSS regression from the document's
      // fixture content while exercising the portal's actual stylesheets.
      await page.locator('article').evaluate((article) => {
        article.insertAdjacentHTML('beforeend', `
          <table data-testid="markdown-table">
            <thead><tr><th>LINKS</th><th>Description</th></tr></thead>
            <tbody>
              <tr><td><a href="products/products.md"><strong>Products</strong></a></td><td>Products where is implemented CGPP</td></tr>
              <tr><td><a href="udb/udb.md"><strong>UDB</strong></a></td><td>CGPP over RS485 - Ultra Data Bus (UDB)</td></tr>
              <tr><td><a href="#MsgList"><strong>Commands</strong></a></td><td>List of all application layer messages</td></tr>
            </tbody>
          </table>`);
      });

      if (customHeader) {
        // The Comelit theme styles the row group rather than each header cell.
        await page.addStyleTag({ content: `
          html[data-theme="light"] .prose thead { background-color: #1C1F2B; color: #ffffff; }
          html[data-theme="dark"] .prose thead { background-color: #262b3e; color: #D8DFE1; }
        ` });
      }

      const headers = page.getByTestId('markdown-table').locator('th');
      await expect(headers).toHaveText(['LINKS', 'Description']);
      const contrasts = await headers.evaluateAll((cells) => cells.map((cell) => {
        const canvas = document.createElement('canvas');
        canvas.width = canvas.height = 1;
        const ctx = canvas.getContext('2d')!;
        const paint = (color: string) => {
          ctx.fillStyle = color;
          ctx.fillRect(0, 0, 1, 1);
        };
        const luminance = () => {
          const [r, g, b] = Array.from(ctx.getImageData(0, 0, 1, 1).data.slice(0, 3)).map((value) => {
            const channel = value / 255;
            return channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4;
          });
          return 0.2126 * r + 0.7152 * g + 0.0722 * b;
        };
        // Composite transparent backgrounds and text over the actual ancestor
        // surfaces; this also catches the original half-transparent dark text.
        const ancestors: Element[] = [];
        for (let ancestor: Element | null = cell; ancestor; ancestor = ancestor.parentElement) {
          ancestors.unshift(ancestor);
        }
        paint('#ffffff');
        for (const ancestor of ancestors) paint(getComputedStyle(ancestor).backgroundColor);
        const background = luminance();
        paint(getComputedStyle(cell).color);
        const foreground = luminance();
        return (Math.max(foreground, background) + 0.05) / (Math.min(foreground, background) + 0.05);
      }));
      for (const contrast of contrasts) expect(contrast).toBeGreaterThanOrEqual(4.5);
    });
  }
}
