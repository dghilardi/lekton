import { test, expect } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { mkdir } from 'node:fs/promises';

for (const locale of ['en', 'it']) {
  for (const theme of ['light', 'dark']) {
    test(`${locale}/${theme}: localized static content, shared theme and accessible interface`, async ({ page }) => {
      const errors = [];
      page.on('pageerror', error => errors.push(error.message));
      page.on('response', response => { if (response.url().includes('127.0.0.1') && response.status() >= 400) errors.push(response.url()); });
      await page.addInitScript(mode => localStorage.setItem('lekton-theme', mode), theme);
      await page.goto(locale === 'en' ? '/lekton/' : '/lekton/it/');
      await expect(page.locator('html')).toHaveAttribute('lang', locale);
      await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
      await expect(page.locator('h1')).toContainText(locale === 'en' ? 'Documentation,' : 'La documentazione,');
      await expect(page.locator('h1')).toHaveCount(1);
      const canonical = `https://dghilardi.github.io/lekton/${locale === 'it' ? 'it/' : ''}`;
      await expect(page.locator('link[rel="canonical"]')).toHaveAttribute('href', canonical);
      await expect(page.locator('meta[property="og:url"]')).toHaveAttribute('content', canonical);
      await expect(page.locator('link[hreflang="en"]')).toHaveAttribute('href', 'https://dghilardi.github.io/lekton/');
      await expect(page.locator('link[hreflang="it"]')).toHaveAttribute('href', 'https://dghilardi.github.io/lekton/it/');
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
      const audit = await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21aa']).analyze();
      expect(audit.violations).toEqual([]);
      expect(errors).toEqual([]);
    });
  }
}

test('locale links work under /lekton and content remains usable without JavaScript', async ({ browser, baseURL }) => {
  const context = await browser.newContext({ javaScriptEnabled: false, baseURL });
  const page = await context.newPage();
  await page.goto('/lekton/');
  await page.getByRole('link', { name: 'IT', exact: true }).click();
  await expect(page).toHaveURL(/\/lekton\/it\/$/);
  await expect(page.locator('h1')).toContainText('La documentazione,');
  await page.locator('.workflow summary').click();
  await expect(page.locator('.workflow-expanded')).toBeVisible();
  await context.close();
});

test('crawler and social assets are served under the project prefix', async ({ request }) => {
  const sitemap = await request.get('/lekton/sitemap.xml');
  expect(sitemap.ok()).toBe(true);
  expect(sitemap.headers()['content-type']).toContain('xml');
  expect(await sitemap.text()).toContain('<loc>https://dghilardi.github.io/lekton/it/</loc>');
  const image = await request.get('/lekton/assets/social.png');
  expect(image.ok()).toBe(true);
  expect(image.headers()['content-type']).toBe('image/png');
});

test('theme cycles through light, dark and system and copy uses the displayed commands', async ({ page, context }) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write']);
  await page.emulateMedia({ colorScheme: 'light' });
  await page.goto('/lekton/');
  const toggle = page.locator('[data-theme-toggle]');
  await toggle.click();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
  await toggle.click();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  await page.reload();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  await toggle.click();
  await expect(page.locator('html')).toHaveAttribute('data-theme-mode', 'system');
  await page.emulateMedia({ colorScheme: 'dark' });
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  await page.getByRole('button', { name: 'Copy commands', exact: true }).click();
  await expect(page.getByRole('status')).toContainText('Copied');
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(await page.locator('#setup-commands').textContent());
});

test('Italian mobile layout and expandable content fit narrow screens', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/lekton/it/');
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.locator('.capabilities summary').first().click();
  await expect(page.locator('.capabilities details').first()).toHaveAttribute('open', '');
  await page.locator('.workflow summary').click();
  await expect(page.locator('.workflow-expanded')).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
});

test('release selection is reachable in both locales on desktop and narrow mobile screens', async ({ page }) => {
  for (const locale of ['en', 'it']) {
    await page.goto(locale === 'en' ? '/lekton/' : '/lekton/it/');
    const name = locale === 'en' ? 'Releases' : 'Versioni';
    const headerLink = page.locator('header').getByRole('link', { name, exact: true });
    await expect(headerLink).toHaveAttribute('href', 'https://github.com/dghilardi/lekton/releases');
    await expect(page.locator('footer').getByRole('link', { name, exact: true })).toHaveAttribute('href', 'https://github.com/dghilardi/lekton/releases');
    for (const width of [1440, 390, 320]) {
      await page.setViewportSize({ width, height: 844 });
      await page.evaluate(() => document.fonts.ready);
      await expect(headerLink).toBeVisible();
      const overflow = await page.evaluate(() => [...document.querySelectorAll('body *')]
        .filter(element => element.getBoundingClientRect().right > innerWidth && element.getBoundingClientRect().width)
        .slice(0, 8).map(element => ({ tag: element.tagName, class: element.className, text: element.textContent.slice(0, 60), right: element.getBoundingClientRect().right })));
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `${locale}/${width}: ${JSON.stringify(overflow)}`).toBe(true);
    }
  }
});

test('capture desktop and mobile theme variants for visual verification', async ({ page }) => {
  await mkdir('.impeccable/review', { recursive: true });
  for (const capture of [
    { name: 'hero-repro', width: 1513, height: 1039, theme: 'light', locale: 'en', fullPage: false },
    { name: 'desktop', width: 1440, height: 1000, theme: 'light', locale: 'en', fullPage: true },
    { name: 'desktop-dark', width: 1440, height: 1000, theme: 'dark', locale: 'en', fullPage: true },
    { name: 'mobile', width: 390, height: 844, theme: 'light', locale: 'it', fullPage: true },
    { name: 'mobile-dark', width: 390, height: 844, theme: 'dark', locale: 'it', fullPage: true },
    { name: 'mobile-small', width: 320, height: 844, theme: 'light', locale: 'it', fullPage: true },
    { name: 'user-1280', width: 1280, height: 800, theme: 'dark', locale: 'en', fullPage: true },
    { name: 'desktop-it', width: 1440, height: 1000, theme: 'light', locale: 'it', fullPage: true },
  ]) {
    await page.setViewportSize({ width: capture.width, height: capture.height });
    await page.emulateMedia({ colorScheme: capture.theme, reducedMotion: 'reduce' });
    await page.goto(capture.locale === 'en' ? '/lekton/' : '/lekton/it/');
    await page.evaluate(() => document.fonts.ready);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await page.screenshot({ path: `.impeccable/review/${capture.name}.png`, fullPage: capture.fullPage });
  }
});

test('blocked browser storage and clipboard still leave a usable page', async ({ page }) => {
  await page.addInitScript(() => {
    Storage.prototype.getItem = () => { throw new Error('Storage blocked'); };
    Storage.prototype.setItem = () => { throw new Error('Storage blocked'); };
    Storage.prototype.removeItem = () => { throw new Error('Storage blocked'); };
    Object.defineProperty(navigator, 'clipboard', { value: { writeText: () => Promise.reject(new Error('Clipboard denied')) } });
  });
  await page.goto('/lekton/');
  await page.locator('[data-theme-toggle]').click();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
  await page.getByRole('button', { name: 'Copy commands', exact: true }).click();
  await expect(page.getByRole('status')).toContainText('Copy failed');
  expect(await page.evaluate(() => getSelection().toString())).toBe(await page.locator('#setup-commands').textContent());
});
