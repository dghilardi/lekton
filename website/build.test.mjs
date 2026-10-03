import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { execFileSync } from 'node:child_process';
import { extractTheme } from './theme.mjs';
import { content } from './content.mjs';

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
  }
  assert.equal(await readFile(new URL('./dist/.nojekyll', import.meta.url), 'utf8'), '');
});
