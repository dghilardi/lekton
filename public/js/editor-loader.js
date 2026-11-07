import { loadScript } from './schema-viewers.js';
export function load() {
  return loadScript('/js/tiptap-bundle.min.js', () => !!window.TipTap);
}
