(() => {
  const root = document.documentElement;
  const preference = window.matchMedia('(prefers-color-scheme: dark)');
  let saved;
  try { saved = localStorage.getItem('lekton-theme'); } catch { /* Storage may be blocked. */ }
  const mode = ['light', 'dark'].includes(saved) ? saved : 'system';
  root.dataset.themeMode = mode;
  root.dataset.theme = mode === 'system' ? (preference.matches ? 'dark' : 'light') : mode;
})();
