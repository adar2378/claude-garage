/* Shared header behaviour: GitHub star count and the mobile menu. Small on purpose so nothing waits on it. */
(function () {
  var REPO = 'adar2378/claude-garage', KEY = 'cg-stars';
  function fmt(n) { return n >= 1000 ? (n / 1000).toFixed(n >= 10000 ? 0 : 1).replace(/\.0$/, '') + 'k' : String(n); }
  function show(n) {
    document.querySelectorAll('[data-star-count]').forEach(function (el) { el.textContent = fmt(n); el.hidden = false; });
  }
  function stars() {
    var cached = null;
    try { cached = JSON.parse(sessionStorage.getItem(KEY) || 'null'); } catch (e) {}
    if (cached && Date.now() - cached.t < 3600e3) { show(cached.n); return; }
    if (!window.fetch) return;
    fetch('https://api.github.com/repos/' + REPO, { headers: { Accept: 'application/vnd.github+json' } })
      .then(function (r) { return r.ok ? r.json() : null; })
      .then(function (d) {
        if (!d || typeof d.stargazers_count !== 'number') return;
        try { sessionStorage.setItem(KEY, JSON.stringify({ n: d.stargazers_count, t: Date.now() })); } catch (e) {}
        show(d.stargazers_count);
      })
      .catch(function () { /* no number is fine */ });
  }
  var idle = window.requestIdleCallback || function (f) { return setTimeout(f, 300); };
  idle(stars);

  var menu = document.querySelector('[data-menu]');
  if (menu) {
    menu.addEventListener('click', function (e) { if (e.target.closest('a')) menu.open = false; });
    document.addEventListener('click', function (e) { if (menu.open && !menu.contains(e.target)) menu.open = false; });
    document.addEventListener('keydown', function (e) { if (e.key === 'Escape' && menu.open) { menu.open = false; menu.querySelector('summary').focus(); } });
  }
})();
