// ==========================================================================
// Colours: colour picker and colour tools Chewtoy
// ==========================================================================
(function () {
  'use strict';

  // ---------------- Bridge to Brum ----------------
  // Chewtoys run sandboxed: storage, clipboard and the eyedropper go through Brum.
  let hostReqId = 0;
  const pending = new Map();
  let saved = {};
  let capabilities = {};
  let readyDone = false;
  const readyCallbacks = [];

  window.addEventListener('message', (e) => {
    if (e.source !== window.parent || !e.data || typeof e.data !== 'object') return;
    if (e.data.type === 'BRUM_RES') {
      const p = pending.get(e.data.reqId);
      if (!p) return;
      pending.delete(e.data.reqId);
      if (e.data.error) p.reject(new Error(e.data.error));
      else p.resolve(e.data.result);
    } else if ((e.data.type === 'BRUM_READY' || e.data.type === 'BRUM_CONTEXT_UPDATE') && !readyDone) {
      const ctx = e.data.context || {};
      if (ctx.storage && typeof ctx.storage === 'object') saved = Object.assign({}, ctx.storage);
      capabilities = ctx.capabilities || {};
      readyDone = true;
      readyCallbacks.splice(0).forEach((cb) => { try { cb(); } catch (err) { console.error(err); } });
    }
  });

  function host(action, payload) {
    return new Promise((resolve, reject) => {
      const reqId = 'colours-' + (++hostReqId);
      pending.set(reqId, { resolve, reject });
      window.parent.postMessage({ type: 'BRUM_REQ', reqId, action, payload }, '*');
    });
  }

  const store = {
    get: (key) => (Object.prototype.hasOwnProperty.call(saved, key) ? saved[key] : null),
    set: (key, value) => { saved[key] = String(value); host('storage.set', { key, value: String(value) }).catch(() => {}); },
  };

  function notify(message, type) {
    host('ui.notify', { message, opts: { type: type || 'info' } }).catch(() => {});
  }

  function copy(text) {
    host('clipboard.writeText', { text })
      .then(() => { notify('Copied ' + text, 'success'); addRecent(); })
      .catch((err) => notify('Could not copy: ' + err.message, 'warning'));
  }

  // ---------------- Colour maths ----------------
  const clamp = (v, lo, hi) => Math.min(hi, Math.max(lo, v));
  const round = (v, d) => { const f = Math.pow(10, d || 0); return Math.round(v * f) / f; };
  const normHue = (h) => ((h % 360) + 360) % 360;

  const toLinear = (c) => (c <= 0.04045 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4));
  const fromLinear = (c) => (c <= 0.0031308 ? 12.92 * c : 1.055 * Math.pow(c, 1 / 2.4) - 0.055);

  // sRGB (0..1, may be out of range) <-> OKLab / OKLCH (Björn Ottosson's matrices)
  function rgbToOklab([r, g, b]) {
    const lr = toLinear(r), lg = toLinear(g), lb = toLinear(b);
    const l = Math.cbrt(0.4122214708 * lr + 0.5363325363 * lg + 0.0514459929 * lb);
    const m = Math.cbrt(0.2119034982 * lr + 0.6806995451 * lg + 0.1073969566 * lb);
    const s = Math.cbrt(0.0883024619 * lr + 0.2817188376 * lg + 0.6299787005 * lb);
    return [
      0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
      1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
      0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s,
    ];
  }

  function oklabToRgb([L, a, b]) {
    const l = Math.pow(L + 0.3963377774 * a + 0.2158037573 * b, 3);
    const m = Math.pow(L - 0.1055613458 * a - 0.0638541728 * b, 3);
    const s = Math.pow(L - 0.0894841775 * a - 1.2914855480 * b, 3);
    return [
      fromLinear(4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s),
      fromLinear(-1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s),
      fromLinear(-0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s),
    ];
  }

  function rgbToOklch(rgb) {
    const [L, a, b] = rgbToOklab(rgb);
    const C = Math.sqrt(a * a + b * b);
    return [L, C, normHue(Math.atan2(b, a) * 180 / Math.PI)];
  }

  function oklchToRgb([L, C, H]) {
    const h = H * Math.PI / 180;
    return oklabToRgb([L, C * Math.cos(h), C * Math.sin(h)]);
  }

  const inGamut = (rgb) => rgb.every((c) => c >= -0.0005 && c <= 1.0005);

  // Nearest displayable colour: keep lightness and hue, lower chroma until it fits sRGB
  function gamutMap([L, C, H]) {
    if (inGamut(oklchToRgb([L, C, H]))) return oklchToRgb([L, C, H]).map((c) => clamp(c, 0, 1));
    let lo = 0, hi = C;
    for (let i = 0; i < 24; i++) {
      const mid = (lo + hi) / 2;
      if (inGamut(oklchToRgb([L, mid, H]))) lo = mid; else hi = mid;
    }
    return oklchToRgb([L, lo, H]).map((c) => clamp(c, 0, 1));
  }

  function rgbToHsl([r, g, b]) {
    const max = Math.max(r, g, b), min = Math.min(r, g, b);
    const l = (max + min) / 2;
    const d = max - min;
    if (d === 0) return [null, 0, l * 100];
    const s = d / (1 - Math.abs(2 * l - 1));
    return [hueOf(r, g, b, max, d), s * 100, l * 100];
  }

  function hueOf(r, g, b, max, d) {
    let h;
    if (max === r) h = ((g - b) / d) % 6;
    else if (max === g) h = (b - r) / d + 2;
    else h = (r - g) / d + 4;
    return normHue(h * 60);
  }

  function hslToRgb([h, s, l]) {
    s /= 100; l /= 100;
    const k = (n) => (n + h / 30) % 12;
    const a = s * Math.min(l, 1 - l);
    const f = (n) => l - a * Math.max(-1, Math.min(k(n) - 3, Math.min(9 - k(n), 1)));
    return [f(0), f(8), f(4)];
  }

  function rgbToHsv([r, g, b]) {
    const max = Math.max(r, g, b), min = Math.min(r, g, b);
    const d = max - min;
    return [d === 0 ? null : hueOf(r, g, b, max, d), max === 0 ? 0 : (d / max) * 100, max * 100];
  }

  function hsvToRgb([h, s, v]) {
    s /= 100; v /= 100;
    const f = (n) => { const k = (n + h / 60) % 6; return v - v * s * Math.max(0, Math.min(k, 4 - k, 1)); };
    return [f(5), f(3), f(1)];
  }

  function rgbToHwb(rgb) {
    const [h] = rgbToHsv(rgb);
    const w = Math.min(...rgb), bl = 1 - Math.max(...rgb);
    return [h, w * 100, bl * 100];
  }

  function hwbToRgb([h, w, b]) {
    w /= 100; b /= 100;
    if (w + b >= 1) { const g = w / (w + b); return [g, g, g]; }
    return hsvToRgb([h, (1 - w / (1 - b)) * 100, (1 - b) * 100]);
  }

  const luminance = (rgb) => {
    const [r, g, b] = rgb.map((c) => toLinear(clamp(c, 0, 1)));
    return 0.2126 * r + 0.7152 * g + 0.0722 * b;
  };

  const contrastRatio = (a, b) => {
    const [x, y] = [luminance(a), luminance(b)].sort((p, q) => q - p);
    return (x + 0.05) / (y + 0.05);
  };

  const hex2 = (v) => Math.round(clamp(v, 0, 1) * 255).toString(16).padStart(2, '0');
  const toHex = (rgb, alpha) => '#' + rgb.map(hex2).join('') + (alpha < 1 ? hex2(alpha) : '');

  // ---------------- Parsing typed or pasted colours ----------------
  function parseNumber(token, scaleForPercent) {
    const t = String(token).trim();
    if (t === 'none') return 0;
    if (t.endsWith('%')) return parseFloat(t) / 100 * scaleForPercent;
    return parseFloat(t);
  }

  function splitArgs(inner) {
    const [main, alphaPart] = inner.split('/');
    const parts = main.replace(/,/g, ' ').trim().split(/\s+/);
    let alpha = 1;
    if (alphaPart !== undefined) alpha = parseNumber(alphaPart, 1);
    else if (parts.length === 4) alpha = parseNumber(parts.pop(), 1);
    return { parts, alpha: clamp(isNaN(alpha) ? 1 : alpha, 0, 1) };
  }

  // Returns { model, values, alpha } or null
  function parseColour(text) {
    const s = String(text || '').trim().toLowerCase();
    if (!s) return null;
    let m = s.match(/^#?([0-9a-f]{3,8})$/);
    if (m && [3, 4, 6, 8].includes(m[1].length)) {
      let h = m[1];
      if (h.length <= 4) h = h.split('').map((c) => c + c).join('');
      const n = (i) => parseInt(h.slice(i, i + 2), 16) / 255;
      return { model: 'rgb', values: [n(0), n(2), n(4)].map((v) => v * 255), alpha: h.length === 8 ? n(6) : 1 };
    }
    m = s.match(/^([a-z]+)\((.*)\)$/);
    if (m) {
      const { parts, alpha } = splitArgs(m[2]);
      if (parts.length < 3) return null;
      const fn = m[1];
      const hue = (t) => { const v = String(t); return v.endsWith('turn') ? parseFloat(v) * 360 : (v.endsWith('rad') ? parseFloat(v) * 180 / Math.PI : parseFloat(v)); };
      let out = null;
      if (fn === 'rgb' || fn === 'rgba') out = { model: 'rgb', values: parts.slice(0, 3).map((p) => parseNumber(p, 255)) };
      else if (fn === 'hsl' || fn === 'hsla') out = { model: 'hsl', values: [hue(parts[0]), parseNumber(parts[1], 100), parseNumber(parts[2], 100)] };
      else if (fn === 'hwb') out = { model: 'hwb', values: [hue(parts[0]), parseNumber(parts[1], 100), parseNumber(parts[2], 100)] };
      else if (fn === 'hsv' || fn === 'hsb') out = { model: 'hsv', values: [hue(parts[0]), parseNumber(parts[1], 100), parseNumber(parts[2], 100)] };
      else if (fn === 'oklch') out = { model: 'oklch', values: [parseNumber(parts[0], 1), parseNumber(parts[1], 0.4), hue(parts[2])] };
      if (out && out.values.every((v) => Number.isFinite(v))) {
        out.alpha = alpha;
        return out;
      }
      return null;
    }
    // Named colours (rebeccapurple, …): let the browser resolve them
    if (/^[a-z]+$/.test(s)) {
      const ctx = document.createElement('canvas').getContext('2d');
      ctx.fillStyle = '#010203';
      ctx.fillStyle = s;
      if (ctx.fillStyle !== '#010203' || s === '#010203') return parseColour(ctx.fillStyle);
    }
    return null;
  }

  // ---------------- State ----------------
  // The colour is kept in the active model's own numbers, so switching models or
  // greys never loses the hue the user picked.
  const MODELS = {
    oklch: {
      channels: [
        { key: 'L', label: 'Lightness', min: 0, max: 1, step: 0.001, digits: 3 },
        { key: 'C', label: 'Chroma', min: 0, max: 0.37, step: 0.001, digits: 3 },
        { key: 'H', label: 'Hue', min: 0, max: 360, step: 0.5, digits: 1 },
      ],
      toRgb: oklchToRgb,
      fromRgb: rgbToOklch,
      hueIndex: 2,
    },
    hwb: {
      channels: [
        { key: 'H', label: 'Hue', min: 0, max: 360, step: 1, digits: 0 },
        { key: 'W', label: 'Whiteness', min: 0, max: 100, step: 0.5, digits: 1 },
        { key: 'B', label: 'Blackness', min: 0, max: 100, step: 0.5, digits: 1 },
      ],
      toRgb: hwbToRgb,
      fromRgb: rgbToHwb,
      hueIndex: 0,
    },
    hsl: {
      channels: [
        { key: 'H', label: 'Hue', min: 0, max: 360, step: 1, digits: 0 },
        { key: 'S', label: 'Saturation', min: 0, max: 100, step: 0.5, digits: 1 },
        { key: 'L', label: 'Lightness', min: 0, max: 100, step: 0.5, digits: 1 },
      ],
      toRgb: hslToRgb,
      fromRgb: rgbToHsl,
      hueIndex: 0,
    },
    hsv: {
      channels: [
        { key: 'H', label: 'Hue', min: 0, max: 360, step: 1, digits: 0 },
        { key: 'S', label: 'Saturation', min: 0, max: 100, step: 0.5, digits: 1 },
        { key: 'V', label: 'Value', min: 0, max: 100, step: 0.5, digits: 1 },
      ],
      toRgb: hsvToRgb,
      fromRgb: rgbToHsv,
      hueIndex: 0,
    },
    rgb: {
      channels: [
        { key: 'R', label: 'Red', min: 0, max: 255, step: 1, digits: 0 },
        { key: 'G', label: 'Green', min: 0, max: 255, step: 1, digits: 0 },
        { key: 'B', label: 'Blue', min: 0, max: 255, step: 1, digits: 0 },
      ],
      toRgb: (v) => v.map((c) => c / 255),
      fromRgb: (rgb) => rgb.map((c) => c * 255),
      hueIndex: -1,
    },
  };

  const state = {
    model: 'oklch',
    values: rgbToOklch([245 / 255, 158 / 255, 11 / 255]), // Brum amber, #f59e0b
    alpha: 1,
    tool: 'shades',
    contrastWith: '#ffffff',
    favourites: [],
    recent: [],
  };

  // Converts the current colour into another model, keeping the hue when it is undefined (greys)
  function valuesIn(model, rgb) {
    const def = MODELS[model];
    const v = def.fromRgb(rgb).slice();
    if (def.hueIndex >= 0) {
      const lowChroma = model === 'oklch' ? v[1] < 0.0005 : v[def.hueIndex] === null;
      if (lowChroma || v[def.hueIndex] === null) v[def.hueIndex] = currentHue();
    }
    return v;
  }

  function currentHue() {
    const def = MODELS[state.model];
    if (def.hueIndex >= 0) return state.values[def.hueIndex];
    const h = rgbToOklch(currentRgb())[2];
    return Number.isFinite(h) ? h : 0;
  }

  const currentRgb = () => MODELS[state.model].toRgb(state.values);
  const displayRgb = () => (state.model === 'oklch' ? gamutMap(state.values) : currentRgb().map((c) => clamp(c, 0, 1)));
  const currentHex = () => toHex(displayRgb(), state.alpha);

  function setColour(model, values, alpha) {
    state.model = model;
    state.values = values.slice();
    if (alpha !== undefined) state.alpha = clamp(alpha, 0, 1);
    renderAll();
    persist();
  }

  // Picks a colour given as sRGB, shown in the active model
  function setRgb(rgb, alpha) {
    setColour(state.model, valuesIn(state.model, rgb), alpha);
  }

  let persistTimer = null;
  function persist() {
    clearTimeout(persistTimer);
    persistTimer = setTimeout(() => {
      store.set('current', JSON.stringify({ model: state.model, values: state.values, alpha: state.alpha }));
    }, 300);
  }

  // ---------------- Formats ----------------
  function formats() {
    const rgb = displayRgb();
    const a = state.alpha;
    const alphaPart = a < 1 ? ` / ${round(a, 2)}` : '';
    const hsl = valuesIn('hsl', rgb), hwb = valuesIn('hwb', rgb), hsv = valuesIn('hsv', rgb);
    const ok = state.model === 'oklch' ? state.values : valuesIn('oklch', rgb);
    const r255 = rgb.map((c) => Math.round(c * 255));
    return [
      ['HEX', toHex(rgb, a)],
      ['RGB', `rgb(${r255.join(' ')}${alphaPart})`],
      ['HSL', `hsl(${round(hsl[0], 1)} ${round(hsl[1], 1)}% ${round(hsl[2], 1)}%${alphaPart})`],
      ['HWB', `hwb(${round(hwb[0], 1)} ${round(hwb[1], 1)}% ${round(hwb[2], 1)}%${alphaPart})`],
      ['OKLCH', `oklch(${round(ok[0], 3)} ${round(ok[1], 3)} ${round(ok[2], 1)}${alphaPart})`],
      ['HSV', `hsv(${round(hsv[0], 1)} ${round(hsv[1], 1)}% ${round(hsv[2], 1)}%)`],
    ];
  }

  // ---------------- Rendering ----------------
  const $ = (id) => document.getElementById(id);
  const el = (tag, cls, text) => {
    const n = document.createElement(tag);
    if (cls) n.className = cls;
    if (text !== undefined) n.textContent = text;
    return n;
  };

  function renderPreview() {
    const rgb = displayRgb();
    $('col-preview-fill').style.background = `rgba(${rgb.map((c) => Math.round(c * 255)).join(',')},${state.alpha})`;
    $('col-gamut-badge').hidden = !(state.model === 'oklch' && !inGamut(oklchToRgb(state.values)));
    const input = $('col-input');
    if (document.activeElement !== input) {
      input.value = currentHex();
      input.classList.remove('invalid');
    }
    const isFav = state.favourites.includes(toHex(rgb, state.alpha));
    $('col-fav-toggle').classList.toggle('active', isFav);
    $('col-fav-toggle').title = isFav ? 'Remove from saved colours' : 'Save this colour';
  }

  function trackGradient(index) {
    const def = MODELS[state.model];
    const ch = def.channels[index];
    const stops = [];
    const N = 24;
    for (let i = 0; i <= N; i++) {
      const v = state.values.slice();
      v[index] = ch.min + (ch.max - ch.min) * i / N;
      const rgb = def.toRgb(v);
      // Gaps mark colours a normal screen cannot show (OKLCH only)
      const c = inGamut(rgb) ? `rgb(${rgb.map((x) => Math.round(clamp(x, 0, 1) * 255)).join(',')})` : 'rgba(63,63,70,0.55)';
      stops.push(`${c} ${round(i / N * 100, 2)}%`);
    }
    return `linear-gradient(to right, ${stops.join(', ')})`;
  }

  function alphaGradient() {
    const rgb = displayRgb().map((c) => Math.round(c * 255)).join(',');
    return `linear-gradient(to right, rgba(${rgb},0), rgba(${rgb},1))`;
  }

  let slidersModel = null;
  function renderSliders() {
    const box = $('col-sliders');
    const def = MODELS[state.model];
    if (slidersModel !== state.model) {
      slidersModel = state.model;
      box.textContent = '';
      const rows = def.channels.map((ch, i) => ({ ...ch, i })).concat([{ key: 'A', label: 'Alpha', min: 0, max: 100, step: 1, digits: 0, i: 3 }]);
      rows.forEach((ch) => {
        const row = el('div', 'col-slider-row');
        row.appendChild(el('span', 'col-slider-label', ch.label));
        const track = el('div', 'col-track');
        const fill = el('div', 'col-track-fill');
        fill.id = `col-fill-${ch.i}`;
        const range = el('input');
        range.type = 'range';
        range.min = ch.min; range.max = ch.max; range.step = ch.step;
        range.id = `col-range-${ch.i}`;
        range.setAttribute('aria-label', ch.label);
        track.append(fill, range);
        const num = el('input', 'col-num');
        num.type = 'number';
        num.min = ch.min; num.max = ch.max; num.step = ch.step;
        num.id = `col-num-${ch.i}`;
        num.setAttribute('aria-label', ch.label + ' value');
        const apply = (raw) => {
          const v = clamp(parseFloat(raw), ch.min, ch.max);
          if (!Number.isFinite(v)) return;
          if (ch.i === 3) { state.alpha = v / 100; renderAll(); persist(); return; }
          const values = state.values.slice();
          values[ch.i] = v;
          setColour(state.model, values);
        };
        range.addEventListener('input', () => apply(range.value));
        num.addEventListener('change', () => apply(num.value));
        row.append(track, num);
        box.appendChild(row);
      });
    }
    def.channels.forEach((ch, i) => {
      $(`col-fill-${i}`).style.background = trackGradient(i);
      $(`col-range-${i}`).value = state.values[i];
      if (document.activeElement !== $(`col-num-${i}`)) $(`col-num-${i}`).value = round(state.values[i], ch.digits);
    });
    $('col-fill-3').style.background = alphaGradient();
    $('col-range-3').value = Math.round(state.alpha * 100);
    if (document.activeElement !== $('col-num-3')) $('col-num-3').value = Math.round(state.alpha * 100);
    document.querySelectorAll('#col-model-tabs .col-tab').forEach((t) => t.classList.toggle('active', t.dataset.model === state.model));
  }

  function renderFormats() {
    const box = $('col-formats');
    box.textContent = '';
    formats().forEach(([label, value]) => {
      const row = el('div', 'col-format-row');
      row.appendChild(el('span', 'col-format-label', label + (label === 'HSV' ? '*' : '')));
      const v = el('span', 'col-format-value', value);
      v.title = value;
      row.appendChild(v);
      const btn = copyButton(value, `Copy ${label}`);
      row.appendChild(btn);
      box.appendChild(row);
    });
    const hint = el('div', 'col-hint', '* HSV is not a CSS colour format.');
    box.appendChild(hint);
  }

  function copyButton(text, title) {
    const btn = el('button', 'btn btn-icon btn-xs');
    btn.type = 'button';
    btn.title = title;
    btn.innerHTML = '<svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect width="14" height="14" x="8" y="8" rx="2"/><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/></svg>';
    btn.addEventListener('click', () => copy(text));
    return btn;
  }

  function swatch(rgb, title, onPick) {
    const s = el('button', 'col-swatch');
    s.type = 'button';
    const hex = toHex(rgb, 1);
    s.style.background = hex;
    s.title = title ? `${title}: ${hex}` : hex;
    s.setAttribute('aria-label', s.title);
    if (hex === toHex(displayRgb(), 1)) s.classList.add('current');
    s.addEventListener('click', onPick || (() => setRgb(rgb)));
    return s;
  }

  // ---------------- Tools ----------------
  function okBase() {
    return state.model === 'oklch' ? state.values : valuesIn('oklch', displayRgb());
  }

  function renderShades(body) {
    const [L, C, H] = okBase();
    const steps = [0.97, 0.9, 0.8, 0.7, 0.6, 0.5, 0.4, 0.3, 0.2, 0.12];
    body.appendChild(el('div', 'col-swatch-label', 'Tints and shades (same hue, lighter to darker)'));
    const row = el('div', 'col-swatch-row');
    steps.forEach((l) => row.appendChild(swatch(gamutMap([l, C * Math.min(1, 1.2 - Math.abs(l - L)), H]), `Lightness ${Math.round(l * 100)}%`)));
    body.appendChild(row);
    body.appendChild(el('div', 'col-swatch-label', 'Saturation (same lightness, grey to full)'));
    const row2 = el('div', 'col-swatch-row');
    for (let i = 0; i <= 8; i++) row2.appendChild(swatch(gamutMap([L, 0.32 * i / 8, H]), `Chroma ${round(0.32 * i / 8, 2)}`));
    body.appendChild(row2);
  }

  function renderHarmony(body) {
    const [L, C, H] = okBase();
    const sets = [
      ['Complementary', [0, 180]],
      ['Analogous', [-30, 0, 30]],
      ['Triadic', [0, 120, 240]],
      ['Split complementary', [0, 150, 210]],
      ['Tetradic', [0, 90, 180, 270]],
    ];
    sets.forEach(([name, offsets]) => {
      body.appendChild(el('div', 'col-swatch-label', name));
      const row = el('div', 'col-swatch-row');
      offsets.forEach((o) => row.appendChild(swatch(gamutMap([L, C, normHue(H + o)]), `${name} ${o >= 0 ? '+' : ''}${o}°`)));
      body.appendChild(row);
    });
    body.appendChild(el('div', 'col-hint', 'Hues are rotated in OKLCH, so the colours keep the same perceived lightness.'));
  }

  function contrastCard(fg, bg, label) {
    const card = el('div', 'col-contrast-card');
    const sample = el('div', 'col-contrast-sample');
    sample.style.color = toHex(fg, 1);
    sample.style.background = toHex(bg, 1);
    sample.textContent = label;
    const small = el('small', null, 'The quick brown fox jumps over the lazy dog');
    sample.appendChild(small);
    const ratio = contrastRatio(fg, bg);
    const r = el('div', 'col-contrast-ratio', `${round(ratio, 2)}:1`);
    const badges = el('div', 'col-badges');
    [['AA', 4.5], ['AA large', 3], ['AAA', 7], ['AAA large', 4.5], ['UI', 3]].forEach(([n, min]) => {
      badges.appendChild(el('span', `col-badge ${ratio >= min ? 'pass' : 'fail'}`, `${ratio >= min ? '✓' : '✗'} ${n}`));
    });
    card.append(sample, r, badges);
    return card;
  }

  function renderContrast(body) {
    const rgb = displayRgb();
    body.appendChild(contrastCard(rgb, [1, 1, 1], 'On white'));
    body.appendChild(contrastCard(rgb, [0, 0, 0], 'On black'));
    const row = el('div', 'col-row');
    row.appendChild(el('span', 'col-slider-label', 'Compare with'));
    const input = el('input', 'col-text-input');
    input.value = state.contrastWith;
    input.setAttribute('aria-label', 'Second colour');
    row.appendChild(input);
    body.appendChild(row);
    const other = parseColour(state.contrastWith);
    const holder = el('div');
    if (other) holder.appendChild(contrastCard(MODELS[other.model].toRgb(other.values).map((c) => clamp(c, 0, 1)), rgb, 'Text in that colour'));
    body.appendChild(holder);
    input.addEventListener('change', () => {
      const p = parseColour(input.value);
      input.classList.toggle('invalid', !p);
      if (!p) return;
      state.contrastWith = input.value.trim();
      store.set('contrastWith', state.contrastWith);
      renderTool();
    });
    body.appendChild(el('div', 'col-hint', 'WCAG 2 ratios. AA needs 4.5:1 for normal text and 3:1 for large text or interface parts; AAA needs 7:1. Transparency is ignored.'));
  }

  function hexToRgbAlpha(hex) {
    const p = parseColour(hex);
    return p ? [MODELS.rgb.toRgb(p.values), p.alpha] : null;
  }

  function renderSaved(body) {
    const section = (title, list, emptyText, removable) => {
      body.appendChild(el('div', 'col-swatch-label', title));
      if (!list.length) { body.appendChild(el('div', 'col-empty', emptyText)); return; }
      const grid = el('div', 'col-swatch-grid');
      list.forEach((hex) => {
        const parsed = hexToRgbAlpha(hex);
        if (!parsed) return;
        const s = swatch(parsed[0], hex, () => setRgb(parsed[0], parsed[1]));
        if (removable) {
          s.title += ' (right-click to remove)';
          s.addEventListener('contextmenu', (e) => {
            e.preventDefault();
            state.favourites = state.favourites.filter((h) => h !== hex);
            store.set('favourites', JSON.stringify(state.favourites));
            renderAll();
          });
        }
        grid.appendChild(s);
      });
      body.appendChild(grid);
    };
    section('Saved colours', state.favourites, 'Nothing saved yet. Use the star next to the colour box.', true);
    section('Recently copied', state.recent, 'Colours you copy show up here.', false);
  }

  let brumPalette = null;
  function renderBrum(body) {
    body.appendChild(el('div', 'col-swatch-label', 'Colours of the current Brum theme'));
    if (!brumPalette) {
      body.appendChild(el('div', 'col-empty', 'Loading theme colours…'));
      host('ui.getTheme').then((t) => { brumPalette = (t && t.palette) || {}; renderTool(); }).catch(() => { brumPalette = {}; renderTool(); });
    } else {
      const grid = el('div', 'col-swatch-grid');
      Object.entries(brumPalette).forEach(([name, value]) => {
        const p = parseColour(value);
        if (!p) return;
        const rgb = MODELS[p.model].toRgb(p.values).map((c) => clamp(c, 0, 1));
        grid.appendChild(swatch(rgb, name, () => setRgb(rgb, p.alpha)));
      });
      body.appendChild(grid.childElementCount ? grid : el('div', 'col-empty', 'No theme colours available.'));
    }
    body.appendChild(el('div', 'col-swatch-label', 'Copy as a theme variable'));
    const row = el('div', 'col-row');
    const name = el('input', 'col-text-input');
    name.value = store.get('varName') || '--accent';
    name.setAttribute('aria-label', 'Variable name');
    const btn = el('button', 'btn', 'Copy');
    btn.type = 'button';
    btn.addEventListener('click', () => {
      let n = name.value.trim() || '--accent';
      if (!n.startsWith('--')) n = '--' + n.replace(/^-+/, '');
      store.set('varName', n);
      copy(`${n}: ${currentHex()};`);
    });
    row.append(name, btn);
    body.appendChild(row);
  }

  const TOOLS = { shades: renderShades, harmony: renderHarmony, contrast: renderContrast, saved: renderSaved, brum: renderBrum };

  function renderTool() {
    const body = $('col-tool-body');
    // Keep typing in the contrast field undisturbed
    if (body.contains(document.activeElement) && document.activeElement.tagName === 'INPUT') return;
    body.textContent = '';
    (TOOLS[state.tool] || renderShades)(body);
    document.querySelectorAll('#col-tool-tabs .col-tab').forEach((t) => t.classList.toggle('active', t.dataset.tool === state.tool));
  }

  function renderAll() {
    renderPreview();
    renderSliders();
    renderFormats();
    renderTool();
  }

  function addRecent() {
    const hex = currentHex();
    state.recent = [hex].concat(state.recent.filter((h) => h !== hex)).slice(0, 16);
    store.set('recent', JSON.stringify(state.recent));
    if (state.tool === 'saved') renderTool();
  }

  // ---------------- Events ----------------
  document.querySelectorAll('#col-model-tabs .col-tab').forEach((tab) => {
    tab.addEventListener('click', () => {
      const model = tab.dataset.model;
      if (model === state.model) return;
      // Convert from what is on screen, so out-of-gamut OKLCH values become their nearest colour
      setColour(model, valuesIn(model, displayRgb()));
      store.set('model', model);
    });
  });

  document.querySelectorAll('#col-tool-tabs .col-tab').forEach((tab) => {
    tab.addEventListener('click', () => {
      state.tool = tab.dataset.tool;
      store.set('tool', state.tool);
      renderTool();
    });
  });

  const input = $('col-input');
  const applyInput = () => {
    const p = parseColour(input.value);
    input.classList.toggle('invalid', !p);
    if (!p) return;
    const rgb = MODELS[p.model].toRgb(p.values);
    if (p.model === state.model) setColour(p.model, p.values, p.alpha);
    else setColour(state.model, valuesIn(state.model, rgb), p.alpha);
  };
  input.addEventListener('change', applyInput);
  input.addEventListener('keydown', (e) => { if (e.key === 'Enter') { applyInput(); input.select(); } });
  input.addEventListener('paste', () => setTimeout(applyInput, 0));
  input.addEventListener('focus', () => input.select());

  $('col-fav-toggle').addEventListener('click', () => {
    const hex = currentHex();
    if (state.favourites.includes(hex)) state.favourites = state.favourites.filter((h) => h !== hex);
    else state.favourites = [hex].concat(state.favourites).slice(0, 48);
    store.set('favourites', JSON.stringify(state.favourites));
    renderAll();
  });

  $('col-eyedropper').addEventListener('click', () => {
    host('ui.pickColor')
      .then((hex) => {
        if (!hex) return;
        const p = parseColour(hex);
        if (p) setRgb(MODELS.rgb.toRgb(p.values), 1);
      })
      .catch((err) => notify('Eyedropper: ' + err.message, 'warning'));
  });

  // ---------------- Start ----------------
  function loadSaved() {
    const json = (key, fallback) => { try { const v = JSON.parse(store.get(key)); return v === null ? fallback : v; } catch (_) { return fallback; } };
    const cur = json('current', null);
    if (cur && MODELS[cur.model] && Array.isArray(cur.values) && cur.values.length === 3 && cur.values.every(Number.isFinite)) {
      state.model = cur.model;
      state.values = cur.values;
      state.alpha = Number.isFinite(cur.alpha) ? clamp(cur.alpha, 0, 1) : 1;
    }
    state.tool = TOOLS[store.get('tool')] ? store.get('tool') : state.tool;
    state.contrastWith = store.get('contrastWith') || state.contrastWith;
    const fav = json('favourites', []), rec = json('recent', []);
    state.favourites = Array.isArray(fav) ? fav.filter((h) => typeof h === 'string') : [];
    state.recent = Array.isArray(rec) ? rec.filter((h) => typeof h === 'string') : [];
    $('col-eyedropper').hidden = !capabilities.eyedropper;
    slidersModel = null;
    renderAll();
  }

  renderAll();
  readyCallbacks.push(loadSaved);

  // Test hook for the developer tools: the colour maths, without UI
  window.ColoursMath = { parseColour, rgbToOklch, oklchToRgb, rgbToHsl, hslToRgb, rgbToHwb, hwbToRgb, rgbToHsv, hsvToRgb, gamutMap, contrastRatio, toHex, state, formats };
})();
