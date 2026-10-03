import { readFile, writeFile, mkdir, rm, copyFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';
import { content } from './content.mjs';
import { extractTheme } from './theme.mjs';

const root = fileURLToPath(new URL('../', import.meta.url));
const destination = resolve(root, 'website/dist');
const template = await readFile(new URL('./index.html', import.meta.url), 'utf8');
const theme = extractTheme(await readFile(resolve(root, 'style/tailwind.css'), 'utf8'));
const keys = Object.keys(content.en).sort().join('\n');
const escape = value => String(value).replace(/[&<>"']/g, char => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[char]);

await rm(destination, { recursive: true, force: true });
await mkdir(resolve(destination, 'assets'), { recursive: true });
await writeFile(resolve(destination, 'assets/theme.css'), theme);
for (const name of ['site.css', 'site.js', 'theme-init.js', 'favicon.svg']) {
  await copyFile(resolve(root, 'website', name), resolve(destination, 'assets', name));
}
for (const [lang, translations] of Object.entries(content)) {
  if (Object.keys(translations).sort().join('\n') !== keys) throw new Error(`Translation keys differ for ${lang}`);
  const values = {
    ...translations, lang, ogLocale: lang === 'en' ? 'en_US' : 'it_IT', assetPrefix: lang === 'en' ? './' : '../',
    languageHref: lang === 'en' ? './it/' : '../', alternateLang: lang === 'en' ? 'it' : 'en',
    languageText: lang === 'en' ? 'IT' : 'EN', currentLanguage: lang.toUpperCase(),
    languageTitle: lang === 'en' ? 'Leggi in italiano' : 'Read in English',
    englishHref: lang === 'en' ? './' : '../', italianHref: lang === 'en' ? './it/' : './',
  };
  const html = template.replace(/\{\{(\w+)\}\}/g, (_, key) => {
    if (!(key in values)) throw new Error(`Missing template value: ${key}`);
    return escape(values[key]);
  });
  const folder = lang === 'en' ? destination : resolve(destination, lang);
  await mkdir(folder, { recursive: true });
  await writeFile(resolve(folder, 'index.html'), html);
}
await writeFile(resolve(destination, '.nojekyll'), '');
console.log('Built English and Italian website with the application theme → website/dist');
