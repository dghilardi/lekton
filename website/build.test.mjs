import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { execFileSync } from 'node:child_process';
import { extractTheme } from './theme.mjs';
import { content } from './content.mjs';
import { createSeo, normalizeSiteUrl, defaultSiteUrl } from './seo.mjs';

const stylesheet = await readFile(new URL('../style/tailwind.css', import.meta.url), 'utf8');

test('website theme follows application tokens without importing Tailwind or DaisyUI', () => {
  const theme = extractTheme(stylesheet);
  assert.match(theme, /\[data-theme="light"\]/);
  assert.match(theme, /\[data-theme="dark"\]/);
  assert.match(theme, /--lekton-font-family:\s*"Plus Jakarta Sans"/);
  assert.doesNotMatch(theme, /@plugin|@import "tailwindcss"/);
  const changed = stylesheet.replace('oklch(50%    0.19  64)', 'oklch(52% 0.17 64)');
  assert.match(extractTheme(changed), /--color-primary: oklch\(52% 0.17 64\);/);
});

test('missing theme tokens fail the build instead of publishing an unrelated fallback theme', () => {
  assert.throws(() => extractTheme(stylesheet.replaceAll('--color-primary:', '--removed-primary:')), /missing --color-primary/);
  assert.throws(() => extractTheme(stylesheet.replaceAll('[data-theme="dark"]', '[data-theme="removed"]')), /missing the dark variant/);
});

test('each locale produces complete static HTML with project-relative assets and locale links', async () => {
  execFileSync(process.execPath, [new URL('./build.mjs', import.meta.url).pathname]);
  for (const lang of ['en', 'it']) {
    const html = await readFile(new URL(lang === 'en' ? './dist/index.html' : './dist/it/index.html', import.meta.url), 'utf8');
    assert.match(html, new RegExp(`<html lang="${lang}">`));
    assert.ok(html.includes(content[lang].headingFirst));
    assert.ok(html.includes(content[lang].requirementsTitle));
    assert.doesNotMatch(html, /\{\{\w+\}\}/);
    assert.doesNotMatch(html, /(?:src|href)="\/assets\//);
    assert.match(html, lang === 'en' ? /href="\.\/it\/"/ : /href="\.\.\/"/);
    assert.match(html, /docker compose up --build/);
    assert.match(html, /docker compose up --build lekton demo-loader/);
    const canonical = lang === 'en' ? defaultSiteUrl : `${defaultSiteUrl}it/`;
    assert.ok(html.includes(`<link rel="canonical" href="${canonical}">`));
    assert.ok(html.includes(`<link rel="alternate" hreflang="en" href="${defaultSiteUrl}">`));
    assert.ok(html.includes(`<link rel="alternate" hreflang="it" href="${defaultSiteUrl}it/">`));
    assert.ok(html.includes(`<link rel="alternate" hreflang="x-default" href="${defaultSiteUrl}">`));
    const structuredData = JSON.parse(html.match(/<script type="application\/ld\+json">(.*?)<\/script>/s)[1]);
    const page = structuredData['@graph'].find(entry => entry['@type'] === 'WebPage');
    assert.equal(page.url, canonical);
    assert.equal(page.inLanguage, lang);
    assert.equal(page.description, content[lang].description);
    assert.ok(html.includes(`<meta property="og:url" content="${canonical}">`));
    assert.ok(html.includes(`<meta property="og:image" content="${defaultSiteUrl}assets/social.png">`));
  }
  assert.equal(await readFile(new URL('./dist/.nojekyll', import.meta.url), 'utf8'), '');
  const sitemap = await readFile(new URL('./dist/sitemap.xml', import.meta.url), 'utf8');
  assert.deepEqual([...sitemap.matchAll(/<loc>(.*?)<\/loc>/g)].map(match => match[1]), [defaultSiteUrl, `${defaultSiteUrl}it/`]);
  assert.doesNotMatch(sitemap, /lastmod|localhost/);
  const socialImage = await readFile(new URL('./dist/assets/social.png', import.meta.url));
  assert.equal(socialImage.subarray(1, 4).toString(), 'PNG');
  assert.equal(socialImage.readUInt32BE(16), 1200);
  assert.equal(socialImage.readUInt32BE(20), 630);
});

test('deployment URL validation prevents ambiguous canonical and sitemap URLs', () => {
  assert.equal(normalizeSiteUrl('https://example.com/project'), 'https://example.com/project/');
  for (const invalid of ['not-a-url', 'http://example.com/', 'https://user:password@example.com/', 'https://example.com/?page=1', 'https://example.com/#section']) {
    assert.throws(() => normalizeSiteUrl(invalid));
  }
});

test('translated JSON-LD stays valid without allowing embedded script termination', () => {
  const title = 'Lekton </script><script>alert("example")</script>';
  const { structuredData } = createSeo('en', { ...content.en, title }, defaultSiteUrl);
  assert.doesNotMatch(structuredData, /<script|<\/script/);
  assert.equal(JSON.parse(structuredData)['@graph'][1].name, title);
});

test('a custom deployment prefix updates metadata and sitemap while assets stay relative', async () => {
  try {
    execFileSync(process.execPath, [new URL('./build.mjs', import.meta.url).pathname, '--site-url', 'https://example.com/portal']);
    const html = await readFile(new URL('./dist/it/index.html', import.meta.url), 'utf8');
    assert.match(html, /rel="canonical" href="https:\/\/example.com\/portal\/it\/"/);
    assert.match(html, /property="og:image" content="https:\/\/example.com\/portal\/assets\/social.png"/);
    assert.match(html, /href="\.\.\/assets\/site.css"/);
    const sitemap = await readFile(new URL('./dist/sitemap.xml', import.meta.url), 'utf8');
    assert.ok(sitemap.includes('<loc>https://example.com/portal/it/</loc>'));
    assert.ok(!sitemap.includes(defaultSiteUrl));
  } finally {
    execFileSync(process.execPath, [new URL('./build.mjs', import.meta.url).pathname]);
  }
});
