import { esc, fmtNum, icon, patchHtml, rememberHtml } from './view-helpers.js';

// Key of the pinned group. It goes into a data-* attribute, so it must not contain
// control characters (the HTML parser replaces U+0000).
const PINNED = '__pinned__';
const COLLAPSED_KEY = 'fh-collapsed';

/** Categories the viewer folded, remembered per browser. */
const collapsed = new Set((() => {
  try { return JSON.parse(localStorage.getItem(COLLAPSED_KEY) || '[]'); } catch { return []; }
})());

export function toggleCategory(name) {
  if (collapsed.has(name)) collapsed.delete(name); else collapsed.add(name);
  try { localStorage.setItem(COLLAPSED_KEY, JSON.stringify([...collapsed])); } catch { /* storage blocked */ }
}

const isAdjustable = c => c.kind === 'number' || c.kind === 'slider' || c.kind === 'select';

function keyChips(c) {
  const out = [];
  if (isAdjustable(c)) {
    if (c.hotkey) out.push(`<span class="key"><i>${c.kind === 'select' ? '→' : '↑'}</i>${esc(c.hotkey)}</span>`);
    if (c.hotkey_down) out.push(`<span class="key"><i>${c.kind === 'select' ? '←' : '↓'}</i>${esc(c.hotkey_down)}</span>`);
  } else if (c.hotkey) {
    out.push(`<span class="key">${esc(c.hotkey)}</span>`);
  }
  return out.join('');
}

/** Value a slider shows: the remembered setting, else "off", else its minimum. */
export function sliderValue(c) {
  const v = c.value ?? c.off_value ?? c.min ?? 0;
  return Number(String(v).replace(',', '.'));
}

function control(c, id, k) {
  switch (c.status) {
    case 'ready': break;
    case 'calibrate':
      return `<button class="btn sm outline" data-act="calibrate" data-id="${id}" data-k="${k}-go">${icon('scan')}Настроить</button>`;
    case 'waiting':
      return '<span class="chip warn" title="Указатель пока пуст: загрузите сохранение">Войдите в игру</span>';
    case 'missing':
      return '<span class="chip warn">Не для этой версии</span>';
    case 'nogame':
      return '<span class="chip">Запустите игру</span>';
    default:
      return '<span class="chip warn">Ошибка профиля</span>';
  }
  switch (c.kind) {
    case 'set':
      return `<button class="btn sm primary" data-act="toggle" data-id="${id}" data-k="${k}-go">Применить</button>`;
    case 'number':
      return `<div class="stepper">
        <button class="sbtn" data-act="step" data-dir="-1" data-id="${id}" data-k="${k}-dn" aria-label="Меньше">−</button>
        <input class="inp sm num" data-num data-id="${id}" data-k="${k}-in" inputmode="decimal" aria-label="${esc(c.name)}">
        <button class="sbtn" data-act="step" data-dir="1" data-id="${id}" data-k="${k}-up" aria-label="Больше">+</button>
      </div>`;
    case 'slider':
      return `<div class="slider">
        <input type="range" data-range data-id="${id}" data-k="${k}-in" min="${c.min ?? 0}" max="${c.max ?? 10}" step="${c.step ?? 1}" aria-label="${esc(c.name)}">
        <output data-out="${id}"></output>
      </div>`;
    case 'select':
      return `<div class="selectrow">
        <select class="inp sm" data-sel data-id="${id}" data-k="${k}-in" aria-label="${esc(c.name)}">
          ${(c.options || []).map(o => `<option value="${esc(o.value)}">${esc(o.label)}</option>`).join('')}
        </select>
        <button class="btn sm primary" data-act="sel-apply" data-id="${id}" data-k="${k}-go">Применить</button>
      </div>`;
    default:
      return `<button class="switch${c.active ? ' on' : ''}" role="switch" aria-checked="${!!c.active}" aria-label="${esc(c.name)}" data-act="toggle" data-id="${id}" data-k="${k}-go"></button>`;
  }
}

/** Parts of a cheat row. Rows stay in the DOM and only changed parts are patched,
 *  so a click is never lost to a re-render (values like HP change every second).
 *  Live values inside inputs are set by syncControls, never through the markup. */
function cheatParts(c) {
  const id = esc(c.id), k = 'c-' + id;
  const meta = [keyChips(c)];
  if (c.kind === 'set') meta.push(`<span class="chip sm">Значение: <b>${esc(fmtNum(c.value) || '—')}</b></span>`);
  else if (c.kind === 'freeze') meta.push(`<span class="chip sm">${c.value ? 'Держать: <b>' + esc(fmtNum(c.value)) + '</b>' : 'Держит текущее'}</span>`);
  if (c.kind !== 'number' && c.current !== null && c.current !== undefined) meta.push(`<span class="chip sm">Сейчас: <b>${esc(fmtNum(c.current))}</b></span>`);
  if (c.experimental) meta.push('<span class="chip sm warn">Экспериментально</span>');
  const cal = c.calibration;
  if (cal && c.status === 'ready') {
    if (cal.chains > 0) meta.push(`<span class="chip sm teal" title="Цепочек указателей: ${cal.chains}">${icon('check')}Сохранено</span>`);
    else if (cal.session) meta.push('<span class="chip sm warn" title="Постоянный адрес не найден">До перезапуска игры</span>');
  }
  const tips = (c.info ? `<span class="tip" tabindex="0" title="${esc(c.info)}" aria-label="${esc(c.info)}">i</span>` : '')
    + (c.warn ? `<span class="tip warn" tabindex="0" title="${esc(c.warn)}" aria-label="${esc(c.warn)}">!</span>` : '');
  return {
    cls: `cheat k-${esc(c.kind)}${c.active ? ' on' : ''}${c.status !== 'ready' ? ' idle' : ''}${c.pinned ? ' pinned' : ''}`,
    icon: icon(c.icon),
    head: `<div class="cname">${esc(c.name)}${tips}</div>${c.description ? `<div class="cdesc">${esc(c.description)}</div>` : ''}`,
    meta: meta.join(''),
    ctl: control(c, id, k),
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

/** Puts live values into inputs, except the one the user is editing. */
function syncControls(row, c) {
  const active = document.activeElement;
  const num = row.querySelector('[data-num]');
  if (num && num !== active) num.value = c.current ?? '';
  const range = row.querySelector('[data-range]');
  if (range && range !== active && !range.dataset.dragging) {
    range.value = sliderValue(c);
    const out = row.querySelector('[data-out]');
    if (out) out.textContent = rangeLabel(c, range.value);
  }
  const sel = row.querySelector('[data-sel]');
  if (sel && sel !== active && c.value !== null && c.value !== undefined) sel.value = c.value;
}

export function rangeLabel(c, v) {
  const n = Number(v);
  if (c.off_value !== null && c.off_value !== undefined && Number(c.off_value) === n) return 'выкл';
  return (Math.round(n * 100) / 100).toLocaleString('ru-RU') + (c.max !== null && c.max <= 20 ? '×' : '');
}

/** Groups: pinned first, then profile categories in their order, then the rest as they appear. */
function groups(list, categories) {
  const order = (categories || []).map(x => x.name);
  const icons = Object.fromEntries((categories || []).map(x => [x.name, x.icon]));
  const map = new Map();
  const pinned = list.filter(c => c.pinned);
  if (pinned.length) map.set(PINNED, pinned);
  for (const c of list) {
    if (c.pinned) continue;
    const k = c.category || 'Читы';
    if (!map.has(k)) map.set(k, []);
    map.get(k).push(c);
  }
  const keys = [...map.keys()].sort((a, b) => {
    if (a === PINNED) return -1;
    if (b === PINNED) return 1;
    const ia = order.indexOf(a), ib = order.indexOf(b);
    return (ia < 0 ? 1e6 : ia) - (ib < 0 ? 1e6 : ib);
  });
  return keys.map(k => ({ key: k, title: k === PINNED ? 'Закреплённые' : k, icon: k === PINNED ? 'pin' : (icons[k] || ''), items: map.get(k) }));
}

const renderedKeys = new WeakMap();

export function renderCheatList(box, list, emptyMessage, categories) {
  if (!list.length) {
    renderedKeys.delete(box);
    patchHtml(box, `<div class="panel empty">${esc(emptyMessage)}</div>`);
    return;
  }
  const gs = groups(list, categories);
  const keys = gs.map(g => g.key + (collapsed.has(g.key) ? '-' : '+') + '\u0001' + g.items.map(c => c.id).join('\u0001')).join('\u0002');
  const byId = new Map(list.map(c => [c.id, c]));
  const parts = new Map(list.map(c => [c.id, cheatParts(c)]));
  if (renderedKeys.get(box) !== keys) {
    let html = '';
    for (const g of gs) {
      const folded = collapsed.has(g.key);
      html += `<button class="cat${folded ? ' folded' : ''}" data-act="cat-toggle" data-cat="${esc(g.key)}" data-k="cat-${esc(g.key)}" aria-expanded="${!folded}">
        ${g.icon ? icon(g.icon) : ''}<span>${esc(g.title)}</span><em>${g.items.length}</em>${icon('chevron', 'chev')}</button>`;
      if (!folded) html += g.items.map(c => cheatRowHtml(c, parts.get(c.id))).join('');
    }
    patchHtml(box, html);
    renderedKeys.set(box, keys);
    box.querySelectorAll('[data-row]').forEach(row => {
      const p = parts.get(row.dataset.row);
      for (const [sel, key] of ROW_PARTS) rememberHtml(row.querySelector(sel), p[key]);
      syncControls(row, byId.get(row.dataset.row));
    });
    return;
  }
  box.querySelectorAll('[data-row]').forEach(row => {
    const c = byId.get(row.dataset.row);
    const p = parts.get(row.dataset.row);
    if (!c || !p) return;
    if (row.className !== p.cls) row.className = p.cls;
    for (const [sel, key] of ROW_PARTS) patchHtml(row.querySelector(sel), p[key]);
    syncControls(row, c);
  });
}
