(function () {
  var state = 'idle'; // 'idle' | 'loading' | 'ready'
  var pending = [];
  var mermaidMod = null; // cached module reference so we can re-init on theme change
  // Renders and theme re-renders run one after another: a theme change that
  // resets a diagram while Mermaid is still rendering it makes the render fail
  // and leaves the raw source on the page.
  var rendering = Promise.resolve();

  function removeSpinners() {
    document.querySelectorAll('.mermaid-spinner').forEach(function (el) {
      el.remove();
    });
  }

  function runPending(mermaid) {
    var batch = pending.splice(0);
    if (batch.length === 0) return;

    rendering = rendering.then(function () {
      // A theme change may have queued a diagram twice, or rendered it
      // already; rendering it again would wipe the SVG. Diagrams replaced
      // in the meantime (e.g. by hydration) are rendered via their new node.
      var nodes = batch.filter(function (node, i) {
        return batch.indexOf(node) === i && node.isConnected &&
          !node.hasAttribute('data-processed');
      });
      if (nodes.length === 0) return removeSpinners();

      // Other progressive enhancements may touch <pre> elements while the
      // Mermaid module is loading. Render only the source captured when the
      // diagram was queued, never the node's potentially mutated live content.
      nodes.forEach(function (node) {
        var source = node.getAttribute('data-mermaid-source');
        if (source !== null) {
          node.textContent = node.getAttribute('data-mermaid-surface') === 'light'
            ? withLightTheme(source)
            : source;
        }
      });

      return mermaid.run({ nodes: nodes }).then(function () {
        removeSpinners();
        addExpandButtons(nodes);
      }).catch(function (err) {
        console.error('[mermaid] render failed:', err);
        nodes.forEach(function (n) { n.removeAttribute('data-mermaid-queued'); });
        removeSpinners();
      });
    });
  }

  // ── Expanded view ──────────────────────────────────────────────────
  // Wide diagrams are scaled down to fit the content column. The expand
  // button opens a copy in a full-screen dialog with zoom and pan.
  var EXPAND_ICON =
    '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">' +
    '<polyline points="15 3 21 3 21 9"></polyline><polyline points="9 21 3 21 3 15"></polyline>' +
    '<line x1="21" y1="3" x2="14" y2="10"></line><line x1="3" y1="21" x2="10" y2="14"></line>' +
    '</svg>';
  var viewer = null;

  function addExpandButtons(nodes) {
    nodes.forEach(function (node) {
      if (!node.querySelector('svg') || node.querySelector('.mermaid-expand-btn')) return;
      var btn = document.createElement('button');
      btn.type = 'button';
      btn.className = 'mermaid-expand-btn';
      btn.setAttribute('aria-label', 'Expand diagram');
      btn.title = 'Expand diagram';
      btn.innerHTML = EXPAND_ICON;
      btn.addEventListener('click', function () {
        (viewer || (viewer = createViewer())).open(node);
      });
      node.appendChild(btn);
    });
  }

  function createViewer() {
    var dialog = document.createElement('dialog');
    dialog.className = 'mermaid-viewer';
    dialog.setAttribute('aria-label', 'Diagram');
    dialog.innerHTML =
      '<div class="mermaid-viewer-toolbar">' +
      '<button type="button" data-action="in" aria-label="Zoom in">+</button>' +
      '<button type="button" data-action="out" aria-label="Zoom out">&minus;</button>' +
      '<button type="button" data-action="fit" aria-label="Fit to screen">Fit</button>' +
      '<button type="button" data-action="close" aria-label="Close">&times;</button>' +
      '</div>' +
      '<div class="mermaid-viewer-stage"><div class="mermaid-viewer-canvas"></div></div>';
    var stage = dialog.querySelector('.mermaid-viewer-stage');
    var canvas = dialog.querySelector('.mermaid-viewer-canvas');
    var view = { scale: 1, x: 0, y: 0, w: 1, h: 1 };
    var drag = null;

    function apply() {
      canvas.style.transform =
        'translate(' + view.x + 'px, ' + view.y + 'px) scale(' + view.scale + ')';
    }

    function fit() {
      var r = stage.getBoundingClientRect();
      // Fill the screen, but don't blow small diagrams up past 2x.
      view.scale = Math.min(r.width / view.w, r.height / view.h, 2 / 0.95) * 0.95;
      view.x = (r.width - view.w * view.scale) / 2;
      view.y = (r.height - view.h * view.scale) / 2;
      apply();
    }

    // Zoom by `factor` keeping the stage point (px, py) still; defaults to
    // the stage centre.
    function zoom(factor, px, py) {
      var r = stage.getBoundingClientRect();
      if (px === undefined) {
        px = r.width / 2;
        py = r.height / 2;
      }
      var next = Math.min(Math.max(view.scale * factor, 0.1), 20);
      view.x = px - (px - view.x) * (next / view.scale);
      view.y = py - (py - view.y) * (next / view.scale);
      view.scale = next;
      apply();
    }

    dialog.querySelector('.mermaid-viewer-toolbar').addEventListener('click', function (e) {
      var btn = e.target.closest('button');
      var action = btn && btn.getAttribute('data-action');
      if (action === 'in') zoom(1.25);
      else if (action === 'out') zoom(0.8);
      else if (action === 'fit') fit();
      else if (action === 'close') dialog.close();
    });
    dialog.addEventListener('keydown', function (e) {
      if (e.key === '+' || e.key === '=') zoom(1.25);
      else if (e.key === '-') zoom(0.8);
      else if (e.key === '0') fit();
    });
    stage.addEventListener('wheel', function (e) {
      e.preventDefault();
      var r = stage.getBoundingClientRect();
      zoom(e.deltaY < 0 ? 1.1 : 1 / 1.1, e.clientX - r.left, e.clientY - r.top);
    }, { passive: false });
    stage.addEventListener('pointerdown', function (e) {
      e.preventDefault(); // drag pans the view instead of selecting labels
      drag = { x: e.clientX - view.x, y: e.clientY - view.y };
      stage.setPointerCapture(e.pointerId);
    });
    stage.addEventListener('pointermove', function (e) {
      if (!drag) return;
      view.x = e.clientX - drag.x;
      view.y = e.clientY - drag.y;
      apply();
    });
    stage.addEventListener('pointerup', function () { drag = null; });
    stage.addEventListener('pointercancel', function () { drag = null; });
    // A click on the backdrop targets the dialog itself.
    dialog.addEventListener('click', function (e) {
      if (e.target === dialog) dialog.close();
    });
    dialog.addEventListener('close', function () { canvas.replaceChildren(); });
    document.body.appendChild(dialog);

    return {
      open: function (node) {
        var original = node.querySelector('svg');
        var box = original.viewBox.baseVal;
        var rect = original.getBoundingClientRect();
        var svg = original.cloneNode(true);
        view.w = (box && box.width) || rect.width || 1;
        view.h = (box && box.height) || rect.height || 1;
        svg.style.maxWidth = 'none';
        svg.setAttribute('width', view.w);
        svg.setAttribute('height', view.h);
        canvas.replaceChildren(svg);
        // Keep the diagram on the surface it was rendered for.
        stage.style.backgroundColor = getComputedStyle(node).backgroundColor;
        dialog.showModal();
        fit();
      },
    };
  }

  // Diagrams that set their own colours (classDef, style, linkStyle or theme
  // variables) are written for a light page: their fills rarely set a text
  // colour, so the dark theme's light text becomes unreadable on them. Such
  // diagrams always render with the light theme on a light surface (see the
  // data-mermaid-surface rule in tailwind.css).
  function isAuthorStyled(source) {
    return /^\s*(?:classDef|style|linkStyle)\s/m.test(source) ||
      /themeVariables|["']?theme["']?\s*:/.test(source);
  }

  // Prepend a light-theme directive, after any YAML frontmatter (which must
  // stay first). A theme chosen by the author in a later directive still wins.
  function withLightTheme(source) {
    var directive = '%%{init: {"theme": "redux-color"}}%%\n';
    var frontmatter = source.match(/^\s*---\r?\n[\s\S]*?\r?\n---[ \t]*\r?\n/);
    return frontmatter
      ? frontmatter[0] + directive + source.slice(frontmatter[0].length)
      : directive + source;
  }

  // Mermaid 12's default appearance (neo look, ELK layout) in its light and
  // dark colour themes.
  function currentTheme() {
    return document.documentElement.getAttribute('data-theme') === 'dark'
      ? 'redux-dark-color'
      : 'redux-color';
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
    rendering = rendering.then(function () {
      mermaidMod.initialize({ startOnLoad: false, theme: currentTheme() });
      document.querySelectorAll('pre.mermaid[data-mermaid-source]').forEach(function (node) {
        node.textContent = node.getAttribute('data-mermaid-source');
        node.removeAttribute('data-processed');
        node.removeAttribute('data-mermaid-queued');
      });
      window.renderMermaid();
    });
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
      if (isAuthorStyled(node.getAttribute('data-mermaid-source'))) {
        node.setAttribute('data-mermaid-surface', 'light');
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
