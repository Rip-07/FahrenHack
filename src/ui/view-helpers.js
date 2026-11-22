export const $ = (s, el = document) => el.querySelector(s);
export const $$ = (s, el = document) => [...el.querySelectorAll(s)];
export const esc = s => String(s ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
export const plural = (n, one, few, many) => {
  const a = Math.abs(n) % 100, b = a % 10;
  return a > 10 && a < 20 ? many : b === 1 ? one : b >= 2 && b <= 4 ? few : many;
};
export const fmtNum = v => {
  if (v === null || v === undefined || v === '') return '';
  const s = String(v);
  return /^-?\d+$/.test(s) ? Number(s).toLocaleString('ru-RU') : s;
};
const VTYPES = [['i32', 'Целое, 4 байта'], ['i64', 'Целое, 8 байт'], ['f32', 'Дробное, 4 байта'], ['f64', 'Дробное, 8 байт']];
export const vtypeOptions = sel => VTYPES.map(([v, l]) => `<option value="${v}"${v === sel ? ' selected' : ''}>${l}</option>`).join('');

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
  chevron: '<path d="m6 9 6 6 6-6"/>',
  cup: '<path d="M4 9h13v5a5 5 0 0 1-5 5H9a5 5 0 0 1-5-5zM17 10h1.5a2.5 2.5 0 0 1 0 5H17M8 4v2M12 3v3"/>',
  pin: '<path d="M9 3h6l-1 6 4 4H6l4-4zM12 13v8"/>',
  user: '<circle cx="12" cy="8" r="4"/><path d="M4 21a8 8 0 0 1 16 0"/>',
  bag: '<path d="M5 8h14l-1 13H6zM9 8V6a3 3 0 0 1 6 0v2"/>',
  chart: '<path d="M4 20V10M10 20V4M16 20v-7M22 20H2"/>',
  gun: '<path d="M3 8h17l1 3h-6l-1 3h-3l-1 5H6l1-5H3z"/>',
  skull: '<path d="M12 3a8 8 0 0 0-5 14v3h10v-3a8 8 0 0 0-5-14z"/><circle cx="9" cy="11" r="1.5"/><circle cx="15" cy="11" r="1.5"/>',
  flag: '<path d="M5 21V4M5 4h11l-2 4 2 4H5"/>',
  run: '<circle cx="14" cy="4.5" r="2"/><path d="m7 21 3-6 3 2v4M6 11l4-3 4 1 3 3M10 8l-1 5"/>',
  flame: '<path d="M12 21a6 6 0 0 0 6-6c0-4-3-6-4-10-2 2-3 4-3 6-1-1-2-2-2-4-2 2-3 5-3 8a6 6 0 0 0 6 6z"/>',
  timer: '<circle cx="12" cy="13" r="8"/><path d="M12 9v4l3 2M10 2h4"/>',
  key: '<circle cx="8" cy="15" r="4"/><path d="m11 12 9-9M17 6l3 3M14 9l2 2"/>',
  shield: '<path d="M12 3 4 6v6c0 5 3.5 8 8 9 4.5-1 8-4 8-9V6z"/>',
  target: '<circle cx="12" cy="12" r="8"/><circle cx="12" cy="12" r="3"/><path d="M12 2v4M12 18v4M2 12h4M18 12h4"/>',
  eye: '<path d="M2 12s4-7 10-7 10 7 10 7-4 7-10 7S2 12 2 12z"/><circle cx="12" cy="12" r="3"/>',
};
export const icon = (name, cls = '') => `<svg class="ic ${cls}" viewBox="0 0 24 24" aria-hidden="true">${ICONS[name] || ICONS.star}</svg>`;

const lastHtml = new WeakMap();

/** Remembers the markup already present in a newly rendered element. */
export function rememberHtml(el, html) {
  lastHtml.set(el, html);
}

export function patchHtml(el, html) {
  if (!el || lastHtml.get(el) === html) return;
  const active = document.activeElement;
  const key = active && el.contains(active) ? active.dataset.k : null;
  el.innerHTML = html;
  rememberHtml(el, html);
  if (key) {
    const replacement = el.querySelector(`[data-k="${CSS.escape(key)}"]`);
    if (replacement) replacement.focus({ preventScroll: true });
  }
}
