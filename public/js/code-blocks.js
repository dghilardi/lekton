(function () {
  var COPY_ICON =
    '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">' +
    '<rect x="9" y="9" width="13" height="13" rx="2" ry="2"></rect>' +
    '<path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"></path>' +
    '</svg>';

  var CHECK_ICON =
    '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">' +
    '<polyline points="20 6 9 17 4 12"></polyline>' +
    '</svg>';

  function addCodeBlockFeatures() {
    document.querySelectorAll('.prose pre:not(.mermaid):not([data-cb-init])').forEach(function (pre) {
      pre.setAttribute('data-cb-init', '');

      var code = pre.querySelector('code');

      // Language badge from class="language-xxx"
      var lang = '';
      if (code) {
        var m = code.className.match(/language-(\S+)/);
        if (m) lang = m[1];
      }
      if (lang) {
        var badge = document.createElement('span');
        badge.className = 'code-lang-badge';
        badge.textContent = lang;
        pre.appendChild(badge);
      }

      // Copy button
      var btn = document.createElement('button');
      btn.className = 'code-copy-btn';
      btn.setAttribute('aria-label', 'Copy code');
      btn.innerHTML = COPY_ICON + '<span>copy</span>';

      btn.addEventListener('click', function () {
        var text = code ? code.innerText : pre.innerText;
        navigator.clipboard.writeText(text).then(function () {
          btn.innerHTML = CHECK_ICON + '<span>copied</span>';
          btn.classList.add('copied');
          setTimeout(function () {
            btn.innerHTML = COPY_ICON + '<span>copy</span>';
            btn.classList.remove('copied');
          }, 2000);
        }).catch(function () {
          // fallback: select text
          var sel = window.getSelection();
          var range = document.createRange();
          range.selectNodeContents(code || pre);
          sel.removeAllRanges();
          sel.addRange(range);
        });
      });

      pre.appendChild(btn);
    });
  }

  window.initCodeBlocks = addCodeBlockFeatures;

  // Pick up blocks injected via Leptos inner_html after hydration
  new MutationObserver(function (mutations) {
    for (var i = 0; i < mutations.length; i++) {
      var added = mutations[i].addedNodes;
      for (var j = 0; j < added.length; j++) {
        var node = added[j];
        if (node.nodeType === 1 && (node.querySelector && node.querySelector('.prose pre'))) {
          addCodeBlockFeatures();
          return;
        }
      }
    }
  }).observe(document.documentElement, { childList: true, subtree: true });
})();
