'use strict';
/* FahrenHack UI: one page for the desktop window (console style, gamepad + keyboard) and for phones. */

// ---------------------------------------------------------------- helpers
const $ = (s, el = document) => el.querySelector(s);
const $$ = (s, el = document) => [...el.querySelectorAll(s)];
const esc = s => String(s ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const sleep = ms => new Promise(r => setTimeout(r, ms));
const plural = (n, one, few, many) => {
  const a = Math.abs(n) % 100, b = a % 10;
  return a > 10 && a < 20 ? many : b === 1 ? one : b >= 2 && b <= 4 ? few : many;
};
const fmtNum = v => {
  if (v === null || v === undefined || v === '') return '';
  const s = String(v);
  return /^-?\d+$/.test(s) ? Number(s).toLocaleString('ru-RU') : s;
};
const VTYPES = [['i32', 'Целое, 4 байта'], ['i64', 'Целое, 8 байт'], ['f32', 'Дробное, 4 байта'], ['f64', 'Дробное, 8 байт']];
const vtypeOptions = sel => VTYPES.map(([v, l]) => `<option value="${v}"${v === sel ? ' selected' : ''}>${l}</option>`).join('');
const TABS = ['home', 'scanner', 'remote', 'settings'];
const tabOf = v => (v === 'game' ? 'home' : v);

const Q = new URLSearchParams(location.search);
let TOKEN = Q.get('t') || '';
try {
  if (TOKEN) localStorage.setItem('fh-token', TOKEN);
  else TOKEN = localStorage.getItem('fh-token') || '';
} catch (e) { /* storage can be blocked */ }
const DESKTOP = Q.has('desktop');
document.body.classList.toggle('desktop', DESKTOP);
// Message to the desktop window (drag, min, max, fullscreen, close). wry defines window.ipc
// as a non-configurable global, so no top-level binding in this file may be named `ipc`;
// app.js is also loaded as a module to keep its names off the global object.
const host = msg => { try { if (window.ipc) window.ipc.postMessage(msg); } catch (e) { /* not the desktop window */ } };

const S = { st: null, games: [], view: 'home', params: {}, stack: [], sel: 0, pad: false, online: true, authFail: false };

// ---------------------------------------------------------------- server
async function api(path, method = 'GET', body) {
  const opt = { method, headers: { 'x-token': TOKEN } };
  if (body !== undefined) {
    opt.headers['content-type'] = 'application/json';
    opt.body = JSON.stringify(body);
  }
  const r = await fetch('/api' + path, opt);
  const text = await r.text();
  if (r.status === 401) S.authFail = true;
  if (!r.ok) throw new Error(text || 'Ошибка ' + r.status);
  return text ? JSON.parse(text) : null;
}
const artUrl = (g, kind) => `/api/art/${encodeURIComponent(g)}/${kind}?t=${encodeURIComponent(TOKEN)}`;

/** Starts a server job and waits for it, reporting progress. */
async function runJob(start, onTick) {
  const { job } = await start;
  for (;;) {
    await sleep(170);
    let j = null;
    try { j = await api('/job'); } catch (e) { continue; }
    if (!j || j.id !== job) continue;
    if (onTick) onTick(j);
    if (j.done) {
      if (j.error) throw new Error(j.error);
      return j.result;
    }
  }
}

async function act(fn, okMsg) {
  try {
    const r = await fn();
    if (okMsg) toast(okMsg);
    return r ?? true;
  } catch (e) {
    toast(e.message, true);
    sfx('err');
    return null;
  }
}

function toast(msg, err) {
  const t = $('#toast');
  t.textContent = msg;
  t.className = 'show' + (err ? ' err' : '');
  clearTimeout(t._h);
  t._h = setTimeout(() => { t.className = ''; }, err ? 4500 : 2600);
}

// ---------------------------------------------------------------- sound
let AC = null;
function sfx(kind) {
  if (!S.st || !S.st.settings || !S.st.settings.sounds) return;
  try {
    AC = AC || new (window.AudioContext || window.webkitAudioContext)();
    const tones = { move: [660, 0, .016, .025], ok: [880, 1320, .05, .05], back: [520, 360, .05, .045], on: [740, 1480, .08, .06], off: [700, 350, .08, .05], err: [220, 180, .12, .05] };
    const [f0, f1, dur, vol] = tones[kind] || tones.move;
    const t = AC.currentTime, o = AC.createOscillator(), g = AC.createGain();
    o.type = kind === 'err' ? 'square' : 'sine';
    o.frequency.setValueAtTime(f0, t);
    if (f1) o.frequency.exponentialRampToValueAtTime(f1, t + dur);
    g.gain.setValueAtTime(0.0001, t);
    g.gain.exponentialRampToValueAtTime(vol, t + 0.008);
    g.gain.exponentialRampToValueAtTime(0.0001, t + dur + 0.06);
    o.connect(g).connect(AC.destination);
    o.start(t);
    o.stop(t + dur + 0.08);
  } catch (e) { /* sound is optional */ }
}

// ---------------------------------------------------------------- icons
const ICONS = {
  heart: '<path d="M12 20s-7-4.4-7-10a4 4 0 0 1 7-2.6A4 4 0 0 1 19 10c0 5.6-7 10-7 10z"/>',
  ammo: '<path d="M6 21V9.5a2 2 0 0 1 4 0V21M14 21V9.5a2 2 0 0 1 4 0V21M4 21h16M6 13h4M14 13h4"/>',
  coin: '<circle cx="12" cy="12" r="8"/><path d="M14.5 9.3c-.5-.8-1.4-1.3-2.5-1.3-1.5 0-2.5.8-2.5 1.9 0 2.6 5 1.3 5 4 0 1.1-1 1.9-2.5 1.9-1.1 0-2-.5-2.5-1.3M12 6.5V8M12 16v1.5"/>',
  gem: '<path d="M7 4h10l4 5-9 11L3 9z"/><path d="M3 9h18M12 20 9 9l3-5 3 5z"/>',
  knife: '<path d="M14.5 4.5 20 2l-2.5 5.5L8 17l-3-3z"/><path d="m5 14-2 2 3 3 2-2"/>',
  star: '<path d="m12 3.5 2.6 5.3 5.9.9-4.3 4.1 1 5.8L12 16.9l-5.2 2.7 1-5.8-4.3-4.1 5.9-.9z"/>',
  bolt: '<path d="M13 2 4 14h7l-1 8 9-12h-7z"/>',
  dots: '<circle cx="5" cy="12" r="1.3"/><circle cx="12" cy="12" r="1.3"/><circle cx="19" cy="12" r="1.3"/>',
  play: '<path d="M7 4.5v15l12-7.5z"/>',
  plus: '<path d="M12 5v14M5 12h14"/>',
  games: '<rect x="3" y="7" width="18" height="11" rx="5"/><path d="M8 10.5v4M6 12.5h4M15.5 12h.01M18 14h.01"/>',
  scan: '<circle cx="11" cy="11" r="6.5"/><path d="m16 16 4.5 4.5"/>',
  phone: '<rect x="7" y="2.5" width="10" height="19" rx="2.5"/><path d="M11 18.5h2"/>',
  power: '<path d="M12 3v8M6.3 6.8a8 8 0 1 0 11.4 0"/>',
  check: '<path d="m5 12.5 4.5 4.5L19 7.5"/>',
  folder: '<path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>',
  refresh: '<path d="M20 11a8 8 0 1 0-2.3 5.7M20 4v7h-7"/>',
  wifi: '<path d="M2.5 9a14 14 0 0 1 19 0M5.5 12.5a9.5 9.5 0 0 1 13 0M8.8 16a5 5 0 0 1 6.4 0"/><circle cx="12" cy="19.2" r=".9"/>',
};
const icon = (name, cls = '') => `<svg class="ic ${cls}" viewBox="0 0 24 24" aria-hidden="true">${ICONS[name] || ICONS.star}</svg>`;

// ---------------------------------------------------------------- DOM patching (keeps focus)
function patchHtml(el, html) {
  if (!el || el._html === html) return;
  const a = document.activeElement;
  const key = a && el.contains(a) ? a.dataset.k : null;
  setHtml(el, html);
  if (key) {
    const n = el.querySelector(`[data-k="${CSS.escape(key)}"]`);
    if (n) n.focus({ preventScroll: true });
  }
}

/** Sets innerHTML and remembers it, so later patchHtml calls skip identical content. */
function setHtml(el, html) {
  el.innerHTML = html;
  el._html = html;
}

// ---------------------------------------------------------------- focus & navigation
function focusables(root) {
  const scope = root || (M ? $('#modal') : document);
  return $$('button:not([disabled]):not([tabindex="-1"]), input:not([disabled]), select:not([disabled]), [tabindex="0"]', scope)
    .filter(el => {
      const r = el.getBoundingClientRect();
      return r.width > 0 && r.height > 0 && !el.closest('.hide');
    });
}

function focusEl(el) {
  if (!el) return;
  el.focus({ preventScroll: true });
  el.scrollIntoView({ block: 'nearest', inline: 'nearest', behavior: 'smooth' });
}

/** Spatial navigation for arrows, D-pad and stick. */
function moveFocus(dir) {
  const cur = document.activeElement;
  const list = focusables();
  if (!cur || cur === document.body || !list.includes(cur)) {
    focusEl(list.find(e => $('#view').contains(e)) || list[0]);
    return;
  }
  const a = cur.getBoundingClientRect();
  const ax = a.left + a.width / 2, ay = a.top + a.height / 2;
  let best = null, bestScore = Infinity;
  for (const el of list) {
    if (el === cur) continue;
    const b = el.getBoundingClientRect();
    const bx = b.left + b.width / 2, by = b.top + b.height / 2;
    let gap, cross;
    if (dir === 'left') { if (bx >= ax - 1) continue; gap = a.left - b.right; cross = by - ay; }
    else if (dir === 'right') { if (bx <= ax + 1) continue; gap = b.left - a.right; cross = by - ay; }
    else if (dir === 'up') { if (by >= ay - 1) continue; gap = a.top - b.bottom; cross = bx - ax; }
    else { if (by <= ay + 1) continue; gap = b.top - a.bottom; cross = bx - ax; }
    const score = Math.max(gap, 0) + Math.abs(cross) * 1.8;
    if (score < bestScore) { bestScore = score; best = el; }
  }
  if (best) { focusEl(best); sfx('move'); }
}

function activate() {
  const el = document.activeElement;
  if (!el || el === document.body) { moveFocus('down'); return; }
  if (el.tagName === 'SELECT') {
    el.selectedIndex = (el.selectedIndex + 1) % el.options.length;
    el.dispatchEvent(new Event('change', { bubbles: true }));
    return;
  }
  if (el.tagName === 'INPUT') { el.focus(); return; }
  el.click();
}

function go(view, params = {}) {
  S.stack.push({ view: S.view, params: S.params });
  if (S.stack.length > 30) S.stack.shift();
  show(view, params);
}
function tab(view) { S.stack = []; show(view, {}); sfx('move'); }
function cycleTab(d) {
  const i = Math.max(0, TABS.indexOf(tabOf(S.view)));
  tab(TABS[(i + d + TABS.length) % TABS.length]);
}
function back() {
  if (M) { closeModal(); sfx('back'); return; }
  const p = S.stack.pop();
  if (p) { show(p.view, p.params); sfx('back'); }
  else if (S.view !== 'home' && S.view !== 'auth') { show('home', {}); sfx('back'); }
}
function show(view, params) {
  S.view = view;
  S.params = params || {};
  document.body.dataset.view = view;
  $('#view').innerHTML = views[view].render();
  renderHeader();
  renderHints();
  if (views[view].mount) views[view].mount();
  if (!$('#view').contains(document.activeElement)) focusView();
}
function focusView() {
  const f = $('#view [data-autofocus]') || focusables(document).find(e => $('#view').contains(e));
  if (f) f.focus({ preventScroll: true });
}

// ---------------------------------------------------------------- modal
let M = null;
function openModal(m) {
  M = m;
  const el = $('#modal');
  el.innerHTML = '<div class="mcard"></div>';
  el.classList.remove('hide');
  el.classList.toggle('wide', !!m.wide);
  drawModal();
  sfx('ok');
}
/** Redraws the open modal's content; the card itself stays, so its intro animation plays once. */
function drawModal() {
  if (!M) return;
  const el = $('#modal');
  const a = document.activeElement;
  const key = a && el.contains(a) ? a.dataset.k : null;
  const card = el.querySelector('.mcard');
  if (!card) return;
  card.innerHTML = `<button class="mclose" data-act="close-modal" data-k="m-close" title="Закрыть">✕</button>${M.render()}`;
  const f = (key && el.querySelector(`[data-k="${CSS.escape(key)}"]`)) || el.querySelector('[data-autofocus]') ||
    el.querySelector('input:not([readonly]), .btn.primary, .btn');
  if (f) f.focus({ preventScroll: true });
}
function closeModal() {
  if (!M) return;
  const m = M;
  M = null;
  $('#modal').classList.add('hide');
  $('#modal').innerHTML = '';
  if (m.onClose) m.onClose();
  focusView();
}
function jobLine(job) {
  return `<div class="jobline"><div class="progress"><i style="width:${Math.round(((job && job.progress) || 0) * 100)}%"></i></div><span class="joblabel">${esc((job && job.label) || 'Работаю…')}</span></div>`;
}
function tickJob(j) {
  const bar = $('#modal .progress i') || $('#view .progress i');
  if (bar) bar.style.width = Math.round(j.progress * 100) + '%';
  const l = $('#modal .joblabel') || $('#view .joblabel');
  if (l) l.textContent = j.label;
}

// ---------------------------------------------------------------- header, hints, background
function renderHeader() {
  const a = S.st && S.st.attached;
  const chip = $('#gchip');
  chip.classList.toggle('on', !!a && S.online);
  chip.classList.toggle('off', !S.online);
  $('.t', chip).textContent = !S.online ? (S.authFail ? 'Нужен ключ' : 'Нет связи') : a ? a.title : 'Нет игры';
  $$('#tabs button').forEach(b => b.classList.toggle('on', b.dataset.view === tabOf(S.view)));
}

function renderHints() {
  const pad = S.pad;
  const gb = (c, t) => `<span class="gb ${c}">${t}</span>`, kb = t => `<span class="kb">${t}</span>`;
  const hint = (g, k, label) => `<span class="hint">${pad ? g : k}${label}</span>`;
  const parts = [hint(gb('a', 'A'), kb('Enter'), 'Выбрать'), hint(gb('b', 'B'), kb('Esc'), 'Назад'), hint(gb('sh', 'LB') + gb('sh', 'RB'), kb('Q') + kb('E'), 'Разделы')];
  if (pad && S.view === 'game') parts.push(hint(gb('y', 'Y'), '', 'Выключить все'));
  if (DESKTOP) parts.push(hint(gb('sh', 'View'), kb('F11'), 'Полный экран'));
  $('#hints').innerHTML = `<div class="hl">FahrenHack ${esc((S.st && S.st.version) || '')} · только одиночные игры</div><div class="hr">${parts.join('')}</div>`;
}

let bgKey;
function setBg(g) {
  document.documentElement.style.setProperty('--accent', (g && g.accent) || '#4bc0c0');
  const key = g && g.art && g.art.hero ? g.id : null;
  if (key === bgKey) return;
  bgKey = key;
  const el = $('#bg-img');
  el.classList.remove('on');
  if (!key) return;
  const url = artUrl(key, 'hero');
  const img = new Image();
  img.onload = () => {
    if (bgKey !== key) return;
    el.style.backgroundImage = `url("${url}")`;
    el.classList.add('on');
  };
  img.src = url;
}

// ---------------------------------------------------------------- views
const views = {};

views.auth = {
  render: () => `<section class="view"><div class="authbox panel">
    <img src="/logo.svg" alt="" width="72" height="72">
    <h2>Нужен ключ доступа</h2>
    <p>Откройте FahrenHack на компьютере, перейдите в раздел «Пульт» и отсканируйте QR-код камерой телефона.</p>
  </div></section>`,
};

// ----- home: hero + shelf of game tiles
function gameTile(g, i) {
  return `<button class="tile" data-act="open-game" data-id="${esc(g.id)}" data-k="tile-${esc(g.id)}" data-i="${i}" style="--accent:${esc(g.accent)}">
    <span class="tt">${esc(g.title)}</span>
    ${g.art && g.art.cover ? `<img src="${artUrl(g.id, 'cover')}" alt="" loading="lazy" onerror="this.remove()">` : ''}
    ${g.running ? '<span class="badge-live">● LIVE</span>' : ''}
  </button>`;
}
function addTile() {
  return `<button class="tile add" data-act="go" data-view="scanner" data-k="tile-add" data-i="${S.games.length}">${icon('plus')}<span>Любая игра</span></button>`;
}
function selTile(i) {
  if (S.sel === i) return;
  S.sel = i;
  views.home.hero();
}

views.home = {
  render() {
    return `<section class="view home">
      <div class="hero" id="hero"></div>
      <div><div class="shelf-title">Библиотека</div><div class="shelf" id="shelf"></div></div>
    </section>`;
  },
  mount() {
    this.games();
    const tiles = $$('#shelf .tile');
    const t = tiles[Math.min(S.sel, tiles.length - 1)];
    if (t) t.focus({ preventScroll: true });
  },
  update() { this.hero(); },
  games() {
    patchHtml($('#shelf'), S.games.map(gameTile).join('') + addTile());
    this.hero();
  },
  hero() {
    const el = $('#hero');
    if (!el) return;
    const g = S.games[S.sel];
    if (!g) {
      setBg(null);
      patchHtml(el, `<div class="eyebrow">Без профиля</div><h1>Любая игра</h1>
        <p class="sub">Подключитесь к любой одиночной игре, найдите значения сканером и сохраните их как свои читы.</p>
        <div class="actions"><button class="btn primary" data-act="go" data-view="scanner" data-k="hero-scan">${icon('scan')}Открыть сканер</button></div>`);
      return;
    }
    setBg(g);
    const n = g.cheats.length;
    const chips = [];
    if (g.steam_appid) chips.push(g.installed ? '<span class="chip ok">Установлена</span>' : '<span class="chip">Не установлена</span>');
    if (g.running) chips.push('<span class="chip live"><i class="pulse"></i>Запущена</span>');
    if (g.attached) chips.push(`<span class="chip teal">${icon('check')}Подключено</span>`);
    chips.push(`<span class="chip">${n} ${plural(n, 'чит', 'чита', 'читов')}</span>`);
    const title = g.art && g.art.logo
      ? `<img class="glogo" src="${artUrl(g.id, 'logo')}" alt="${esc(g.title)}" onerror="this.hidden=true;this.nextElementSibling.hidden=false"><h1 hidden>${esc(g.title)}</h1>`
      : `<h1>${esc(g.title)}</h1>`;
    patchHtml(el, `<div class="eyebrow">${g.known ? 'Готовый профиль' : 'Свой профиль'}</div>
      ${title}
      <p class="sub">${esc(g.subtitle)}</p>
      <div class="chips">${chips.join('')}</div>
      <div class="actions">
        <button class="btn primary" data-act="open-game" data-id="${esc(g.id)}" data-k="hero-open">${icon('bolt')}Читы</button>
        ${g.steam_appid ? `<button class="btn" data-act="launch" data-id="${esc(g.id)}" data-k="hero-play"${g.running ? ' disabled' : ''}>${icon('play')}${g.running ? 'Игра запущена' : 'Играть'}</button>` : ''}
      </div>`);
  },
};

// ----- game page: cover, process state, cheats
function viewedGame() {
  const id = S.params.id;
  const g = S.games.find(x => x.id === id);
  if (g) return g;
  const a = S.st && S.st.attached;
  if (a && a.game === id) return { id, title: a.title, subtitle: a.exe, notes: '', known: a.known, art: {}, cheats: [], accent: '#2b4c6f' };
  return null;
}

/** Parts of a cheat row. Rows stay in the DOM and only changed parts are patched,
 *  so a click is never lost to a re-render (values like HP change every second). */
function cheatParts(c) {
  const id = esc(c.id), k = 'c-' + id;
  let ctl;
  switch (c.status) {
    case 'ready':
      ctl = c.kind === 'set'
        ? `<button class="btn sm primary" data-act="toggle" data-id="${id}" data-k="${k}-go">Применить</button>`
        : `<button class="switch${c.active ? ' on' : ''}" role="switch" aria-checked="${!!c.active}" aria-label="${esc(c.name)}" data-act="toggle" data-id="${id}" data-k="${k}-go"></button>`;
      break;
    case 'calibrate':
      ctl = `<button class="btn sm outline" data-act="calibrate" data-id="${id}" data-k="${k}-go">${icon('scan')}Настроить</button>`;
      break;
    case 'waiting':
      ctl = '<span class="chip warn" title="Указатель пока пуст: загрузите сохранение">Войдите в игру</span>';
      break;
    case 'missing':
      ctl = '<span class="chip warn">Не для этой версии</span>';
      break;
    case 'nogame':
      ctl = '<span class="chip">Запустите игру</span>';
      break;
    default:
      ctl = '<span class="chip warn">Ошибка профиля</span>';
  }
  const meta = [];
  if (c.hotkey) meta.push(`<span class="key">${esc(c.hotkey)}</span>`);
  if (c.kind === 'set') meta.push(`<span class="chip sm">Значение: <b>${esc(fmtNum(c.value) || '—')}</b></span>`);
  else if (c.kind === 'freeze') meta.push(`<span class="chip sm">${c.value ? 'Держать: <b>' + esc(fmtNum(c.value)) + '</b>' : 'Держит текущее'}</span>`);
  if (c.current !== null && c.current !== undefined) meta.push(`<span class="chip sm">Сейчас: <b>${esc(fmtNum(c.current))}</b></span>`);
  if (c.experimental) meta.push('<span class="chip sm warn">Экспериментально</span>');
  const cal = c.calibration;
  if (cal && c.status === 'ready') {
    if (cal.chains > 0) meta.push(`<span class="chip sm teal" title="Цепочек указателей: ${cal.chains}">${icon('check')}Сохранено</span>`);
    else if (cal.session) meta.push('<span class="chip sm warn" title="Постоянный адрес не найден">До перезапуска игры</span>');
  }
  return {
    cls: `cheat${c.active ? ' on' : ''}${c.status !== 'ready' ? ' idle' : ''}`,
    icon: icon(c.icon),
    head: `<div class="cname">${esc(c.name)}</div>${c.description ? `<div class="cdesc">${esc(c.description)}</div>` : ''}`,
    meta: meta.join(''),
    ctl,
  };
}

const ROW_PARTS = [['.cico', 'icon'], ['.chead', 'head'], ['.cmeta', 'meta'], ['.cctl', 'ctl']];

function cheatRowHtml(c, p) {
  const id = esc(c.id);
  return `<div class="${p.cls}" data-row="${id}">
    <div class="cico">${p.icon}</div>
    <div class="cmain"><div class="chead">${p.head}</div><div class="cmeta">${p.meta}</div></div>
    <div class="cctl">${p.ctl}</div>
    <button class="more" data-act="cheat-menu" data-id="${id}" data-k="c-${id}-more" title="Параметры">${icon('dots')}</button>
  </div>`;
}

function renderCheatList(box, list) {
  const keys = list.map(c => (c.category || 'Читы') + '\u0001' + c.id).join('\u0002');
  const parts = list.map(cheatParts);
  if (box._keys !== keys) {
    let html = '', cat = null;
    list.forEach((c, i) => {
      const k = c.category || 'Читы';
      if (k !== cat) { cat = k; html += `<div class="cat">${esc(k)}</div>`; }
      html += cheatRowHtml(c, parts[i]);
    });
    patchHtml(box, html);
    box._keys = keys;
    list.forEach((c, i) => {
      const row = box.querySelector(`[data-row="${CSS.escape(c.id)}"]`);
      if (row) for (const [sel, key] of ROW_PARTS) row.querySelector(sel)._html = parts[i][key];
    });
    return;
  }
  list.forEach((c, i) => {
    const row = box.querySelector(`[data-row="${CSS.escape(c.id)}"]`);
    if (!row) return;
    const p = parts[i];
    if (row.className !== p.cls) row.className = p.cls;
    for (const [sel, key] of ROW_PARTS) patchHtml(row.querySelector(sel), p[key]);
  });
}

views.game = {
  render() {
    const g = viewedGame();
    if (!g) return '<section class="view"><div class="empty">Игра не найдена</div></section>';
    return `<section class="view gameview" style="--accent:${esc(g.accent)}">
      <aside class="gside">
        <div class="cover">${g.art && g.art.cover ? `<img src="${artUrl(g.id, 'cover')}" alt="">` : `<span class="tt">${esc(g.title)}</span>`}</div>
        <div class="ginfo">
          <h2>${esc(g.title)}</h2>
          <p class="sub">${esc(g.subtitle)}</p>
          <div class="panel procbox" id="procbox"></div>
        </div>
        ${g.notes ? `<p class="note">${esc(g.notes)}</p>` : ''}
      </aside>
      <div class="glist">
        <div class="ghead">
          <div><div class="eyebrow">Читы</div><h3>${esc(g.title)}</h3></div>
          <button class="btn sm" data-act="all-off" data-k="all-off">${icon('power')}Выключить все</button>
        </div>
        <div id="clist"></div>
      </div>
    </section>`;
  },
  mount() {
    setBg(viewedGame());
    this.update();
    const f = $('#clist .cctl button') || $('#clist .more') || $('#procbox button');
    if (f) f.focus({ preventScroll: true });
  },
  games() { this.update(); },
  update() {
    const g = viewedGame();
    const box = $('#clist');
    if (!g || !box) return;
    const a = S.st && S.st.attached;
    const live = !!a && a.game === g.id;
    let pb;
    if (live) {
      pb = `<div class="pstate"><i class="pulse"></i><div><b>Подключено</b><span>${esc(a.exe)} · PID ${a.pid}</span></div></div>
        <div class="mono dim">Сборка ${esc(a.build)}</div>
        <button class="btn sm ghost" data-act="detach" data-k="detach">Отключить</button>`;
    } else if (g.running) {
      pb = `<div class="pstate"><i class="pulse warn"></i><div><b>Игра запущена</b><span>Подключаюсь…</span></div></div>
        <p class="dim small">Если подключение не удаётся, запустите FahrenHack от имени администратора.</p>`;
    } else {
      pb = `<div class="pstate"><i></i><div><b>Игра не запущена</b><span>Читы появятся после запуска</span></div></div>
        ${g.steam_appid ? `<button class="btn primary" data-act="launch" data-id="${esc(g.id)}" data-k="launch">${icon('play')}Запустить игру</button>` : ''}`;
    }
    patchHtml($('#procbox'), pb);
    const list = live ? S.st.cheats : g.cheats.map(c => Object.assign({}, c, { status: 'nogame' }));
    if (!list.length) {
      box._keys = null;
      patchHtml(box, `<div class="panel empty">${g.known === false
        ? 'Своих читов пока нет. Найдите значение во вкладке «Сканер» и нажмите «В читы».'
        : 'В профиле нет читов.'}</div>`);
      return;
    }
    renderCheatList(box, list);
  },
};

// ----- scanner
views.scanner = {
  render() {
    return `<section class="view scanner"><div class="sgrid">
      <div class="panel scol">
        <div class="eyebrow">Процесс</div>
        <div id="sc-proc" class="sproc"></div>
        <div class="eyebrow mt">Поиск значения</div>
        <div class="row2">
          <select id="sc-type" class="inp">${vtypeOptions('i32')}</select>
          <input id="sc-val" class="inp" inputmode="decimal" placeholder="Значение" data-enter="sc-first">
        </div>
        <div class="row2">
          <button class="btn sm primary" data-act="sc-first" data-k="sc-first">Новый поиск</button>
          <button class="btn sm" data-act="sc-unknown" data-k="sc-unk" title="Значение не видно на экране (полоска здоровья)">Неизвестное</button>
        </div>
        <div class="grid5">
          <button class="btn sm" data-act="sc-next" data-mode="exact" data-k="sc-eq" title="Равно введённому">=</button>
          <button class="btn sm" data-act="sc-next" data-mode="increased" data-k="sc-inc" title="Увеличилось">↑</button>
          <button class="btn sm" data-act="sc-next" data-mode="decreased" data-k="sc-dec" title="Уменьшилось">↓</button>
          <button class="btn sm" data-act="sc-next" data-mode="changed" data-k="sc-chg" title="Изменилось">≠</button>
          <button class="btn sm" data-act="sc-next" data-mode="unchanged" data-k="sc-unc" title="Не изменилось">≡</button>
        </div>
        <button class="btn sm ghost" data-act="sc-reset" data-k="sc-reset">Сбросить поиск</button>
        <div id="sc-info" class="sinfo"></div>
        <p class="dim small">Число видно в игре: введите его → «Новый поиск», измените в игре, введите новое → «=». Числа не видно: «Неизвестное», затем ↑ ↓ после каждого изменения.</p>
      </div>
      <div class="panel scol">
        <div class="shead"><div class="eyebrow">Результаты</div><button class="btn sm ghost" data-act="sc-reload" data-k="sc-reload" title="Обновить значения">${icon('refresh')}</button></div>
        <div id="sc-res" class="sres"></div>
      </div>
    </div></section>`;
  },
  mount() {
    this._key = null;
    this.update();
    loadResults();
  },
  update() {
    const a = S.st && S.st.attached;
    patchHtml($('#sc-proc'), a
      ? `<div class="pstate"><i class="pulse"></i><div><b>${esc(a.exe)}</b><span>PID ${a.pid}${a.known ? ' · есть профиль' : ''}</span></div></div>
         <button class="btn sm" data-act="pick-process" data-k="sc-pick">Сменить</button>`
      : `<div class="pstate"><i></i><div><b>Нет процесса</b><span>Выберите запущенную игру</span></div></div>
         <button class="btn sm primary" data-act="pick-process" data-k="sc-pick">Выбрать</button>`);
    const sc = S.st && S.st.scan;
    const j = S.st && S.st.job;
    const busy = j && !j.done && j.kind === 'scan';
    let info = '';
    if (busy) info = jobLine(j);
    else if (sc && sc.snapshot) info = `Снимок памяти: <b>${sc.snapshot_mb} МБ</b>. Измените значение в игре и выберите ↑ ↓ ≠ ≡.`;
    else if (sc) info = `Найдено: <b>${Number(sc.count).toLocaleString('ru-RU')}</b>${sc.truncated ? ' (список обрезан)' : ''}`;
    patchHtml($('#sc-info'), info);
    const key = sc ? `${sc.count}|${sc.snapshot}` : 'none';
    if (!busy && this._key !== key) {
      this._key = key;
      loadResults();
    }
  },
};

async function loadResults() {
  const box = $('#sc-res');
  if (!box) return;
  let list = [];
  try { list = await api('/scan/results'); } catch (e) { /* not attached */ }
  const sc = S.st && S.st.scan;
  const vt = (sc && sc.vtype) || ($('#sc-type') && $('#sc-type').value) || 'i32';
  const total = (sc && sc.count) || 0;
  if (!list.length) {
    patchHtml(box, `<div class="empty">${sc ? 'Адресов нет' : 'Здесь появятся найденные адреса'}</div>`);
    return;
  }
  patchHtml(box, (total > list.length ? `<p class="dim small">Показаны первые ${list.length} из ${total.toLocaleString('ru-RU')}. Отсейте ещё.</p>` : '') +
    list.map((r, i) => `<div class="sr">
      <div class="sr-a"><span class="mono">${esc(r.addr)}</span>${r.static ? `<span class="chip sm teal">${esc(r.static)}</span>` : ''}</div>
      <div class="sr-v"><b>${esc(fmtNum(r.value ?? '—'))}</b><span class="dim">было ${esc(fmtNum(r.prev))}</span></div>
      <input class="inp sm" id="sr-in${i}" inputmode="decimal" placeholder="Новое">
      <button class="btn sm" data-act="sc-write" data-addr="${esc(r.addr)}" data-vt="${vt}" data-i="${i}" data-k="sr-w${i}">Записать</button>
      <button class="btn sm primary" data-act="sc-add" data-addr="${esc(r.addr)}" data-vt="${vt}" data-i="${i}" data-k="sr-add${i}">В читы</button>
    </div>`).join(''));
}

// ----- phone remote
views.remote = {
  render() {
    return '<section class="view remote"><div id="remote-box" class="rgrid"></div></section>';
  },
  mount() { this.update(); },
  update() {
    const st = S.st;
    if (!st) return;
    const on = st.settings.remote;
    const box = $('#remote-box');
    if (!on) {
      patchHtml(box, `<div class="panel qrbox off">${icon('wifi', 'huge')}</div>
        <div class="rtext"><div class="eyebrow">Пульт на телефоне</div><h2>Включайте читы с телефона</h2>
          <p class="lead">Пульт открывает этот же интерфейс в браузере телефона по Wi-Fi. Доступ защищён секретным ключом из QR-кода.</p>
          <p class="dim">При включении Windows может спросить про брандмауэр: разрешите доступ для частных сетей.</p>
          <div class="mrow"><button class="btn primary" data-act="remote" data-on="1" data-k="r-on" data-autofocus>${icon('wifi')}Включить пульт</button></div>
        </div>`);
      return;
    }
    const url = st.lan_url;
    patchHtml(box, `<div class="panel qrbox"><img class="qr" alt="QR-код для телефона" src="/api/qr?t=${encodeURIComponent(TOKEN)}&v=${encodeURIComponent(url)}"><div class="mono url">${esc(url)}</div></div>
      <div class="rtext"><div class="eyebrow">Пульт включён</div><h2>Отсканируйте QR-код</h2>
        <ol class="tips big"><li>Подключите телефон к той же сети Wi-Fi, что и компьютер.</li><li>Наведите камеру телефона на QR-код и откройте ссылку.</li><li>Включайте читы с телефона, не сворачивая игру.</li></ol>
        <p class="dim">Не открывается? Разрешите FahrenHack в брандмауэре Windows для частных сетей. Ссылка содержит ключ: не публикуйте её.</p>
        <div class="mrow">
          <button class="btn" data-act="copy-url" data-k="r-copy">Скопировать ссылку</button>
          <button class="btn" data-act="token-reset" data-k="r-reset">Новый ключ</button>
          <button class="btn ghost" data-act="remote" data-on="0" data-k="r-off">Выключить пульт</button>
        </div>
      </div>`);
  },
};

// ----- settings
views.settings = {
  render() {
    return `<section class="view"><div class="sett">
      <div class="eyebrow">Настройки</div>
      <h2>FahrenHack</h2>
      <div class="panel list" id="set-list"></div>
      <div class="panel about">
        <div class="brand big"><img src="/logo.svg" alt=""><span>Fahren<b>Hack</b></span></div>
        <p>Версия <span id="ver"></span> · открытый исходный код, лицензия MIT.</p>
        <p class="dim">Только для одиночных игр. Не используйте в онлайн-режимах и в играх с античитом: за это блокируют аккаунты. FahrenHack не связан с разработчиками и издателями игр.</p>
        <div class="mrow"><button class="btn sm" data-act="open-data" data-k="s-data">${icon('folder')}Папка данных</button></div>
      </div>
    </div></section>`;
  },
  mount() { this.update(); },
  update() {
    const s = S.st && S.st.settings;
    if (!s) return;
    const row = (key, title, desc) => `<button class="srow" data-act="set-toggle" data-key="${key}" data-k="set-${key}">
      <div><b>${title}</b><span>${desc}</span></div><span class="switch${s[key] ? ' on' : ''}"></span></button>`;
    patchHtml($('#set-list'),
      row('auto_attach', 'Автоподключение', 'Подключаться к игре с профилем сразу после её запуска') +
      row('hotkeys', 'Горячие клавиши', 'Включать читы клавишами прямо в игре') +
      row('remote', 'Пульт на телефоне', 'Доступ к FahrenHack с телефона по Wi-Fi') +
      row('sounds', 'Звуки интерфейса', 'Короткие звуки при навигации') +
      row('fullscreen', 'Полный экран при запуске', 'Открывать FahrenHack на весь экран; F11 переключает сразу'));
    $('#ver').textContent = S.st.version;
  },
};

// ---------------------------------------------------------------- cheat actions
async function refresh() {
  try {
    S.st = await api('/state');
    S.online = true;
    S.authFail = false;
  } catch (e) {
    S.online = false;
  }
  renderHeader();
  if (S.authFail && !S.st) {
    if (S.view !== 'auth') show('auth', {});
    return;
  }
  if (!S.st) return;
  const pid = S.st.attached ? S.st.attached.pid : null;
  if (refresh.lastPid !== undefined && pid !== refresh.lastPid) {
    if (pid) { toast(`Подключено: ${S.st.attached.title}`); sfx('on'); }
    loadGames();
  }
  refresh.lastPid = pid;
  const v = views[S.view];
  if (v && v.update) v.update();
}

async function loadGames() {
  try { S.games = await api('/games'); } catch (e) { return; }
  const v = views[S.view];
  if (v && v.games) v.games();
}

function openGame(id) { go('game', { id }); sfx('ok'); }

async function launch(id) {
  if (await act(() => api(`/games/${encodeURIComponent(id)}/launch`, 'POST'), 'Запускаю игру через Steam…')) sfx('ok');
}

async function toggleCheat(id, btn) {
  const isSwitch = btn && btn.classList.contains('switch');
  if (isSwitch) btn.classList.toggle('on');
  const r = await act(() => api(`/cheats/${encodeURIComponent(id)}/toggle`, 'POST'));
  if (r) {
    sfx(r.active ? 'on' : isSwitch ? 'off' : 'ok');
    if (!isSwitch) toast('Готово');
  } else if (isSwitch) {
    btn.classList.toggle('on');
  }
  refresh();
}

async function allOff() {
  if (await act(() => api('/cheats/off', 'POST'), 'Все читы выключены')) sfx('off');
  refresh();
}

async function detach() {
  await act(() => api('/detach', 'POST'), 'Отключено');
  refresh();
  loadGames();
}

function findCheat(id) {
  const g = viewedGame();
  const live = S.st && S.st.attached && g && S.st.attached.game === g.id;
  const list = live ? S.st.cheats : g ? g.cheats : [];
  return { g, live, c: list.find(x => x.id === id) };
}

function cheatMenu(id) {
  const { g, live, c } = findCheat(id);
  if (!c) return;
  const cal = live ? c.calibration : null;
  openModal({
    render: () => `<div class="mhead"><div class="cico big">${icon(c.icon)}</div><div><div class="eyebrow">Параметры чита</div><h2>${esc(c.name)}</h2></div></div>
      <label class="fld"><span>${c.kind === 'set' ? 'Записываемое значение' : 'Держать значение'}<i>${c.kind === 'freeze' ? 'пусто: то, что было при включении' : 'пусто: значение профиля'}</i></span>
        <input id="m-val" class="inp" inputmode="decimal" value="${esc(c.value ?? '')}" placeholder="${c.kind === 'freeze' ? 'текущее' : ''}" data-enter="m-save"></label>
      <label class="fld"><span>Горячая клавиша<i>нажмите сочетание; Backspace: без клавиши</i></span>
        <input id="m-key" class="inp" readonly data-hotkey value="${esc(c.hotkey ?? '')}" placeholder="нет"></label>
      <div class="mrow">
        <button class="btn primary" data-act="cm-save" data-g="${esc(g.id)}" data-id="${esc(c.id)}" data-k="m-save">Сохранить</button>
        <button class="btn" data-act="cm-defaults" data-g="${esc(g.id)}" data-id="${esc(c.id)}" data-k="m-def">Как в профиле</button>
        ${cal ? `<button class="btn" data-act="cm-recal" data-id="${esc(c.id)}" data-k="m-recal">${icon('scan')}Перенастроить</button>` : ''}
        ${cal && (cal.chains || cal.session) ? `<button class="btn ghost" data-act="cm-forget" data-id="${esc(c.id)}" data-k="m-forget">Сбросить привязку</button>` : ''}
        ${live && c.custom ? `<button class="btn danger" data-act="cm-delete" data-id="${esc(c.id)}" data-k="m-del">Удалить</button>` : ''}
      </div>
      ${c.addr ? `<p class="dim mono mt">Адрес сейчас: ${esc(c.addr)}${cal && cal.chains ? ` · цепочек: ${cal.chains}, подтверждений: ${cal.confirmations}` : ''}</p>` : ''}`,
  });
}

function captureKey(e) {
  if (e.key === 'Tab' || e.key.startsWith('Arrow')) return false;
  e.preventDefault();
  e.stopPropagation();
  const t = e.target;
  if (e.key === 'Escape') { t.blur(); return true; }
  if (e.key === 'Backspace' || e.key === 'Delete') { t.value = ''; return true; }
  let k = null;
  if (/^F\d{1,2}$/.test(e.key)) k = e.key;
  else if (/^Numpad\d$/.test(e.code)) k = 'NUM' + e.code.slice(6);
  else if (/^Key[A-Z]$/.test(e.code)) k = e.code.slice(3);
  else if (/^Digit\d$/.test(e.code)) k = e.code.slice(5);
  if (k) t.value = [e.ctrlKey && 'CTRL', e.shiftKey && 'SHIFT', e.altKey && 'ALT', k].filter(Boolean).join('+');
  return true;
}

// ---------------------------------------------------------------- calibration wizard
let W = null;

function calibrate(id) {
  const c = S.st && S.st.cheats && S.st.cheats.find(x => x.id === id);
  if (!c || !c.calibration) { toast('Сначала запустите игру: FahrenHack подключится к ней сам', true); return; }
  W = { id, c, mode: c.calibration.mode, vtype: c.vtype, phase: 'start', count: null, snapshotMb: null, busy: false, job: null, cands: [], result: null, err: '' };
  openModal({ wide: true, render: wizardHtml, onClose: () => { W = null; } });
}

function wizardHtml() {
  const cal = W.c.calibration;
  const steps = cal.steps || [];
  const phases = ['start', 'refine', 'pick', 'done'];
  const pi = phases.indexOf(W.phase === 'binding' ? 'pick' : W.phase);
  const typeSel = `<label class="fld"><span>Тип значения<i>если ничего не находится, попробуйте другой</i></span><select id="w-type" class="inp">${vtypeOptions(W.vtype)}</select></label>`;
  const showBtn = W.count && W.count <= 60 ? '<button class="btn" data-act="wz-pick" data-k="w-show">Показать адреса</button>' : '';
  let body = '';
  if (W.phase === 'start' && W.mode === 'exact') {
    body = `<p class="lead">${esc(steps[0] || 'Найдите это значение на экране игры.')}</p>
      <label class="fld"><span>${esc(cal.prompt || 'Сколько сейчас?')}</span><input id="w-val" class="inp big" inputmode="decimal" data-autofocus data-enter="w-go"></label>
      ${typeSel}
      <div class="mrow"><button class="btn primary" data-act="wz-first" data-k="w-go">${icon('scan')}Искать</button></div>`;
  } else if (W.phase === 'start') {
    body = `<p class="lead">${esc(steps[0] || 'Сделайте снимок памяти, затем меняйте значение в игре.')}</p>
      ${typeSel}
      <p class="dim small mt">Снимок временно копирует память игры на диск (до нескольких ГБ) и удаляется после первого отсева.</p>
      <div class="mrow"><button class="btn primary" data-act="wz-snapshot" data-k="w-go" data-autofocus>${icon('scan')}Снимок</button></div>`;
  } else if (W.phase === 'refine') {
    const cnt = W.count === null
      ? `<div class="count sm">Снимок готов${W.snapshotMb ? ` · ${W.snapshotMb} МБ` : ''}</div>`
      : `<div class="count">${W.count.toLocaleString('ru-RU')}</div><div class="dim">${plural(W.count, 'адрес подходит', 'адреса подходят', 'адресов подходят')}</div>`;
    if (W.mode === 'exact') {
      body = `${cnt}<p class="lead">${esc(steps[1] || 'Измените значение в игре и введите новое.')}</p>
        <label class="fld"><span>Новое значение</span><input id="w-val" class="inp big" inputmode="decimal" data-autofocus data-enter="w-go"></label>
        <div class="mrow"><button class="btn primary" data-act="wz-next" data-mode="exact" data-k="w-go">Отсеять</button>${showBtn}<button class="btn ghost" data-act="wz-restart" data-k="w-restart">Заново</button></div>`;
    } else {
      body = `${cnt}<ol class="tips">${steps.slice(1).map(s => `<li>${esc(s)}</li>`).join('')}</ol>
        <div class="grid4">
          <button class="btn" data-act="wz-next" data-mode="decreased" data-k="w-dec" data-autofocus>↓ Уменьшилось</button>
          <button class="btn" data-act="wz-next" data-mode="increased" data-k="w-inc">↑ Увеличилось</button>
          <button class="btn" data-act="wz-next" data-mode="unchanged" data-k="w-unc"${W.count === null ? ' disabled title="Сначала измените значение в игре"' : ''}>= Не изменилось</button>
          <button class="btn" data-act="wz-next" data-mode="changed" data-k="w-chg">≠ Изменилось</button>
        </div>
        <div class="mrow">${showBtn}<button class="btn ghost" data-act="wz-restart" data-k="w-restart">Заново</button></div>`;
    }
  } else if (W.phase === 'pick') {
    body = `<p class="lead">Осталось ${W.cands.length} ${plural(W.cands.length, 'адрес', 'адреса', 'адресов')}. Чтобы проверить адрес, запишите в него число и посмотрите, изменилось ли значение в игре. Затем нажмите «Это оно».</p>
      <div class="cands">${W.cands.map((r, i) => `<div class="cand">
        <div class="mono">${esc(r.addr)}${r.static ? ` <span class="chip sm teal">${esc(r.static)}</span>` : ''}</div>
        <div class="cv">${esc(fmtNum(r.value ?? '—'))}</div>
        <input class="inp sm" id="w-t${i}" inputmode="decimal" placeholder="Число" data-enter="w-test${i}">
        <button class="btn sm" data-act="wz-test" data-i="${i}" data-k="w-test${i}">Записать</button>
        <button class="btn sm primary" data-act="wz-bind" data-i="${i}" data-k="w-bind${i}"${i === 0 ? ' data-autofocus' : ''}>Это оно</button>
      </div>`).join('')}</div>
      <div class="mrow"><button class="btn ghost" data-act="wz-back" data-k="w-back">Назад к отсеву</button></div>`;
  } else if (W.phase === 'binding') {
    body = '<p class="lead">Ищу постоянный путь к значению, чтобы чит находился сам и после перезапуска игры. Обычно это занимает от 10 секунд до пары минут.</p>';
  } else if (W.phase === 'done') {
    const r = W.result || {};
    const how = r.how === 'confirmed' ? `Привязка подтверждена ещё раз (${r.confirmations}).`
      : r.how === 'static' ? 'Значение лежит в статической памяти игры.'
        : `Найдено цепочек указателей: ${r.chains}.`;
    body = r.chains > 0
      ? `<div class="done ok">${icon('check')}</div><p class="lead">Готово! ${how} Чит будет находиться сам и после перезапуска игры.</p>`
      : `<div class="done warn">!</div><p class="lead">Чит работает, но только до перезапуска игры: постоянный путь к значению не найден.</p>${r.warning ? `<p class="dim small">${esc(r.warning)}</p>` : ''}`;
    body += '<div class="mrow"><button class="btn primary" data-act="close-modal" data-k="w-ok" data-autofocus>Готово</button></div>';
  }
  return `<div class="mhead"><div class="cico big">${icon(W.c.icon)}</div><div>
      <div class="eyebrow">Настройка · ${W.mode === 'exact' ? 'видимое число' : 'скрытое значение'}</div><h2>${esc(W.c.name)}</h2></div></div>
    <div class="steps">${phases.map((p, i) => `<i class="${i <= pi ? 'on' : ''}"></i>`).join('')}</div>
    <div class="wbody${W.busy ? ' busy' : ''}">${body}</div>
    ${W.busy ? jobLine(W.job) : ''}${W.err ? `<p class="err">${esc(W.err)}</p>` : ''}`;
}

async function wzJob(start, after) {
  const w = W;
  w.busy = true;
  w.err = '';
  w.job = null;
  drawModal();
  try {
    const r = await runJob(start, j => { w.job = j; tickJob(j); });
    w.busy = false;
    await after(r);
  } catch (e) {
    w.busy = false;
    w.err = e.message;
    if (w.phase === 'binding') w.phase = 'pick';
    sfx('err');
  }
  if (W === w) drawModal();
}

async function wzLoadCands() {
  try {
    W.cands = (await api('/scan/results')).slice(0, 60);
    W.phase = 'pick';
  } catch (e) {
    W.err = e.message;
  }
}

const wizardActions = {
  'wz-first': () => {
    const val = $('#w-val').value.trim();
    if (!val) { W.err = 'Введите число'; drawModal(); return; }
    W.vtype = $('#w-type').value;
    wzJob(api('/scan/first', 'POST', { vtype: W.vtype, value: val }), async r => {
      W.count = r.count;
      if (!r.count) { W.err = 'Ничего не найдено. Проверьте число или выберите другой тип.'; return; }
      W.phase = 'refine';
      sfx('ok');
      if (r.count <= 6) await wzLoadCands();
    });
  },
  'wz-snapshot': () => {
    W.vtype = $('#w-type').value;
    wzJob(api('/scan/unknown', 'POST', { vtype: W.vtype }), async r => {
      W.count = null;
      W.snapshotMb = r.snapshot_mb;
      W.phase = 'refine';
      sfx('ok');
    });
  },
  'wz-next': el => {
    const mode = el.dataset.mode;
    let value;
    if (mode === 'exact') {
      value = $('#w-val').value.trim();
      if (!value) { W.err = 'Введите новое значение'; drawModal(); return; }
    }
    wzJob(api('/scan/next', 'POST', { mode, value }), async r => {
      W.count = r.count;
      sfx('ok');
      if (r.count === 0) W.err = 'Адресов не осталось. Нажмите «Заново»; возможно, нужен другой тип значения.';
      else if (r.count <= 6) await wzLoadCands();
    });
  },
  'wz-pick': async () => { await wzLoadCands(); drawModal(); },
  'wz-back': () => { W.phase = 'refine'; W.err = ''; drawModal(); },
  'wz-restart': async () => {
    await api('/scan/reset', 'POST').catch(() => null);
    Object.assign(W, { phase: 'start', count: null, snapshotMb: null, err: '', cands: [] });
    drawModal();
  },
  'wz-test': async el => {
    const i = +el.dataset.i;
    const r = W.cands[i];
    const v = $('#w-t' + i).value.trim();
    if (!v) { toast('Введите число для проверки', true); return; }
    if (await act(() => api('/write', 'POST', { addr: r.addr, vtype: W.vtype, value: v }), 'Записано: посмотрите, изменилось ли значение в игре')) {
      await wzLoadCands();
      drawModal();
    }
  },
  'wz-bind': el => {
    const r = W.cands[+el.dataset.i];
    W.phase = 'binding';
    wzJob(api(`/calib/${encodeURIComponent(W.id)}/bind`, 'POST', { addr: r.addr, vtype: W.vtype }), async res => {
      W.result = res;
      W.phase = 'done';
      sfx('on');
      refresh();
    });
  },
};

// ---------------------------------------------------------------- scanner actions
async function scRun(start, msg) {
  const r = await act(() => runJob(start, tickJob));
  await refresh();
  loadResults();
  if (r && msg) toast(msg(r));
}

function pickProcess() {
  api('/processes').then(list => {
    const rows = q => list.filter(p => p.name.toLowerCase().includes(q.toLowerCase())).slice(0, 150)
      .map(p => `<button class="prow" data-act="attach" data-pid="${p.pid}" data-k="p-${p.pid}"><span>${esc(p.name)}</span><span class="dim mono">PID ${p.pid}</span></button>`).join('') ||
      '<div class="empty">Ничего не найдено</div>';
    openModal({
      render: () => `<div class="mhead"><div class="cico big">${icon('games')}</div><div><div class="eyebrow">Подключение</div><h2>Процесс игры</h2></div></div>
        <input id="p-q" class="inp mt" placeholder="Поиск, например re4.exe" data-autofocus>
        <div id="p-list" class="plist">${rows('')}</div>`,
      filter: q => { $('#p-list').innerHTML = rows(q); },
    });
  }).catch(e => toast(e.message, true));
}

let addState = null;
function addCheatModal(addr, vt, value) {
  addState = { addr, vt, busy: false, job: null, err: '' };
  openModal({
    onClose: () => { addState = null; },
    render: () => `<div class="mhead"><div class="cico big">${icon('star')}</div><div><div class="eyebrow">Новый чит</div><h2>Адрес ${esc(addr)}</h2></div></div>
      <label class="fld"><span>Название</span><input id="a-name" class="inp" placeholder="Например: Бесконечные патроны" data-autofocus data-enter="a-save"></label>
      <div class="row2">
        <label class="fld"><span>Режим</span><select id="a-kind" class="inp"><option value="freeze">Держать значение</option><option value="set">Записывать по кнопке</option></select></label>
        <label class="fld"><span>Значение<i>пусто: текущее</i></span><input id="a-val" class="inp" inputmode="decimal" value="${esc(value)}"></label>
      </div>
      <div class="wbody${addState && addState.busy ? ' busy' : ''}"><div class="mrow"><button class="btn primary" data-act="add-save" data-k="a-save">Добавить</button></div></div>
      ${addState && addState.busy ? jobLine(addState.job) : ''}${addState && addState.err ? `<p class="err">${esc(addState.err)}</p>` : ''}`,
  });
}

async function addSave() {
  const A = addState;
  if (!A || A.busy) return;
  const name = $('#a-name').value.trim(), kind = $('#a-kind').value, value = $('#a-val').value.trim();
  if (!name) { A.err = 'Введите название'; drawModal(); return; }
  if (kind === 'set' && !value) { A.err = 'Для режима «по кнопке» нужно значение'; drawModal(); return; }
  A.busy = true;
  A.err = '';
  drawModal();
  $('#a-name').value = name;
  try {
    const { id } = await api('/custom', 'POST', { name, vtype: A.vt, kind, value: value || null });
    const r = await runJob(api(`/calib/${encodeURIComponent(id)}/bind`, 'POST', { addr: A.addr, vtype: A.vt }), j => { A.job = j; tickJob(j); });
    closeModal();
    toast(r.chains > 0 ? 'Чит добавлен и будет работать после перезапуска игры' : 'Чит добавлен (работает до перезапуска игры)');
    sfx('on');
    await refresh();
    await loadGames();
    const gid = S.st && S.st.attached && S.st.attached.game;
    if (gid) go('game', { id: gid });
  } catch (e) {
    A.busy = false;
    A.err = e.message;
    drawModal();
  }
}

// ---------------------------------------------------------------- action dispatch
const ACTIONS = Object.assign({
  'close-modal': () => closeModal(),
  'go': el => go(el.dataset.view),
  'open-game': el => openGame(el.dataset.id),
  'launch': el => launch(el.dataset.id),
  'toggle': el => toggleCheat(el.dataset.id, el),
  'calibrate': el => calibrate(el.dataset.id),
  'cheat-menu': el => cheatMenu(el.dataset.id),
  'all-off': () => allOff(),
  'detach': () => detach(),
  'cm-save': async el => {
    const body = { value: $('#m-val').value.trim(), hotkey: $('#m-key').value.trim() };
    if (await act(() => api(`/games/${encodeURIComponent(el.dataset.g)}/cheats/${encodeURIComponent(el.dataset.id)}`, 'PUT', body), 'Сохранено')) {
      closeModal();
      refresh();
      loadGames();
    }
  },
  'cm-defaults': async el => {
    if (await act(() => api(`/games/${encodeURIComponent(el.dataset.g)}/cheats/${encodeURIComponent(el.dataset.id)}`, 'DELETE'), 'Вернул настройки профиля')) {
      closeModal();
      refresh();
      loadGames();
    }
  },
  'cm-recal': el => { closeModal(); calibrate(el.dataset.id); },
  'cm-forget': async el => {
    if (await act(() => api(`/cheats/${encodeURIComponent(el.dataset.id)}/calibration`, 'DELETE'), 'Привязка сброшена')) { closeModal(); refresh(); }
  },
  'cm-delete': async el => {
    if (await act(() => api(`/custom/${encodeURIComponent(el.dataset.id)}`, 'DELETE'), 'Чит удалён')) { closeModal(); refresh(); loadGames(); }
  },
  'pick-process': () => pickProcess(),
  'attach': async el => {
    if (await act(() => api('/attach', 'POST', { pid: +el.dataset.pid }), 'Подключено')) { closeModal(); await refresh(); loadGames(); }
  },
  'sc-first': () => {
    const value = $('#sc-val').value.trim();
    if (!value) { toast('Введите значение', true); return; }
    scRun(api('/scan/first', 'POST', { vtype: $('#sc-type').value, value }), r => `Найдено: ${r.count.toLocaleString('ru-RU')}`);
  },
  'sc-unknown': () => scRun(api('/scan/unknown', 'POST', { vtype: $('#sc-type').value }), r => `Снимок памяти: ${r.snapshot_mb} МБ`),
  'sc-next': el => {
    const mode = el.dataset.mode;
    const value = $('#sc-val').value.trim();
    if (mode === 'exact' && !value) { toast('Введите значение', true); return; }
    scRun(api('/scan/next', 'POST', { mode, value: mode === 'exact' ? value : undefined }), r => `Осталось: ${r.count.toLocaleString('ru-RU')}`);
  },
  'sc-reset': async () => { await act(() => api('/scan/reset', 'POST')); await refresh(); loadResults(); },
  'sc-reload': () => loadResults(),
  'sc-write': async el => {
    const v = $('#sr-in' + el.dataset.i).value.trim();
    if (!v) { toast('Введите новое значение', true); return; }
    if (await act(() => api('/write', 'POST', { addr: el.dataset.addr, vtype: el.dataset.vt, value: v }), 'Записано')) loadResults();
  },
  'sc-add': el => {
    if (!S.st || !S.st.attached) { toast('Сначала подключитесь к игре', true); return; }
    addCheatModal(el.dataset.addr, el.dataset.vt, $('#sr-in' + el.dataset.i).value.trim());
  },
  'add-save': () => addSave(),
  'remote': async el => {
    const on = el.dataset.on === '1';
    if (await act(() => api('/settings', 'PUT', { remote: on }))) {
      sfx(on ? 'on' : 'off');
      await refresh();
    }
  },
  'copy-url': async () => {
    try { await navigator.clipboard.writeText(S.st.lan_url); toast('Ссылка скопирована'); }
    catch (e) { toast(S.st.lan_url); }
  },
  'token-reset': async () => {
    if (!confirm('Создать новый ключ? Старые ссылки на телефонах перестанут работать.')) return;
    const r = await act(() => api('/token/reset', 'POST'));
    if (r) {
      TOKEN = r.token;
      try { localStorage.setItem('fh-token', TOKEN); } catch (e) { /* ignore */ }
      toast('Ключ обновлён: отсканируйте новый QR-код');
      refresh();
    }
  },
  'set-toggle': async el => {
    const key = el.dataset.key;
    const v = !S.st.settings[key];
    S.st.settings[key] = v;
    views.settings.update();
    if (await act(() => api('/settings', 'PUT', { [key]: v }))) sfx(v ? 'on' : 'off');
    refresh();
  },
  'open-data': () => act(() => api('/open-data', 'POST')),
}, wizardActions);

document.addEventListener('click', e => {
  const el = e.target.closest('[data-act]');
  if (!el || el.disabled) return;
  const f = ACTIONS[el.dataset.act];
  if (f) { e.preventDefault(); f(el, e); }
});

document.addEventListener('input', e => {
  if (e.target.id === 'p-q' && M && M.filter) M.filter(e.target.value);
});

$('#view').addEventListener('focusin', e => {
  const t = e.target.closest('.tile');
  if (t && S.view === 'home') selTile(+t.dataset.i);
});
$('#view').addEventListener('mouseover', e => {
  const t = e.target.closest('.tile');
  if (t && S.view === 'home') selTile(+t.dataset.i);
});

// ---------------------------------------------------------------- keyboard
document.addEventListener('keydown', e => {
  const t = e.target;
  if (S.pad) { S.pad = false; document.body.classList.remove('pad'); renderHints(); }
  if (t.dataset && t.dataset.hotkey !== undefined && captureKey(e)) return;
  const typing = t.tagName === 'TEXTAREA' || (t.tagName === 'INPUT' && !t.readOnly);
  if (/^F([1-9]|10)$/.test(e.key)) { e.preventDefault(); return; } // game hotkeys are handled globally; no page reload
  if (DESKTOP && e.ctrlKey && ['KeyR', 'KeyP', 'KeyF', 'KeyU'].includes(e.code)) { e.preventDefault(); return; }
  switch (e.key) {
    case 'ArrowUp':
    case 'ArrowDown':
      if (t.tagName === 'SELECT') return;
      e.preventDefault();
      moveFocus(e.key === 'ArrowUp' ? 'up' : 'down');
      return;
    case 'ArrowLeft':
    case 'ArrowRight':
      if (typing || t.tagName === 'SELECT') return;
      e.preventDefault();
      moveFocus(e.key === 'ArrowLeft' ? 'left' : 'right');
      return;
    case 'Escape':
      e.preventDefault();
      back();
      return;
    case 'Backspace':
      if (!typing) { e.preventDefault(); back(); }
      return;
    case 'F11':
      e.preventDefault();
      host('fullscreen');
      return;
    case 'Enter':
      if (t.dataset && t.dataset.enter) {
        e.preventDefault();
        const b = (M ? $('#modal') : $('#view')).querySelector(`[data-k="${CSS.escape(t.dataset.enter)}"]`);
        if (b) b.click();
      }
      return;
  }
  if (!typing && !e.ctrlKey && !e.altKey) {
    if (e.code === 'KeyQ' || e.key === 'PageUp') { e.preventDefault(); cycleTab(-1); }
    else if (e.code === 'KeyE' || e.key === 'PageDown') { e.preventDefault(); cycleTab(1); }
  }
});

// ---------------------------------------------------------------- gamepad
const PAD = { prev: {}, rep: {} };
function padEdge(name, down, fn) {
  if (down && !PAD.prev[name]) fn();
  PAD.prev[name] = down;
}
function padRepeat(name, down, ts, fn) {
  if (!down) { delete PAD.rep[name]; return; }
  const next = PAD.rep[name];
  if (next === undefined) { PAD.rep[name] = ts + 360; fn(); }
  else if (ts >= next) { PAD.rep[name] = ts + 110; fn(); }
}
function padLoop(ts) {
  const pads = navigator.getGamepads ? [...navigator.getGamepads()].filter(Boolean) : [];
  const p = pads[0];
  if (p) {
    const btn = i => !!(p.buttons[i] && p.buttons[i].pressed);
    const ax = p.axes[0] || 0, ay = p.axes[1] || 0;
    if (!S.pad && (p.buttons.some(b => b.pressed) || Math.abs(ax) > 0.5 || Math.abs(ay) > 0.5)) {
      S.pad = true;
      document.body.classList.add('pad');
      renderHints();
    }
    padRepeat('up', btn(12) || ay < -0.55, ts, () => moveFocus('up'));
    padRepeat('down', btn(13) || ay > 0.55, ts, () => moveFocus('down'));
    padRepeat('left', btn(14) || ax < -0.55, ts, () => moveFocus('left'));
    padRepeat('right', btn(15) || ax > 0.55, ts, () => moveFocus('right'));
    padEdge('a', btn(0), () => { activate(); sfx('ok'); });
    padEdge('b', btn(1), back);
    padEdge('y', btn(3), () => { if (S.view === 'game' && !M) allOff(); });
    padEdge('lb', btn(4), () => cycleTab(-1));
    padEdge('rb', btn(5), () => cycleTab(1));
    padEdge('view', btn(8), () => host('fullscreen'));
    padEdge('start', btn(9), () => tab('settings'));
  }
  requestAnimationFrame(padLoop);
}

// ---------------------------------------------------------------- start
function init() {
  $$('#tabs button').forEach(b => b.addEventListener('click', () => tab(b.dataset.view)));
  $$('.winctl button').forEach(b => b.addEventListener('click', () => host(b.dataset.win)));
  $('#top').addEventListener('mousedown', e => {
    if (!DESKTOP || e.button !== 0 || e.target.closest('button, input, select, a')) return;
    host(e.detail === 2 ? 'max' : 'drag');
  });
  $('#modal').addEventListener('mousedown', e => { if (e.target.id === 'modal') closeModal(); });
  document.addEventListener('contextmenu', e => { if (DESKTOP && !e.target.closest('input, textarea')) e.preventDefault(); });
  const clock = () => { $('#clock').textContent = new Date().toLocaleTimeString('ru-RU', { hour: '2-digit', minute: '2-digit' }); };
  clock();
  setInterval(clock, 5000);
  renderHints();
  Promise.all([refresh(), loadGames()]).then(() => {
    if (S.view !== 'auth') show('home', {});
    const stateLoop = async () => { await refresh(); setTimeout(stateLoop, document.hidden ? 2500 : 800); };
    const gamesLoop = async () => { await loadGames(); setTimeout(gamesLoop, 4000); };
    setTimeout(stateLoop, 800);
    setTimeout(gamesLoop, 4000);
  });
  requestAnimationFrame(padLoop);
}
init();
