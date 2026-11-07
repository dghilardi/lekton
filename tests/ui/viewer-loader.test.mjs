import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadScript, assetUrl } from '../../public/js/schema-viewers.js';

test('viewer asset URLs use the deployment fingerprint', () => {
  globalThis.document = { querySelector: () => ({ content: '/js/viewer.js?v=123' }) };
  assert.equal(assetUrl('/js/viewer.js'), '/js/viewer.js?v=123');
});

test('concurrent viewer requests share one script; loaded viewers make no request', async () => {
  const scripts = [];
  globalThis.document = {
    querySelector: () => null,
    createElement: () => ({}),
    head: { append: script => scripts.push(script) },
  };
  const first = loadScript('/js/test-viewer.js', () => false);
  const second = loadScript('/js/test-viewer.js', () => false);
  assert.equal(first, second);
  assert.equal(scripts.length, 1);
  scripts[0].onload();
  await first;
  await loadScript('/js/test-viewer.js', () => true);
  assert.equal(scripts.length, 1);
});

test('failed script requests can be retried', async () => {
  const scripts = [];
  globalThis.document = { querySelector: () => null, createElement: () => ({ remove() {} }), head: { append: s => scripts.push(s) } };
  const first = loadScript('/js/retry-viewer.js', () => false);
  scripts[0].onerror();
  await assert.rejects(first);
  const retry = loadScript('/js/retry-viewer.js', () => false);
  assert.equal(scripts.length, 2);
  scripts[1].onload();
  await retry;
});
