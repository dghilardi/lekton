import { test, expect } from '@playwright/test';

/**
 * Regression guard for production asset 404s.
 *
 * The frontend depends on static JS/CSS bundles being present in the served
 * site root. These are staged by scripts/stage-site-assets.sh (shared by the
 * Docker and e2e builds). If a release ships without one of them, the relevant
 * feature silently breaks (e.g. mermaid diagrams render as raw text).
 *
 * This list mirrors TRACKED_ASSETS in src/static_assets.rs — keep them in sync.
 */
const CRITICAL_ASSETS = [
  { path: '/js/mermaid.esm.min.mjs', contentType: /javascript/ },
  { path: '/js/mermaid-loader.js', contentType: /javascript/ },
  { path: '/js/scalar-standalone.js', contentType: /javascript/ },
  { path: '/js/scalar-style.css', contentType: /css/ },
  { path: '/js/asyncapi-standalone.js', contentType: /javascript/ },
  { path: '/js/asyncapi-default.min.css', contentType: /css/ },
  { path: '/js/tiptap-bundle.min.js', contentType: /javascript/ },
];

test.describe('Static assets are served', () => {
  for (const asset of CRITICAL_ASSETS) {
    test(`${asset.path} returns 200`, async ({ request }) => {
      const resp = await request.get(asset.path);
      expect(resp.status(), `${asset.path} must be served from the site root`).toBe(200);
      expect(resp.headers()['content-type'] ?? '').toMatch(asset.contentType);
    });
  }

  test('mermaid esm module resolves its chunk imports', async ({ request }) => {
    // The mermaid entry module dynamically imports chunks from /js/chunks/.
    // A missing chunk dir would let the entry 200 while rendering still fails,
    // so verify the entry references chunks that are actually served.
    const resp = await request.get('/js/mermaid.esm.min.mjs');
    expect(resp.status()).toBe(200);
    const body = await resp.text();
    const match = body.match(/["']\.\/chunks\/mermaid\.esm\.min\/[^"']+\.mjs["']/);
    expect(match, 'mermaid entry should import at least one chunk').not.toBeNull();
    const chunkPath = '/js/chunks/mermaid.esm.min/' + match![0].split('/').pop()!.replace(/["']/g, '');
    const chunkResp = await request.get(chunkPath);
    expect(chunkResp.status(), `${chunkPath} must be served`).toBe(200);
  });
});
