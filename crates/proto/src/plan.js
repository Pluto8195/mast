(function () {
  var root = document.getElementById('pl-root');
  var $$ = function (s, r) { return Array.prototype.slice.call((r || root).querySelectorAll(s)); };
  $$('.pl-tabs button').forEach(function (b) {
    b.addEventListener('click', function () {
      $$('.pl-tabs button').forEach(function (x) { x.classList.toggle('on', x === b); });
      $$('.pl-pane').forEach(function (p) { p.hidden = p.getAttribute('data-pane') !== b.getAttribute('data-tab'); });
    });
  });
  var tiles = $$('.pl-tile');
  function select(id) {
    var sel = tiles.filter(function (t) { return t.getAttribute('data-id') === id; })[0];
    var deps = (sel.getAttribute('data-deps') || '').split(',').filter(Boolean);
    tiles.forEach(function (t) {
      var tid = t.getAttribute('data-id');
      var needs = deps.indexOf(tid) >= 0;
      var unblocks = (t.getAttribute('data-deps') || '').split(',').indexOf(id) >= 0;
      t.classList.toggle('on', tid === id);
      t.classList.toggle('up', needs);
      t.classList.toggle('down', unblocks);
      t.querySelector('.pl-tl').textContent = needs ? 'needs' : unblocks ? 'unblocks' : '';
    });
    $$('.pl-ticket').forEach(function (p) { p.hidden = p.getAttribute('data-id') !== id; });
  }
  tiles.forEach(function (t) { t.addEventListener('click', function () { select(t.getAttribute('data-id')); }); });
  $$('.pl-dep[data-go]').forEach(function (b) { b.addEventListener('click', function () { select(b.getAttribute('data-go')); }); });
  $$('.pl-row[data-row]').forEach(function (b) {
    b.addEventListener('click', function () {
      var p = root.querySelector('.pl-rowpanel[data-for="' + b.getAttribute('data-t') + b.getAttribute('data-row') + '"]');
      p.hidden = !p.hidden;
      b.classList.toggle('on', !p.hidden);
    });
  });
  $$('.pl-rail a').forEach(function (a) {
    a.addEventListener('click', function () {
      var el = document.getElementById(a.getAttribute('data-to'));
      if (el) el.scrollIntoView({ behavior: 'smooth', block: 'start' });
      $$('.pl-rail a').forEach(function (x) { x.classList.toggle('on', x === a); });
    });
  });
  if (tiles.length) select(tiles[0].getAttribute('data-id'));
})();
