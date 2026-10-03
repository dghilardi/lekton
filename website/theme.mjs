const required = [
  '--lekton-font-family', '--lekton-mono-font',
  '--color-base-100', '--color-base-200', '--color-base-300',
  '--color-base-content', '--color-primary', '--color-primary-content',
  '--radius-field', '--radius-box',
];

// Read the existing application theme. The public website owns no palette.
export function extractTheme(source) {
  const blocks = [...source.matchAll(/([^{}]+)\{([^{}]*)\}/g)]
    .filter(([, , body]) => /--(?:lekton|color|radius)-/.test(body))
    .map(([, selector, body]) => {
      const clean = selector.replace(/\/\*[\s\S]*?\*\//g, '').replace(/@[^;]+;/g, '').trim();
      const declarations = [...body.matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)]
        .filter(([, name]) => required.includes(name))
        .map(([, name, value]) => `${name}: ${value.trim()};`);
      return { selector: clean, declarations };
    })
    .filter(({ selector, declarations }) => declarations.length && /:root|\[data-theme=/.test(selector));
  for (const name of required) {
    if (!blocks.some(({ declarations }) => declarations.some(line => line.startsWith(`${name}:`)))) {
      throw new Error(`Application theme is missing ${name}`);
    }
  }
  for (const mode of ['light', 'dark']) {
    if (!blocks.some(({ selector }) => selector.includes(`[data-theme="${mode}"]`))) {
      throw new Error(`Application theme is missing the ${mode} variant`);
    }
  }
  const fonts = source.match(/@import url\("https:\/\/fonts\.bunny\.net\/[^"\n]+"\);/);
  if (!fonts) throw new Error('Application font import was not found');
  const dark = blocks.find(({ selector }) => selector === '[data-theme="dark"]');
  return `${fonts[0]}\n\n${blocks.map(({ selector, declarations }) => `${selector} {\n  ${declarations.join('\n  ')}\n}`).join('\n\n')}\n\n@media (prefers-color-scheme: dark) {\n  :root:not([data-theme]) {\n    ${dark.declarations.join('\n    ')}\n  }\n}\n`;
}
