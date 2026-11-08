export async function load() {
  const path = '/js/schema-viewers.js';
  const url = document.querySelector(`meta[name="lekton-asset"][data-path="${path}"]`)?.content || path;
  const { loadScript } = await import(url);
  return loadScript('/js/tiptap-bundle.min.js', () => !!window.TipTap);
}
