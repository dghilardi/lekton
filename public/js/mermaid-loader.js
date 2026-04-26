(function () {
  var state = 'idle'; // 'idle' | 'loading' | 'ready'
  var pending = [];

  function removeSpinners() {
    document.querySelectorAll('.mermaid-spinner').forEach(function (el) {
      el.remove();
    });
  }

  function runPending(mermaid) {
    var nodes = pending.splice(0);
    if (nodes.length === 0) return;
    mermaid.run({ nodes: nodes }).then(removeSpinners).catch(function () {
      nodes.forEach(function (n) { n.removeAttribute('data-mermaid-queued'); });
      removeSpinners();
    });
  }

  function loadAndRun() {
    state = 'loading';
    var theme = document.documentElement.getAttribute('data-theme') === 'dark' ? 'dark' : 'default';
    import('/js/mermaid.esm.min.mjs').then(function (mod) {
      var mermaid = mod.default;
      mermaid.initialize({ startOnLoad: false, theme: theme });
      state = 'ready';
      runPending(mermaid);
    }).catch(function () {
      state = 'idle';
      pending.forEach(function (n) { n.removeAttribute('data-mermaid-queued'); });
      pending.length = 0;
      removeSpinners();
    });
  }

  window.renderMermaid = function () {
    var nodes = Array.from(
      document.querySelectorAll('pre.mermaid:not([data-processed]):not([data-mermaid-queued])')
    );
    if (nodes.length === 0) return;

    nodes.forEach(function (node) {
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
})();
