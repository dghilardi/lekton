(() => {
  const root = document.documentElement;
  const preference = window.matchMedia('(prefers-color-scheme: dark)');
  const toggle = document.querySelector('[data-theme-toggle]');
  const status = document.querySelector('[data-copy-status]');
  const syncTheme = () => {
    const mode = root.dataset.themeMode;
    root.dataset.theme = mode === 'system' ? (preference.matches ? 'dark' : 'light') : mode;
    toggle.setAttribute('aria-label', toggle.dataset[`${mode}Label`]);
    toggle.title = toggle.dataset[`${mode}Label`];
  };
  toggle.hidden = false;
  toggle.addEventListener('click', () => {
    const modes = ['system', 'light', 'dark'];
    const next = modes[(modes.indexOf(root.dataset.themeMode) + 1) % modes.length];
    root.dataset.themeMode = next;
    try {
      if (next === 'system') localStorage.removeItem('lekton-theme');
      else localStorage.setItem('lekton-theme', next);
    } catch { /* The selected theme still works when storage is unavailable. */ }
    syncTheme();
  });
  preference.addEventListener('change', syncTheme);
  syncTheme();

  for (const button of document.querySelectorAll('[data-copy]')) {
    button.hidden = false;
    button.addEventListener('click', async () => {
      const text = document.getElementById(button.dataset.copy).textContent;
      try {
        await navigator.clipboard.writeText(text);
        status.textContent = status.dataset.success;
      } catch {
        status.textContent = status.dataset.error;
        const selection = window.getSelection();
        const range = document.createRange();
        range.selectNodeContents(document.getElementById(button.dataset.copy));
        selection.removeAllRanges();
        selection.addRange(range);
      }
    });
  }
})();
