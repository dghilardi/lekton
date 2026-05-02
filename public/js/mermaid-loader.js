(function () {
  var state = 'idle'; // 'idle' | 'loading' | 'ready'
  var pending = [];
  var mermaidMod = null; // cached module reference so we can re-init on theme change

  function removeSpinners() {
    document.querySelectorAll('.mermaid-spinner').forEach(function (el) {
      el.remove();
    });
  }

  function runPending(mermaid) {
    var nodes = pending.splice(0);
    if (nodes.length === 0) return;
    mermaid.run({ nodes: nodes }).then(removeSpinners).catch(function (err) {
      console.error('[mermaid] render failed:', err);
      nodes.forEach(function (n) { n.removeAttribute('data-mermaid-queued'); });
      removeSpinners();
    });
  }

  function currentTheme() {
    return document.documentElement.getAttribute('data-theme') === 'dark' ? 'dark' : 'default';
  }

  function loadAndRun() {
    state = 'loading';
    import('/js/mermaid.esm.min.mjs').then(function (mod) {
      mermaidMod = mod.default;
      mermaidMod.initialize({ startOnLoad: false, theme: currentTheme() });
      state = 'ready';
      runPending(mermaidMod);
    }).catch(function (err) {
      console.error('[mermaid] failed to load mermaid module:', err);
      state = 'idle';
      pending.forEach(function (n) { n.removeAttribute('data-mermaid-queued'); });
      pending.length = 0;
      removeSpinners();
    });
  }

  // Re-initialize mermaid with the current theme and re-render all diagrams.
  // Each processed node has its original diagram source stored in data-mermaid-source.
  function rerenderAll() {
    if (!mermaidMod) return;
    mermaidMod.initialize({ startOnLoad: false, theme: currentTheme() });
    document.querySelectorAll('pre.mermaid[data-mermaid-source]').forEach(function (node) {
      node.textContent = node.getAttribute('data-mermaid-source');
      node.removeAttribute('data-processed');
      node.removeAttribute('data-mermaid-queued');
    });
    window.renderMermaid();
  }

  window.renderMermaid = function () {
    var nodes = Array.from(
      document.querySelectorAll('pre.mermaid:not([data-processed]):not([data-mermaid-queued])')
    );
    if (nodes.length === 0) return;

    nodes.forEach(function (node) {
      // Persist original diagram source before mermaid replaces the element content with SVG
      if (!node.hasAttribute('data-mermaid-source')) {
        node.setAttribute('data-mermaid-source', node.textContent || '');
      }
      node.setAttribute('data-mermaid-queued', '');
      var spinner = document.createElement('div');
      spinner.className = 'mermaid-spinner flex justify-center py-6';
      spinner.innerHTML = '<span class="loading loading-spinner loading-md text-primary"></span>';
      node.insertAdjacentElement('beforebegin', spinner);
      pending.push(node);
    });

    if (state === 'ready') {
      import('/js/mermaid.esm.min.mjs').then(function (mod) { runPending(mod.default); });
    } else if (state === 'idle') {
      loadAndRun();
    }
    // if 'loading': nodes are in pending, processed when import resolves
  };

  // Re-render all diagrams whenever the user switches theme
  new MutationObserver(function (mutations) {
    mutations.forEach(function (mutation) {
      if (mutation.attributeName === 'data-theme') {
        rerenderAll();
      }
    });
  }).observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });

  // Safety net: render any pre.mermaid elements that are added to the DOM after this
  // script runs (e.g. injected via Leptos inner_html during hydration).
  new MutationObserver(function (mutations) {
    var hasNewMermaid = false;
    for (var i = 0; i < mutations.length; i++) {
      var added = mutations[i].addedNodes;
      for (var j = 0; j < added.length; j++) {
        var node = added[j];
        if (node.nodeType === 1) {
          if (
            (node.tagName === 'PRE' && node.classList.contains('mermaid')) ||
            node.querySelector('pre.mermaid')
          ) {
            hasNewMermaid = true;
            break;
          }
        }
      }
      if (hasNewMermaid) break;
    }
    if (hasNewMermaid) {
      window.renderMermaid();
    }
  }).observe(document.documentElement, { childList: true, subtree: true });
})();
