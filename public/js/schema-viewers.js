// Third-party bundles are loaded only for an interactive schema, once per document.
const scripts = new Map();
export function assetUrl(path) {
  return document.querySelector(`meta[name="lekton-asset"][data-path="${path}"]`)?.content || path;
}
export function loadScript(path, ready) {
  if (ready()) return Promise.resolve();
  const url = assetUrl(path);
  if (!scripts.has(url)) {
    scripts.set(url, new Promise((resolve, reject) => {
      const script = document.createElement('script');
      script.src = url;
      script.onload = resolve;
      script.onerror = () => { scripts.delete(url); script.remove(); reject(new Error('Viewer library could not be loaded')); };
      document.head.append(script);
    }));
  }
  return scripts.get(url);
}
function loadStyle(path) {
  const url = assetUrl(path);
  if ([...document.querySelectorAll('link[rel="stylesheet"]')].some(l => l.getAttribute('href') === url)) return;
  const link = document.createElement('link');
  link.rel = 'stylesheet'; link.href = url; document.head.append(link);
}
export async function mount(kind, id, content) {
  const container = document.getElementById(id);
  if (!container) return;
  container.__lektonDispose?.();
  let disposed = false, instance, themeObserver, errorObserver, linkObserver;
  const dispose = () => {
    disposed = true;
    themeObserver?.disconnect(); errorObserver?.disconnect(); linkObserver?.disconnect();
    instance?.destroy?.();
  };
  container.__lektonDispose = dispose;
  container.replaceChildren();
  const status = document.createElement('p');
  status.setAttribute('role', 'status');
  status.className = 'p-4 text-base-content/70';
  status.textContent = 'Loading interactive schema…';
  container.append(status);
  const fail = message => {
    if (disposed) return;
    status.textContent = message;
    status.className = 'p-4 border border-warning rounded-box';
    status.setAttribute('role', 'alert');
    container.prepend(status);
    const source = container.parentElement.querySelector(':scope > details');
    if (source) source.open = true;
  };
  try {
    if (kind === 'openapi') {
      loadStyle('/js/scalar-style.css');
      await loadScript('/js/scalar-standalone.js', () => !!window.Scalar?.createApiReference);
      if (disposed || !container.isConnected) return;
      status.remove();
      const target = document.createElement('div'); container.append(target);
      const config = () => ({
        content, theme: 'none', showSidebar: false, withDefaultFonts: false,
        showDeveloperTools: 'never', hideDarkModeToggle: true, agent: { disabled: true },
        forceDarkModeState: document.documentElement.dataset.theme === 'dark' ? 'dark' : 'light',
      });
      instance = window.Scalar.createApiReference(target, config());
      // Scalar's operation links have click handlers but no href. Preserve
      // those handlers while making the same operation navigation keyboard usable.
      const enhanced = new WeakSet();
      const enhanceLinks = () => target.querySelectorAll('a.endpoint:not([href])').forEach(link => {
        if (enhanced.has(link)) return;
        enhanced.add(link); link.tabIndex = 0; link.setAttribute('role', 'button');
        link.addEventListener('keydown', event => {
          if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); link.click(); }
        });
      });
      linkObserver = new MutationObserver(enhanceLinks);
      linkObserver.observe(target, { childList: true, subtree: true });
      enhanceLinks();
      themeObserver = new MutationObserver(() => instance.updateConfiguration(config()));
      themeObserver.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
    } else {
      loadStyle('/js/asyncapi-default.min.css');
      await loadScript('/js/asyncapi-standalone.js', () => !!window.AsyncApiStandalone);
      if (disposed || !container.isConnected) return;
      status.remove();
      const target = document.createElement('div'); container.append(target);
      // The standalone viewer renders parser errors asynchronously. Keep its
      // diagnostics, but explain the recovery and expose the untouched source.
      errorObserver = new MutationObserver(() => {
        if (target.textContent.includes('There are errors in your Asyncapi document')) {
          errorObserver.disconnect();
          fail('This specification could not be rendered. Review the validation details below or open the original schema source.');
        }
      });
      errorObserver.observe(target, { childList: true, subtree: true });
      // In the portal's content width the viewer collapses its navigation
      // behind a clickable div. Give that toggle button semantics and keys.
      const enhanceToggle = () => {
        const toggle = target.querySelector('.burger-menu');
        if (!toggle) return;
        const sidebar = target.querySelector('.sidebar');
        const expanded = String(!!sidebar && !sidebar.classList.contains('hidden'));
        if (toggle.getAttribute('aria-expanded') !== expanded) toggle.setAttribute('aria-expanded', expanded);
        if (toggle.hasAttribute('role')) return;
        toggle.setAttribute('role', 'button'); toggle.tabIndex = 0;
        toggle.setAttribute('aria-label', 'Schema navigation');
        toggle.addEventListener('keydown', event => {
          if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); toggle.click(); }
        });
      };
      linkObserver = new MutationObserver(enhanceToggle);
      linkObserver.observe(target, { childList: true, subtree: true, attributes: true, attributeFilter: ['class'] });
      window.AsyncApiStandalone.render({ schema: content, config: { show: { sidebar: true } } }, target);
    }
  } catch (error) {
    console.error('Schema viewer failed', error);
    fail('The interactive viewer is unavailable. You can read the original schema source below.');
  }
}
