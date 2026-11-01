/* Small interactions that must work the moment the page paints: copy buttons and the install tabs.
   Kept apart from landing.js so they never wait on the hero or the story. */
document.addEventListener('click', function (e) {
  var b = e.target.closest('[data-copy]'); if (!b) return;
  var t = b.getAttribute('data-copy');
  var done = function () {
    var l = b.querySelector('.lbl'); b.classList.add('done');
    if (l) { l.textContent = 'Copied'; } else { b.dataset.o = b.textContent; b.textContent = 'Copied'; }
    setTimeout(function () { b.classList.remove('done'); if (l) { l.textContent = 'Copy'; } else { b.textContent = b.dataset.o || 'Copy'; } }, 1600);
    document.dispatchEvent(new CustomEvent('cg:copied'));
  };
  if (navigator.clipboard && navigator.clipboard.writeText) { navigator.clipboard.writeText(t).then(done, done); }
  else { var ta = document.createElement('textarea'); ta.value = t; document.body.appendChild(ta); ta.select(); try { document.execCommand('copy'); } catch (_) { } ta.remove(); done(); }
});

var tabs = Array.prototype.slice.call(document.querySelectorAll('.tab'));
function selTab(i, focus) {
  tabs.forEach(function (t, k) {
    var on = k === i; t.setAttribute('aria-selected', on); t.tabIndex = on ? 0 : -1;
    document.getElementById(t.getAttribute('aria-controls')).hidden = !on; if (on && focus) t.focus();
  });
}
tabs.forEach(function (t, i) {
  t.addEventListener('click', function () { selTab(i); });
  t.addEventListener('keydown', function (e) {
    if (e.key === 'ArrowRight') selTab((i + 1) % tabs.length, 1);
    if (e.key === 'ArrowLeft') selTab((i + tabs.length - 1) % tabs.length, 1);
  });
});
