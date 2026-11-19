import { esc, fmtNum, icon, patchHtml, rememberHtml } from './view-helpers.js';

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

const renderedKeys = new WeakMap();

export function renderCheatList(box, list, emptyMessage) {
  if (!list.length) {
    renderedKeys.delete(box);
    patchHtml(box, `<div class="panel empty">${esc(emptyMessage)}</div>`);
    return;
  }
  const keys = list.map(c => (c.category || 'Читы') + '\u0001' + c.id).join('\u0002');
  const parts = list.map(cheatParts);
  if (renderedKeys.get(box) !== keys) {
    let html = '', cat = null;
    list.forEach((c, i) => {
      const k = c.category || 'Читы';
      if (k !== cat) { cat = k; html += `<div class="cat">${esc(k)}</div>`; }
      html += cheatRowHtml(c, parts[i]);
    });
    patchHtml(box, html);
    renderedKeys.set(box, keys);
    list.forEach((c, i) => {
      const row = box.querySelector(`[data-row="${CSS.escape(c.id)}"]`);
      if (!row) return;
      for (const [sel, key] of ROW_PARTS) {
        rememberHtml(row.querySelector(sel), parts[i][key]);
      }
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
