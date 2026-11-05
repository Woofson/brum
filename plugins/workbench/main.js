// ==========================================================================
// Workbench: workshop conversions and calculators (offline, no permissions
// beyond notifications and the clipboard)
// ==========================================================================
(function () {
  'use strict';

  // ---------------- Bridge to Brum ----------------
  let hostReqId = 0;
  const pending = new Map();
  let saved = {};
  let region = {};
  let readyDone = false;

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
      region = ctx.region || {};
      readyDone = true;
      start();
    }
  });

  function host(action, payload) {
    return new Promise((resolve, reject) => {
      const reqId = 'wb-' + (++hostReqId);
      pending.set(reqId, { resolve, reject });
      window.parent.postMessage({ type: 'BRUM_REQ', reqId, action, payload }, '*');
    });
  }

  const store = {
    get: (k) => (Object.prototype.hasOwnProperty.call(saved, k) ? saved[k] : null),
    set: (k, v) => { saved[k] = String(v); host('storage.set', { key: k, value: String(v) }).catch(() => {}); },
    json: (k, dflt) => { try { const v = JSON.parse(store.get(k) || 'null'); return v === null ? dflt : v; } catch (_) { return dflt; } },
  };

  function copy(text) {
    host('clipboard.writeText', { text })
      .then(() => host('ui.notify', { message: 'Copied ' + text, opts: { type: 'success' } }).catch(() => {}))
      .catch((err) => host('ui.notify', { message: 'Could not copy: ' + err.message, opts: { type: 'warning' } }).catch(() => {}));
  }

  const $ = (id) => document.getElementById(id);
  const esc = (s) => String(s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
  const imperial = () => region.units === 'imperial';

  // ---------------- Numbers in the user's style (Settings → Region & Formats) ----------------
  const NUMBER_STYLES = { space_comma: [' ', ','], dot_comma: ['.', ','], comma_dot: [',', '.'], space_dot: [' ', '.'], apostrophe_dot: ['’', '.'] };

  function fmt(n, digits = 4) {
    if (n === null || n === undefined || !isFinite(n)) return '—';
    const abs = Math.abs(n);
    // Very large or tiny numbers read better in scientific form
    if (abs !== 0 && (abs >= 1e12 || abs < 1e-6)) return n.toExponential(3).replace('.', (NUMBER_STYLES[region.number] || [',', '.'])[1]);
    const d = abs >= 1000 ? Math.min(digits, 2) : digits;
    const style = NUMBER_STYLES[region.number];
    if (!style) return n.toLocaleString(undefined, { maximumFractionDigits: d });
    const [whole, frac] = Math.abs(n).toFixed(d).replace(/\.?0+$/, '').split('.');
    const grouped = whole.replace(/\B(?=(\d{3})+(?!\d))/g, style[0]);
    return (n < 0 ? '−' : '') + grouped + (frac ? style[1] + frac : '');
  }

  // Accepts 1.5, 1,5, 1 234,5, 1/4, 1 1/4, 1-1/4, 4k7 style is handled by the resistor tool
  function num(s) {
    if (s === null || s === undefined) return NaN;
    let t = String(s).trim().replace(/[  ’']/g, '').replace(/−/g, '-');
    if (!t) return NaN;
    const mixed = /^(-?\d+)[\s-]+(\d+)\/(\d+)$/.exec(t);
    if (mixed) return Math.sign(+mixed[1] || 1) * (Math.abs(+mixed[1]) + mixed[2] / mixed[3]);
    const frac = /^(-?\d+)\/(\d+)$/.exec(t);
    if (frac) return frac[1] / frac[2];
    t = t.replace(/\s/g, '');
    if (t.includes(',') && t.includes('.')) {
      // The last separator is the decimal one
      t = t.lastIndexOf(',') > t.lastIndexOf('.') ? t.replace(/\./g, '').replace(',', '.') : t.replace(/,/g, '');
    } else {
      t = t.replace(',', '.');
    }
    return /^-?(\d+\.?\d*|\.\d+)(e-?\d+)?$/i.test(t) ? parseFloat(t) : NaN;
  }

  // ---------------- Small form helpers ----------------
  function rowsHtml(rows) {
    return rows.map(([k, v, note, big]) => `<div class="wb-k">${esc(k)}</div><div class="wb-v${big ? ' wb-big' : ''}" data-copy="${esc(String(v).replace(/<[^>]+>/g, ''))}" title="Click to copy">${esc(v)}${note ? `<small>${esc(note)}</small>` : ''}</div>`).join('');
  }

  function field(id, label, opts = {}) {
    if (opts.options) {
      return `<div class="wb-field"><label for="${id}">${esc(label)}</label><select id="${id}" class="wb-select">${opts.options.map((o) => {
        const [v, t] = Array.isArray(o) ? o : [o, o];
        return `<option value="${esc(v)}">${esc(t)}</option>`;
      }).join('')}</select></div>`;
    }
    return `<div class="wb-field"><label for="${id}">${esc(label)}</label><input id="${id}" class="wb-input" inputmode="decimal" autocomplete="off" placeholder="${esc(opts.placeholder || '')}"></div>`;
  }

  // Remembers a tool's inputs between visits; calling it again only fills in fields added since
  function persist(tool, el) {
    const key = 'in:' + tool.id;
    const values = store.json(key, {});
    el.querySelectorAll('input[id], select[id]').forEach((i) => {
      if (Object.prototype.hasOwnProperty.call(values, i.id)) i.value = values[i.id];
    });
    if (el.dataset.persist) return;
    el.dataset.persist = '1';
    let timer;
    el.addEventListener('input', () => {
      clearTimeout(timer);
      timer = setTimeout(() => {
        const v = {};
        el.querySelectorAll('input[id], select[id]').forEach((i) => { if (i.type !== 'search') v[i.id] = i.value; });
        store.set(key, JSON.stringify(v));
      }, 600);
    });
  }

  function seg(id, items, value) {
    return `<div class="wb-seg" id="${id}">${items.map(([v, t]) => `<button type="button" data-v="${esc(v)}" class="${v === value ? 'active' : ''}">${esc(t)}</button>`).join('')}</div>`;
  }

  function wireSeg(el, id, onChange) {
    const box = el.querySelector('#' + id);
    box.addEventListener('click', (e) => {
      const b = e.target.closest('button[data-v]');
      if (!b) return;
      box.querySelectorAll('button').forEach((x) => x.classList.toggle('active', x === b));
      onChange(b.dataset.v);
    });
  }

  function tableHtml(head, rows, hitIndex = -1) {
    return `<div class="wb-table-wrap"><table class="wb-table"><thead><tr>${head.map((h) => `<th>${esc(h)}</th>`).join('')}</tr></thead><tbody>${rows.map((r, i) => `<tr class="${i === hitIndex ? 'hit' : ''}">${r.map((c) => `<td data-copy="${esc(c)}">${esc(c)}</td>`).join('')}</tr>`).join('')}</tbody></table></div>`;
  }

  // ==========================================================================
  // Data
  // ==========================================================================

  // Unit → factor to the base unit (first unit of each group is the base)
  const UNITS = {
    length: { label: 'Length', units: [['m', 'm', 1], ['mm', 'mm', 1e-3], ['cm', 'cm', 1e-2], ['km', 'km', 1e3], ['um', 'µm', 1e-6], ['in', 'inch', 0.0254], ['ft', 'foot', 0.3048], ['yd', 'yard', 0.9144], ['mi', 'mile', 1609.344], ['thou', 'thou (mil)', 0.0000254], ['nmi', 'nautical mile', 1852]], metric: ['mm', 'in'], imperial: ['in', 'mm'] },
    area: { label: 'Area', units: [['m2', 'm²', 1], ['mm2', 'mm²', 1e-6], ['cm2', 'cm²', 1e-4], ['ha', 'hectare', 1e4], ['km2', 'km²', 1e6], ['in2', 'in²', 0.00064516], ['ft2', 'ft²', 0.09290304], ['yd2', 'yd²', 0.83612736], ['acre', 'acre', 4046.8564224]], metric: ['m2', 'ft2'], imperial: ['ft2', 'm2'] },
    volume: { label: 'Volume', units: [['l', 'litre', 1], ['ml', 'ml', 1e-3], ['m3', 'm³', 1000], ['cm3', 'cm³', 1e-3], ['in3', 'in³', 0.016387064], ['ft3', 'ft³', 28.316846592], ['usgal', 'US gallon', 3.785411784], ['ukgal', 'UK gallon', 4.54609], ['usqt', 'US quart', 0.946352946], ['uspt', 'US pint', 0.473176473], ['ukpt', 'UK pint', 0.56826125], ['usfloz', 'US fl oz', 0.0295735295625]], metric: ['l', 'usgal'], imperial: ['usgal', 'l'] },
    mass: { label: 'Weight / mass', units: [['kg', 'kg', 1], ['g', 'g', 1e-3], ['t', 'tonne', 1000], ['oz', 'ounce', 0.028349523125], ['lb', 'pound', 0.45359237], ['st', 'stone', 6.35029318], ['ston', 'US ton', 907.18474]], metric: ['kg', 'lb'], imperial: ['lb', 'kg'] },
    pressure: { label: 'Pressure', units: [['pa', 'Pa', 1], ['kpa', 'kPa', 1e3], ['mpa', 'MPa', 1e6], ['bar', 'bar', 1e5], ['mbar', 'mbar', 100], ['psi', 'psi', 6894.757293168], ['atm', 'atm', 101325], ['mmhg', 'mmHg', 133.322387415], ['inhg', 'inHg', 3386.388640341], ['kgfcm2', 'kgf/cm²', 98066.5]], metric: ['bar', 'psi'], imperial: ['psi', 'bar'] },
    torque: { label: 'Torque', units: [['nm', 'N·m', 1], ['kgfm', 'kgf·m', 9.80665], ['lbfft', 'lbf·ft', 1.3558179483314], ['lbfin', 'lbf·in', 0.1129848290276], ['ozfin', 'ozf·in', 0.0070615518142]], metric: ['nm', 'lbfft'], imperial: ['lbfft', 'nm'] },
    temperature: { label: 'Temperature', units: [['c', '°C'], ['f', '°F'], ['k', 'K']], metric: ['c', 'f'], imperial: ['f', 'c'] },
    speed: { label: 'Speed', units: [['ms', 'm/s', 1], ['kmh', 'km/h', 1 / 3.6], ['mph', 'mph', 0.44704], ['kn', 'knot', 1852 / 3600], ['fts', 'ft/s', 0.3048], ['ftmin', 'ft/min', 0.00508]], metric: ['kmh', 'mph'], imperial: ['mph', 'kmh'] },
    flow: { label: 'Flow', units: [['lmin', 'l/min', 1], ['lh', 'l/h', 1 / 60], ['m3h', 'm³/h', 1000 / 60], ['usgpm', 'US gal/min', 3.785411784], ['ukgpm', 'UK gal/min', 4.54609], ['cfm', 'cfm (ft³/min)', 28.316846592]], metric: ['lmin', 'usgpm'], imperial: ['usgpm', 'lmin'] },
    power: { label: 'Power', units: [['w', 'W', 1], ['kw', 'kW', 1000], ['hp', 'hp (mechanical)', 745.69987158227], ['ps', 'PS (metric hp)', 735.49875], ['btuh', 'BTU/h', 0.29307107]], metric: ['kw', 'hp'], imperial: ['hp', 'kw'] },
    energy: { label: 'Energy', units: [['j', 'J', 1], ['kj', 'kJ', 1e3], ['wh', 'Wh', 3600], ['kwh', 'kWh', 3.6e6], ['cal', 'kcal', 4184], ['btu', 'BTU', 1055.05585262]], metric: ['kwh', 'btu'], imperial: ['btu', 'kwh'] },
  };

  function toBase(cat, unit, v) {
    if (cat === 'temperature') return unit === 'c' ? v : unit === 'f' ? (v - 32) * 5 / 9 : v - 273.15;
    return v * UNITS[cat].units.find((u) => u[0] === unit)[2];
  }

  function fromBase(cat, unit, v) {
    if (cat === 'temperature') return unit === 'c' ? v : unit === 'f' ? v * 9 / 5 + 32 : v + 273.15;
    return v / UNITS[cat].units.find((u) => u[0] === unit)[2];
  }

  // AWG: diameter 0.127 mm × 92^((36 − n) / 39); 0000 = −3 … 0 = 0.
  // Current ratings: the widely used PowerStream table (chassis wiring / power transmission).
  const AWG_AMPS = {
    '-3': [380, 302], '-2': [328, 239], '-1': [283, 190], 0: [245, 150], 1: [211, 119], 2: [181, 95], 3: [158, 75], 4: [135, 60], 5: [118, 47], 6: [101, 37], 7: [89, 30], 8: [73, 24], 9: [64, 19], 10: [55, 15], 11: [47, 12], 12: [41, 9.3], 13: [35, 7.4], 14: [32, 5.9], 15: [28, 4.7], 16: [22, 3.7], 17: [19, 2.9], 18: [16, 2.3], 19: [14, 1.8], 20: [11, 1.5], 21: [9, 1.2], 22: [7, 0.92], 23: [4.7, 0.729], 24: [3.5, 0.577], 25: [2.7, 0.457], 26: [2.2, 0.361], 27: [1.7, 0.288], 28: [1.4, 0.226], 29: [1.2, 0.182], 30: [0.86, 0.142], 31: [0.7, 0.113], 32: [0.53, 0.091], 33: [0.43, 0.072], 34: [0.33, 0.056], 35: [0.27, 0.044], 36: [0.21, 0.035], 37: [0.17, 0.0289], 38: [0.13, 0.0228], 39: [0.11, 0.0175], 40: [0.09, 0.0137],
  };
  const awgName = (n) => (n < 0 ? '0'.repeat(1 - n) : String(n));
  const awgDiameter = (n) => 0.127 * Math.pow(92, (36 - n) / 39);
  const awgArea = (n) => Math.PI * Math.pow(awgDiameter(n), 2) / 4;
  const RHO = { copper: 1.724e-8, aluminium: 2.82e-8 }; // Ω·m at 20 °C (annealed copper, EC aluminium)

  // Metric threads (ISO 261/262): tap drill = diameter − pitch (about 75 % thread). Charts give
  // the drill you can buy: from M4 up a half tenth goes to the even tenth (6.75 → 6.8, 10.25 → 10.2).
  function metricTapDrill(d, p) {
    const exact = Math.round((d - p) * 1000) / 1000;
    if (d < 4) return exact;
    const tenths = exact * 10;
    const floor = Math.floor(tenths + 1e-9);
    const rest = tenths - floor;
    let r;
    if (Math.abs(rest - 0.5) < 1e-6) r = floor % 2 === 0 ? floor : floor + 1;
    else r = Math.round(tenths);
    return r / 10;
  }
  const METRIC_COARSE = [[1.6, 0.35], [2, 0.4], [2.5, 0.45], [3, 0.5], [3.5, 0.6], [4, 0.7], [5, 0.8], [6, 1], [8, 1.25], [10, 1.5], [12, 1.75], [14, 2], [16, 2], [18, 2.5], [20, 2.5], [22, 2.5], [24, 3], [27, 3], [30, 3.5], [33, 3.5], [36, 4]];
  const METRIC_FINE = [[3, 0.35], [4, 0.5], [5, 0.5], [6, 0.75], [8, 0.75], [8, 1], [10, 0.75], [10, 1], [10, 1.25], [12, 1], [12, 1.25], [12, 1.5], [14, 1.25], [14, 1.5], [16, 1], [16, 1.5], [18, 1.5], [18, 2], [20, 1], [20, 1.5], [20, 2], [22, 1.5], [22, 2], [24, 1.5], [24, 2], [27, 2], [30, 2]];
  // ISO 273 clearance holes: close / medium / loose
  const CLEARANCE = { 1.6: [1.7, 1.8, 2], 2: [2.2, 2.4, 2.6], 2.5: [2.7, 2.9, 3.1], 3: [3.2, 3.4, 3.6], 3.5: [3.7, 3.9, 4.2], 4: [4.3, 4.5, 4.8], 5: [5.3, 5.5, 5.8], 6: [6.4, 6.6, 7], 8: [8.4, 9, 10], 10: [10.5, 11, 12], 12: [13, 13.5, 14.5], 14: [15, 15.5, 16.5], 16: [17, 17.5, 18.5], 18: [19, 20, 21], 20: [21, 22, 24], 22: [23, 24, 26], 24: [25, 26, 28], 27: [28, 30, 32], 30: [31, 33, 35], 33: [34, 36, 38], 36: [37, 39, 42] };
  // Unified threads: size, major diameter (in), TPI, standard tap drill (75 %)
  const UNIFIED = [
    ['#0', 0.06, 80, null, '3/64'], ['#1', 0.073, 64, 'UNC', '#53'], ['#1', 0.073, 72, 'UNF', '#53'], ['#2', 0.086, 56, 'UNC', '#50'], ['#2', 0.086, 64, 'UNF', '#50'],
    ['#3', 0.099, 48, 'UNC', '#47'], ['#3', 0.099, 56, 'UNF', '#45'], ['#4', 0.112, 40, 'UNC', '#43'], ['#4', 0.112, 48, 'UNF', '#42'], ['#5', 0.125, 40, 'UNC', '#38'], ['#5', 0.125, 44, 'UNF', '#37'],
    ['#6', 0.138, 32, 'UNC', '#36'], ['#6', 0.138, 40, 'UNF', '#33'], ['#8', 0.164, 32, 'UNC', '#29'], ['#8', 0.164, 36, 'UNF', '#29'], ['#10', 0.19, 24, 'UNC', '#25'], ['#10', 0.19, 32, 'UNF', '#21'],
    ['#12', 0.216, 24, 'UNC', '#16'], ['#12', 0.216, 28, 'UNF', '#14'], ['1/4', 0.25, 20, 'UNC', '#7'], ['1/4', 0.25, 28, 'UNF', '#3'], ['5/16', 0.3125, 18, 'UNC', 'F'], ['5/16', 0.3125, 24, 'UNF', 'I'],
    ['3/8', 0.375, 16, 'UNC', '5/16'], ['3/8', 0.375, 24, 'UNF', 'Q'], ['7/16', 0.4375, 14, 'UNC', 'U'], ['7/16', 0.4375, 20, 'UNF', '25/64'], ['1/2', 0.5, 13, 'UNC', '27/64'], ['1/2', 0.5, 20, 'UNF', '29/64'],
    ['9/16', 0.5625, 12, 'UNC', '31/64'], ['9/16', 0.5625, 18, 'UNF', '33/64'], ['5/8', 0.625, 11, 'UNC', '17/32'], ['5/8', 0.625, 18, 'UNF', '37/64'], ['3/4', 0.75, 10, 'UNC', '21/32'], ['3/4', 0.75, 16, 'UNF', '11/16'],
    ['7/8', 0.875, 9, 'UNC', '49/64'], ['7/8', 0.875, 14, 'UNF', '13/16'], ['1', 1, 8, 'UNC', '7/8'], ['1', 1, 12, 'UNF', '59/64'], ['1', 1, 14, 'UNS', '15/16'],
  ];
  UNIFIED[0][3] = 'UNF';
  // Pipe threads: size, TPI, BSP parallel (G) tap drill mm, BSP taper (R) mm, NPT tap drill
  const PIPE = [['1/16', null, null, null, '1/4', 27], ['1/8', 28, 8.8, 8.4, 'R (8.6 mm)', 27], ['1/4', 19, 11.8, 11.2, '7/16', 18], ['3/8', 19, 15.25, 14.75, '37/64', 18], ['1/2', 14, 19, 18.25, '23/32', 14], ['3/4', 14, 24.5, 24, '59/64', 14], ['1', 11, 30.75, 30.25, '1 5/32', 11.5], ['1 1/4', 11, 39.5, 39, '1 1/2', 11.5], ['1 1/2', 11, 45.25, 45, '1 47/64', 11.5], ['2', 11, 57, 56.75, '2 7/32', 11.5]];

  // Drill sizes in inches
  const NUMBER_DRILLS = [0.228, 0.221, 0.213, 0.209, 0.2055, 0.204, 0.201, 0.199, 0.196, 0.1935, 0.191, 0.189, 0.185, 0.182, 0.18, 0.177, 0.173, 0.1695, 0.166, 0.161, 0.159, 0.157, 0.154, 0.152, 0.1495, 0.147, 0.144, 0.1405, 0.136, 0.1285, 0.12, 0.116, 0.113, 0.111, 0.11, 0.1065, 0.104, 0.1015, 0.0995, 0.098, 0.096, 0.0935, 0.089, 0.086, 0.082, 0.081, 0.0785, 0.076, 0.073, 0.07, 0.067, 0.0635, 0.0595, 0.055, 0.052, 0.0465, 0.043, 0.042, 0.041, 0.04, 0.039, 0.038, 0.037, 0.036, 0.035, 0.033, 0.032, 0.031, 0.0292, 0.028, 0.026, 0.025, 0.024, 0.0225, 0.021, 0.02, 0.018, 0.016, 0.0145, 0.0135];
  const LETTER_DRILLS = { A: 0.234, B: 0.238, C: 0.242, D: 0.246, E: 0.25, F: 0.257, G: 0.261, H: 0.266, I: 0.272, J: 0.277, K: 0.281, L: 0.29, M: 0.295, N: 0.302, O: 0.316, P: 0.323, Q: 0.332, R: 0.339, S: 0.348, T: 0.358, U: 0.368, V: 0.377, W: 0.386, X: 0.397, Y: 0.404, Z: 0.413 };

  function allDrills() {
    const list = [];
    NUMBER_DRILLS.forEach((inch, i) => list.push(['#' + (i + 1), inch]));
    Object.entries(LETTER_DRILLS).forEach(([l, inch]) => list.push([l, inch]));
    for (let n = 1; n <= 64; n++) list.push([fracName(n, 64), n / 64]);
    return list.sort((a, b) => a[1] - b[1]);
  }

  function gcd(a, b) { return b ? gcd(b, a % b) : a; }
  function fracName(n, d) {
    const whole = Math.floor(n / d);
    const r = n % d;
    if (!r) return String(whole);
    const g = gcd(r, d);
    return (whole ? whole + ' ' : '') + (r / g) + '/' + (d / g);
  }
  // Size of a drill or thread named like "#7", "F", "5/16", "27/64"
  function drillInches(name) {
    if (/^#\d+$/.test(name)) return NUMBER_DRILLS[+name.slice(1) - 1];
    if (LETTER_DRILLS[name] !== undefined) return LETTER_DRILLS[name];
    return num(name);
  }

  // Materials: density g/cm³
  const MATERIALS = [['steel', 'Steel (mild)', 7.85], ['stainless', 'Stainless steel', 8.0], ['castiron', 'Cast iron', 7.2], ['aluminium', 'Aluminium', 2.7], ['brass', 'Brass', 8.5], ['bronze', 'Bronze', 8.8], ['copper', 'Copper', 8.96], ['titanium', 'Titanium', 4.5], ['lead', 'Lead', 11.34], ['oak', 'Oak', 0.75], ['pine', 'Pine', 0.5], ['plywood', 'Plywood', 0.6], ['mdf', 'MDF', 0.75], ['acrylic', 'Acrylic (PMMA)', 1.18], ['pc', 'Polycarbonate', 1.2], ['nylon', 'Nylon (PA6)', 1.14], ['pom', 'POM / Delrin', 1.41], ['hdpe', 'HDPE', 0.95], ['pvc', 'PVC', 1.4], ['pla', 'PLA', 1.24], ['petg', 'PETG', 1.27], ['abs', 'ABS', 1.04]];

  // Cutting speeds (m/min) for HSS tools; carbide about 3×
  const CUT_SPEEDS = [['aluminium', 'Aluminium', 90], ['brass', 'Brass / bronze', 60], ['plastic', 'Plastics', 60], ['castiron', 'Cast iron', 20], ['mild', 'Mild steel', 28], ['tool', 'Tool / alloy steel', 15], ['stainless', 'Stainless steel', 12], ['titanium', 'Titanium', 9], ['wood', 'Wood', 100]];

  // Resistor colours
  const COLORS = [
    ['black', 'Black', '#111111'], ['brown', 'Brown', '#7a4a1e'], ['red', 'Red', '#d32f2f'], ['orange', 'Orange', '#f57c00'], ['yellow', 'Yellow', '#fbc02d'],
    ['green', 'Green', '#388e3c'], ['blue', 'Blue', '#1976d2'], ['violet', 'Violet', '#7b1fa2'], ['grey', 'Grey', '#9e9e9e'], ['white', 'White', '#f5f5f5'],
    ['gold', 'Gold', '#c9a227'], ['silver', 'Silver', '#c0c0c0'],
  ];
  const DIGIT = { black: 0, brown: 1, red: 2, orange: 3, yellow: 4, green: 5, blue: 6, violet: 7, grey: 8, white: 9 };
  const MULT = { black: 1, brown: 10, red: 100, orange: 1e3, yellow: 1e4, green: 1e5, blue: 1e6, violet: 1e7, grey: 1e8, white: 1e9, gold: 0.1, silver: 0.01 };
  const TOL = { brown: 1, red: 2, green: 0.5, blue: 0.25, violet: 0.1, grey: 0.05, gold: 5, silver: 10 };
  const TEMPCO = { brown: 100, red: 50, orange: 15, yellow: 25, blue: 10, violet: 5 };
  const colorHex = (c) => (COLORS.find((x) => x[0] === c) || [0, 0, '#000'])[2];
  const colorName = (c) => (COLORS.find((x) => x[0] === c) || [0, c])[1];

  const E12 = [10, 12, 15, 18, 22, 27, 33, 39, 47, 56, 68, 82];
  const E24 = [10, 11, 12, 13, 15, 16, 18, 20, 22, 24, 27, 30, 33, 36, 39, 43, 47, 51, 56, 62, 68, 75, 82, 91];
  const E96 = [100, 102, 105, 107, 110, 113, 115, 118, 121, 124, 127, 130, 133, 137, 140, 143, 147, 150, 154, 158, 162, 165, 169, 174, 178, 182, 187, 191, 196, 200, 205, 210, 215, 221, 226, 232, 237, 243, 249, 255, 261, 267, 274, 280, 287, 294, 301, 309, 316, 324, 332, 340, 348, 357, 365, 374, 383, 392, 402, 412, 422, 432, 442, 453, 464, 475, 487, 499, 511, 523, 536, 549, 562, 576, 590, 604, 619, 634, 649, 665, 681, 698, 715, 732, 750, 768, 787, 806, 825, 845, 866, 887, 909, 931, 953, 976];

  // Nearest value of an E series (series values are one decade, scaled to the value's decade)
  function nearestE(value, series) {
    if (!(value > 0)) return NaN;
    const base = series[0];
    const decade = Math.pow(10, Math.floor(Math.log10(value / base)));
    let best = NaN;
    let bestErr = Infinity;
    for (const d of [decade / 10, decade, decade * 10]) {
      for (const s of series) {
        const cand = s * d;
        const err = Math.abs(Math.log(cand / value));
        if (err < bestErr) { bestErr = err; best = cand; }
      }
    }
    return Math.round(best * 1e6) / 1e6;
  }
  // Smallest E-series value at or above `value`
  function eAtLeast(value, series) {
    const n = nearestE(value, series);
    if (n >= value * 0.9999) return n;
    const base = series[0];
    const decade = Math.pow(10, Math.floor(Math.log10(value / base)));
    for (const d of [decade, decade * 10]) for (const s of series) if (s * d >= value) return Math.round(s * d * 1e6) / 1e6;
    return n;
  }

  // "4k7" → 4700, "2M2" → 2.2e6, "470R" → 470, "0R47" → 0.47, "4.7 kΩ" → 4700
  function parseOhms(s) {
    const t = String(s || '').trim().replace(/[Ωω\s]|ohms?/gi, '').replace(',', '.');
    if (!t) return NaN;
    const m = /^(\d*\.?\d*)([rkmg])(\d*)$/i.exec(t);
    if (m && (m[1] || m[3])) {
      const mult = { r: 1, k: 1e3, m: 1e6, g: 1e9 }[m[2].toLowerCase()];
      return parseFloat((m[1] || '0') + (m[3] ? '.' + m[3] : '')) * mult;
    }
    return num(t);
  }

  function ohmsText(r) {
    if (!isFinite(r)) return '—';
    if (r >= 1e9) return fmt(r / 1e9, 3) + ' GΩ';
    if (r >= 1e6) return fmt(r / 1e6, 3) + ' MΩ';
    if (r >= 1e3) return fmt(r / 1e3, 3) + ' kΩ';
    return fmt(r, 3) + ' Ω';
  }

  // Colour bands for a value: digits (2 or 3) then multiplier
  function bandsFor(value, digits) {
    if (!(value > 0)) return null;
    let exp = Math.floor(Math.log10(value)) - (digits - 1);
    let sig = Math.round(value / Math.pow(10, exp));
    if (sig >= Math.pow(10, digits)) { sig /= 10; exp += 1; }
    if (exp < -2 || exp > 9) return null;
    const names = ['black', 'brown', 'red', 'orange', 'yellow', 'green', 'blue', 'violet', 'grey', 'white'];
    const ds = String(sig).padStart(digits, '0').split('').map((d) => names[+d]);
    const multName = exp === -1 ? 'gold' : exp === -2 ? 'silver' : names[exp];
    return { digits: ds, mult: multName, exact: sig * Math.pow(10, exp) };
  }

  // ==========================================================================
  // Tools
  // ==========================================================================
  const TOOLS = [];
  const tool = (t) => TOOLS.push(t);

  // ---------------- Units ----------------
  tool({
    id: 'units', group: 'Units', name: 'Unit converter', desc: 'Length, weight, pressure, torque, temperature…',
    keys: 'convert units length area volume weight mass pressure torque temperature speed flow power energy mm inch psi bar nm lbft kg lb celsius fahrenheit litre gallon kw hp',
    render(el) {
      const cats = Object.entries(UNITS).map(([k, c]) => [k, c.label]);
      el.innerHTML = `
        ${field('u-cat', 'What', { options: cats })}
        <div class="wb-row">
          <input id="u-val" class="wb-input" inputmode="decimal" autocomplete="off" value="1" aria-label="Value">
          <select id="u-from" class="wb-select" aria-label="From"></select>
          <span>→</span>
          <select id="u-to" class="wb-select" aria-label="To"></select>
        </div>
        <div class="wb-results" id="u-main"></div>
        <div class="wb-hint">In every unit:</div>
        <div class="wb-results" id="u-all"></div>`;
      const fill = (keepUnits) => {
        const cat = $('u-cat').value;
        const c = UNITS[cat];
        const opts = c.units.map((u) => `<option value="${u[0]}">${esc(u[1])}</option>`).join('');
        const [f, t] = imperial() ? c.imperial : c.metric;
        const pf = $('u-from').value;
        const pt = $('u-to').value;
        $('u-from').innerHTML = opts;
        $('u-to').innerHTML = opts;
        $('u-from').value = keepUnits && c.units.some((u) => u[0] === pf) ? pf : f;
        $('u-to').value = keepUnits && c.units.some((u) => u[0] === pt) ? pt : t;
      };
      const calc = () => {
        const cat = $('u-cat').value;
        const v = num($('u-val').value);
        const c = UNITS[cat];
        if (!isFinite(v)) { $('u-main').innerHTML = ''; $('u-all').innerHTML = ''; return; }
        const base = toBase(cat, $('u-from').value, v);
        const name = (u) => c.units.find((x) => x[0] === u)[1];
        const out = fromBase(cat, $('u-to').value, base);
        $('u-main').innerHTML = rowsHtml([[`${fmt(v, 6)} ${name($('u-from').value)}`, `${fmt(out, 6)} ${name($('u-to').value)}`, '', true]]);
        $('u-all').innerHTML = rowsHtml(c.units.map((u) => [u[1], fmt(fromBase(cat, u[0], base), 6)]));
      };
      persist(this, el);
      fill(true);
      $('u-cat').addEventListener('change', () => { fill(false); calc(); });
      el.addEventListener('input', calc);
      el.addEventListener('change', calc);
      calc();
    },
  });

  tool({
    id: 'fraction', group: 'Units', name: 'mm ↔ inch fractions', desc: 'Nearest 1/16", 1/32", 1/64"',
    keys: 'fraction inch mm millimetre decimal 1/16 1/32 1/64',
    render(el) {
      el.innerHTML = `
        <div class="wb-row">
          <input id="fr-val" class="wb-input" inputmode="decimal" autocomplete="off" value="10" aria-label="Value">
          <select id="fr-unit" class="wb-select" aria-label="Unit"><option value="mm">mm</option><option value="in">inch</option></select>
        </div>
        <div class="wb-hint">Inches can be typed as 0.375, 3/8 or 1 3/8.</div>
        <div class="wb-results" id="fr-out"></div>`;
      const calc = () => {
        const v = num($('fr-val').value);
        if (!isFinite(v)) { $('fr-out').innerHTML = ''; return; }
        const inch = $('fr-unit').value === 'mm' ? v / 25.4 : v;
        const rows = [['mm', fmt(inch * 25.4, 3)], ['Decimal inch', fmt(inch, 4) + '"'], ['Thou', fmt(inch * 1000, 1)]];
        for (const d of [16, 32, 64]) {
          const n = Math.round(inch * d);
          const diff = (n / d - inch) * 25.4;
          rows.push([`Nearest 1/${d}"`, fracName(n, d) + '"', diff === 0 ? 'exact' : `${diff > 0 ? '+' : '−'}${fmt(Math.abs(diff), 3)} mm`]);
        }
        $('fr-out').innerHTML = rowsHtml(rows);
      };
      if (imperial()) $('fr-unit').value = 'in';
      persist(this, el);
      el.addEventListener('input', calc);
      el.addEventListener('change', calc);
      calc();
    },
  });

  // ---------------- Electrical ----------------
  tool({
    id: 'awg', group: 'Electrical', name: 'Wire sizes (AWG ↔ mm²)', desc: 'Diameter, area, resistance, current',
    keys: 'awg wire gauge mm2 cable copper ampacity current amps resistance',
    takes: /^(\d+|0{1,4})\s*(awg|ga|gauge)$|^awg\s*(\d+|0{1,4})$|^\d+(?:[.,]\d+)?\s*mm(2|²)$/i,
    render(el, query) {
      el.innerHTML = `
        <div class="wb-row">
          <input id="awg-find" class="wb-input" autocomplete="off" placeholder="Find: 12 AWG, 2.5 mm², 4/0" aria-label="Find a wire size">
        </div>
        <div class="wb-results" id="awg-hit"></div>
        <div id="awg-table"></div>
        <div class="wb-hint">Copper, 20 °C. Current ratings are the widely used typical values for chassis wiring (short runs in air) and power transmission (long runs); always follow your local electrical code.</div>`;
      const sizes = [];
      for (let n = -3; n <= 40; n++) sizes.push(n);
      const row = (n) => {
        const d = awgDiameter(n);
        const a = awgArea(n);
        const [ch, tr] = AWG_AMPS[n];
        return [awgName(n) + (n < 0 ? ` (${-n + 1}/0)` : ''), fmt(d, 3), fmt(d / 25.4, 4), fmt(a, 3), fmt(RHO.copper * 1e9 / a, 3), fmt(RHO.copper * 1e9 / a * 0.3048, 3), fmt(ch, 2), fmt(tr, 3)];
      };
      const head = ['AWG', 'Ø mm', 'Ø in', 'mm²', 'Ω/km', 'Ω/1000 ft', 'A chassis', 'A transm.'];
      const calc = () => {
        const q = $('awg-find').value.trim().toLowerCase().replace(',', '.');
        let hit = -1;
        let n = null;
        const mm = /^(\d+(?:\.\d+)?)\s*mm(2|²)?$/.exec(q);
        const g = /^(?:awg\s*)?(\d+|0{1,4}|[1-4]\/0)\s*(?:awg|ga|gauge)?$/.exec(q);
        if (mm && (mm[2] || q.includes('.'))) {
          const area = +mm[1];
          let best = Infinity;
          sizes.forEach((s) => { const e = Math.abs(Math.log(awgArea(s) / area)); if (e < best) { best = e; n = s; } });
          $('awg-hit').innerHTML = rowsHtml([[`${fmt(area, 3)} mm² is closest to`, `${awgName(n)} AWG`, `${fmt(awgArea(n), 3)} mm²`, true]]);
        } else if (g) {
          const t = g[1];
          n = /^\d\/0$/.test(t) ? 1 - +t[0] : /^0+$/.test(t) ? 1 - t.length : +t;
          if (n < -3 || n > 40) n = null;
          $('awg-hit').innerHTML = n === null ? '' : rowsHtml([[`${awgName(n)} AWG`, `${fmt(awgArea(n), 3)} mm²`, `Ø ${fmt(awgDiameter(n), 3)} mm`, true], ['Nearest metric cable', nearestMetricCable(awgArea(n))]]);
        } else {
          $('awg-hit').innerHTML = '';
        }
        if (n !== null) hit = sizes.indexOf(n);
        $('awg-table').innerHTML = tableHtml(head, sizes.map(row), hit);
        if (hit >= 0) {
          const tr = $('awg-table').querySelectorAll('tbody tr')[hit];
          if (tr) tr.scrollIntoView({ block: 'center' });
        }
      };
      persist(this, el);
      if (query) $('awg-find').value = query;
      el.addEventListener('input', calc);
      calc();
    },
  });

  const METRIC_CABLES = [0.5, 0.75, 1, 1.5, 2.5, 4, 6, 10, 16, 25, 35, 50, 70, 95, 120, 150, 185, 240];
  function nearestMetricCable(area) {
    const up = METRIC_CABLES.find((c) => c >= area * 0.97);
    return up ? `${fmt(up, 2)} mm² (next size up)` : 'larger than 240 mm²';
  }

  tool({
    id: 'ohm', group: 'Electrical', name: "Ohm's law and power", desc: 'Any two of V, I, R, P',
    keys: "ohm ohms law voltage current resistance power watts volts amps v i r p",
    render(el) {
      el.innerHTML = `
        <div class="wb-hint">Fill in any two; the other two are worked out.</div>
        <div class="wb-grid">${field('o-v', 'Voltage (V)')}${field('o-i', 'Current (A)')}${field('o-r', 'Resistance (Ω)', { placeholder: 'e.g. 4k7' })}${field('o-p', 'Power (W)')}</div>
        <div class="wb-results" id="o-out"></div>
        <div class="wb-error" id="o-err"></div>`;
      const calc = () => {
        const V = num($('o-v').value);
        const I = num($('o-i').value);
        const R = parseOhms($('o-r').value);
        const P = num($('o-p').value);
        const have = [isFinite(V), isFinite(I), isFinite(R), isFinite(P)].filter(Boolean).length;
        $('o-err').textContent = have > 2 ? 'Fill in only two values.' : '';
        if (have !== 2) { $('o-out').innerHTML = ''; return; }
        let v = V, i = I, r = R, p = P;
        if (isFinite(V) && isFinite(I)) { r = V / I; p = V * I; }
        else if (isFinite(V) && isFinite(R)) { i = V / R; p = V * V / R; }
        else if (isFinite(V) && isFinite(P)) { i = P / V; r = V * V / P; }
        else if (isFinite(I) && isFinite(R)) { v = I * R; p = I * I * R; }
        else if (isFinite(I) && isFinite(P)) { v = P / I; r = P / (I * I); }
        else if (isFinite(R) && isFinite(P)) { v = Math.sqrt(P * R); i = Math.sqrt(P / R); }
        $('o-out').innerHTML = rowsHtml([['Voltage', fmt(v, 4) + ' V'], ['Current', i < 1 ? fmt(i * 1000, 4) + ' mA' : fmt(i, 4) + ' A'], ['Resistance', ohmsText(r)], ['Power', p < 1 ? fmt(p * 1000, 4) + ' mW' : fmt(p, 4) + ' W']]);
      };
      persist(this, el);
      el.addEventListener('input', calc);
      calc();
    },
  });

  tool({
    id: 'resistor', group: 'Electrical', name: 'Resistor colour codes', desc: '4, 5 and 6 bands, both ways; E12/E24/E96',
    keys: 'resistor colour color code bands e12 e24 e96 ohm',
    render(el) {
      const mode = store.get('res-mode') || 'value';
      el.innerHTML = `
        ${seg('res-mode', [['value', 'Value → colours'], ['bands', 'Colours → value']], mode)}
        <div id="res-body"></div>`;
      const body = $('res-body');
      const drawResistor = (bands) => `<div class="wb-resistor"><span class="lead"></span><span class="body">${bands.map((c, i) => `<span class="band${i === bands.length - 1 && bands.length > 3 ? ' gap' : ''}" style="background:${colorHex(c)}" title="${esc(colorName(c))}"></span>`).join('')}</span><span class="lead"></span></div>`;
      const showValue = () => {
        body.innerHTML = `
          <div class="wb-grid">${field('rv-val', 'Resistance', { placeholder: '4k7, 470, 2.2M' })}${field('rv-bands', 'Bands', { options: [['4', '4 bands (5 % / 10 %)'], ['5', '5 bands (1 % and better)'], ['6', '6 bands (with temperature)']] })}${field('rv-tol', 'Tolerance', { options: [['gold', '5 %'], ['silver', '10 %'], ['brown', '1 %'], ['red', '2 %'], ['green', '0.5 %'], ['blue', '0.25 %'], ['violet', '0.1 %']] })}</div>
          <div id="rv-draw"></div>
          <div class="wb-results" id="rv-out"></div>`;
        const calc = () => {
          const r = parseOhms($('rv-val').value);
          const n = +$('rv-bands').value;
          const digits = n === 4 ? 2 : 3;
          const b = bandsFor(r, digits);
          if (!b) { $('rv-out').innerHTML = ''; $('rv-draw').innerHTML = ''; return; }
          const tol = $('rv-tol').value;
          const bands = [...b.digits, b.mult, tol];
          if (n === 6) bands.push('brown');
          $('rv-draw').innerHTML = drawResistor(bands);
          const rows = [['Colours', bands.map(colorName).join(' · ')], ['Value shown', ohmsText(b.exact), b.exact !== r ? `rounded from ${ohmsText(r)}` : '']];
          rows.push(['Nearest E12', ohmsText(nearestE(r, E12))], ['Nearest E24', ohmsText(nearestE(r, E24))], ['Nearest E96', ohmsText(nearestE(r, E96))]);
          if (n === 6) rows.push(['Temperature coefficient', '100 ppm/K (brown)']);
          $('rv-out').innerHTML = rowsHtml(rows);
        };
        if (!$('rv-val').value) $('rv-val').value = '4k7';
        body.oninput = calc;
        body.onchange = calc;
        calc();
      };
      const showBands = () => {
        const digitOpts = COLORS.slice(0, 10).map((c) => [c[0], c[1]]);
        const multOpts = COLORS.map((c) => [c[0], c[1] + ' ×' + (MULT[c[0]] >= 1000 ? fmt(MULT[c[0]], 0) : MULT[c[0]])]);
        const tolOpts = Object.keys(TOL).map((c) => [c, `${colorName(c)} ±${TOL[c]} %`]);
        const tcOpts = Object.keys(TEMPCO).map((c) => [c, `${colorName(c)} ${TEMPCO[c]} ppm/K`]);
        body.innerHTML = `
          ${field('rb-n', 'Bands', { options: [['4', '4 bands'], ['5', '5 bands'], ['6', '6 bands']] })}
          <div class="wb-grid" id="rb-sel"></div>
          <div id="rb-draw"></div>
          <div class="wb-results" id="rb-out"></div>`;
        const build = () => {
          const n = +$('rb-n').value;
          const d = n === 4 ? 2 : 3;
          let html = '';
          for (let k = 1; k <= d; k++) html += field('rb-d' + k, `Band ${k} (digit)`, { options: digitOpts });
          html += field('rb-m', `Band ${d + 1} (multiplier)`, { options: multOpts });
          html += field('rb-t', `Band ${d + 2} (tolerance)`, { options: tolOpts });
          if (n === 6) html += field('rb-c', 'Band 6 (temperature)', { options: tcOpts });
          $('rb-sel').innerHTML = html;
          const defaults = d === 2 ? ['yellow', 'violet'] : ['yellow', 'violet', 'black'];
          defaults.forEach((c, i) => { $('rb-d' + (i + 1)).value = c; });
          $('rb-m').value = d === 2 ? 'red' : 'brown';
          $('rb-t').value = n === 4 ? 'gold' : 'brown';
        };
        const calc = () => {
          const n = +$('rb-n').value;
          const d = n === 4 ? 2 : 3;
          const digits = [];
          for (let k = 1; k <= d; k++) digits.push($('rb-d' + k).value);
          const value = parseInt(digits.map((c) => DIGIT[c]).join(''), 10) * MULT[$('rb-m').value];
          const tol = TOL[$('rb-t').value];
          const bands = [...digits, $('rb-m').value, $('rb-t').value];
          if (n === 6) bands.push($('rb-c').value);
          $('rb-draw').innerHTML = drawResistor(bands);
          const rows = [['Resistance', ohmsText(value), '', true], ['Tolerance', `±${tol} %`, `${ohmsText(value * (1 - tol / 100))} … ${ohmsText(value * (1 + tol / 100))}`]];
          if (n === 6) rows.push(['Temperature coefficient', `${TEMPCO[$('rb-c').value]} ppm/K`]);
          $('rb-out').innerHTML = rowsHtml(rows);
        };
        build();
        $('rb-n').addEventListener('change', () => { build(); calc(); });
        body.oninput = calc;
        body.onchange = (e) => { if (e.target.id !== 'rb-n') calc(); };
        calc();
      };
      const show = (m) => { store.set('res-mode', m); if (m === 'bands') showBands(); else showValue(); };
      wireSeg(el, 'res-mode', show);
      show(mode);
    },
  });

  tool({
    id: 'led', group: 'Electrical', name: 'LED series resistor', desc: 'Resistor and its power for one or more LEDs',
    keys: 'led resistor series forward voltage current',
    render(el) {
      el.innerHTML = `
        <div class="wb-grid">${field('l-vs', 'Supply (V)')}${field('l-vf', 'LED forward voltage (V)')}${field('l-ma', 'LED current (mA)')}${field('l-n', 'LEDs in series')}</div>
        <div class="wb-hint">Typical forward voltage: red 2.0 V, yellow/green 2.1 V, blue/white 3.0–3.3 V. Typical current: 10–20 mA.</div>
        <div class="wb-results" id="l-out"></div>
        <div class="wb-error" id="l-err"></div>`;
      const calc = () => {
        const vs = num($('l-vs').value);
        const vf = num($('l-vf').value);
        const i = num($('l-ma').value) / 1000;
        const n = Math.max(1, Math.round(num($('l-n').value) || 1));
        $('l-err').textContent = '';
        if (![vs, vf, i].every(isFinite) || i <= 0) { $('l-out').innerHTML = ''; return; }
        const drop = vs - n * vf;
        if (drop <= 0) { $('l-out').innerHTML = ''; $('l-err').textContent = `The supply must be higher than ${fmt(n * vf, 3)} V for ${n} LED${n > 1 ? 's' : ''}.`; return; }
        const r = drop / i;
        const e24 = eAtLeast(r, E24);
        const p = drop * drop / e24;
        const rating = [0.125, 0.25, 0.5, 1, 2, 3, 5].find((w) => w >= p * 2) || 10;
        $('l-out').innerHTML = rowsHtml([['Exact resistor', ohmsText(r)], ['Use (E24, next up)', ohmsText(e24), '', true], ['Actual current', fmt(drop / e24 * 1000, 3) + ' mA'], ['Power in resistor', fmt(p * 1000, 3) + ' mW'], ['Resistor rating', rating < 1 ? `${fmt(rating, 3)} W` : `${rating} W`, 'at least twice the power']]);
      };
      persist(this, el);
      ['l-vs', 'l-vf', 'l-ma', 'l-n'].forEach((id, k) => { if (!$(id).value) $(id).value = ['5', '2', '20', '1'][k]; });
      el.addEventListener('input', calc);
      calc();
    },
  });

  tool({
    id: 'divider', group: 'Electrical', name: 'Voltage divider', desc: 'Output voltage, or the resistor for a wanted voltage',
    keys: 'voltage divider resistor r1 r2 vout',
    render(el) {
      el.innerHTML = `
        <div class="wb-grid">${field('d-vin', 'Input (V)')}${field('d-r1', 'R1, top (Ω)', { placeholder: '10k' })}${field('d-r2', 'R2, bottom (Ω)', { placeholder: '4k7' })}${field('d-want', 'Wanted output (V, optional)')}</div>
        <div class="wb-results" id="d-out"></div>`;
      const calc = () => {
        const vin = num($('d-vin').value);
        const r1 = parseOhms($('d-r1').value);
        const r2 = parseOhms($('d-r2').value);
        const want = num($('d-want').value);
        const rows = [];
        if ([vin, r1, r2].every(isFinite) && r1 + r2 > 0) {
          const vout = vin * r2 / (r1 + r2);
          rows.push(['Output', fmt(vout, 4) + ' V', '', true], ['Current through the divider', fmt(vin / (r1 + r2) * 1000, 4) + ' mA'], ['Ratio', fmt(r2 / (r1 + r2), 4)]);
        }
        if ([vin, r2, want].every(isFinite) && want > 0 && want < vin) {
          const r1n = r2 * (vin - want) / want;
          rows.push([`R1 for ${fmt(want, 3)} V`, ohmsText(r1n)], ['Nearest E24', ohmsText(nearestE(r1n, E24)), `gives ${fmt(vin * r2 / (nearestE(r1n, E24) + r2), 4)} V`], ['Nearest E96', ohmsText(nearestE(r1n, E96)), `gives ${fmt(vin * r2 / (nearestE(r1n, E96) + r2), 4)} V`]);
        }
        $('d-out').innerHTML = rowsHtml(rows);
      };
      persist(this, el);
      el.addEventListener('input', calc);
      calc();
    },
  });

  tool({
    id: 'vdrop', group: 'Electrical', name: 'Cable voltage drop', desc: 'Drop and loss over a cable run',
    keys: 'voltage drop cable wire length loss awg mm2 12v 24v dc',
    render(el) {
      const awgOpts = [];
      for (let n = -3; n <= 30; n++) awgOpts.push(['awg' + n, `${awgName(n)} AWG (${fmt(awgArea(n), 2)} mm²)`]);
      const mmOpts = METRIC_CABLES.map((c) => ['mm' + c, `${fmt(c, 2)} mm²`]);
      el.innerHTML = `
        <div class="wb-grid">
          ${field('vd-len', `Cable length, one way (${imperial() ? 'ft' : 'm'})`)}
          ${field('vd-size', 'Wire size', { options: imperial() ? [...awgOpts, ...mmOpts] : [...mmOpts, ...awgOpts] })}
          ${field('vd-a', 'Current (A)')}
          ${field('vd-v', 'System voltage (V)')}
          ${field('vd-mat', 'Conductor', { options: [['copper', 'Copper'], ['aluminium', 'Aluminium']] })}
        </div>
        <div class="wb-results" id="vd-out"></div>
        <div class="wb-hint">Counts both conductors (out and back), at 20 °C. A drop under 3 % is a common target; 10 % for non-critical loads.</div>`;
      const calc = () => {
        const len = num($('vd-len').value) * (imperial() ? 0.3048 : 1);
        const a = num($('vd-a').value);
        const v = num($('vd-v').value);
        const s = $('vd-size').value;
        const area = s.startsWith('awg') ? awgArea(+s.slice(3)) : +s.slice(2);
        if (![len, a].every(isFinite)) { $('vd-out').innerHTML = ''; return; }
        const r = RHO[$('vd-mat').value] * (2 * len) / (area * 1e-6);
        const drop = a * r;
        const rows = [['Voltage drop', fmt(drop, 3) + ' V', '', true], ['Cable resistance (both ways)', ohmsText(r)], ['Power lost in the cable', fmt(a * a * r, 3) + ' W']];
        if (isFinite(v) && v > 0) rows.splice(1, 0, ['Drop in percent', fmt(drop / v * 100, 2) + ' %', `${fmt(v - drop, 3)} V left at the load`]);
        $('vd-out').innerHTML = rowsHtml(rows);
      };
      persist(this, el);
      el.addEventListener('input', calc);
      el.addEventListener('change', calc);
      calc();
    },
  });

  tool({
    id: 'battery', group: 'Electrical', name: 'Battery capacity and runtime', desc: 'Ah, Wh and how long a load runs',
    keys: 'battery capacity runtime ah wh mah load hours lifepo4 lead acid',
    render(el) {
      el.innerHTML = `
        <div class="wb-grid">
          ${field('b-ah', 'Capacity (Ah)')}
          ${field('b-v', 'Battery voltage (V)')}
          ${field('b-load', 'Load (W)')}
          ${field('b-use', 'Usable capacity', { options: [['100', '100 % (theoretical)'], ['90', '90 % (lithium, LiFePO₄)'], ['50', '50 % (lead-acid, for long life)'], ['80', '80 %']] })}
          ${field('b-eff', 'Efficiency', { options: [['100', '100 % (DC load)'], ['90', '90 % (good inverter)'], ['85', '85 % (inverter)']] })}
        </div>
        <div class="wb-results" id="b-out"></div>`;
      const calc = () => {
        const ah = num($('b-ah').value);
        const v = num($('b-v').value);
        const w = num($('b-load').value);
        if (![ah, v].every(isFinite)) { $('b-out').innerHTML = ''; return; }
        const wh = ah * v;
        const usable = wh * +$('b-use').value / 100 * +$('b-eff').value / 100;
        const rows = [['Energy stored', fmt(wh, 1) + ' Wh', `${fmt(wh / 1000, 3)} kWh`], ['Usable', fmt(usable, 1) + ' Wh']];
        if (isFinite(w) && w > 0) {
          const h = usable / w;
          const hh = Math.floor(h);
          const mm = Math.round((h - hh) * 60);
          rows.push(['Runtime', `${hh} h ${mm} min`, '', true], ['Current drawn from the battery', fmt(w / v / (+$('b-eff').value / 100), 3) + ' A']);
        }
        $('b-out').innerHTML = rowsHtml(rows);
      };
      persist(this, el);
      el.addEventListener('input', calc);
      el.addEventListener('change', calc);
      calc();
    },
  });

  // ---------------- Threads & drills ----------------
  tool({
    id: 'tap', group: 'Threads & drills', name: 'Tap drill sizes', desc: 'Metric, UNC/UNF, BSP and NPT',
    keys: 'tap drill thread metric coarse fine unc unf bsp npt pipe m3 m4 m5 m6 m8 m10 m12',
    takes: /^m\d+(?:[.,]\d+)?(?:\s*[x×]\s*\d+(?:[.,]\d+)?)?$|^(#\d+|\d+\/\d+)\s*-\s*\d+$|^(unc|unf|bsp|npt)$/i,
    render(el, query) {
      const kind = store.get('tap-kind') || 'mc';
      el.innerHTML = `
        ${seg('tap-kind', [['mc', 'Metric coarse'], ['mf', 'Metric fine'], ['un', 'UNC / UNF'], ['pipe', 'Pipe (BSP, NPT)']], kind)}
        <input id="tap-find" type="search" class="wb-input" autocomplete="off" placeholder="Filter: M6, 1/4-20, 3/8…" aria-label="Filter">
        <div id="tap-table"></div>
        <div class="wb-hint" id="tap-note"></div>`;
      let cur = kind;
      const draw = () => {
        const q = $('tap-find').value.trim().toLowerCase().replace(/\s/g, '').replace('×', 'x').replace(',', '.');
        let head;
        let rows;
        let note;
        if (cur === 'mc' || cur === 'mf') {
          const list = cur === 'mc' ? METRIC_COARSE : METRIC_FINE;
          head = ['Thread', 'Pitch', 'Tap drill', 'Clearance: close', 'normal', 'loose'];
          rows = list.map(([d, p]) => {
            const c = CLEARANCE[d] || ['', '', ''];
            return [`M${fmt(d, 1)}${cur === 'mf' ? '×' + fmt(p, 2) : ''}`, fmt(p, 2) + ' mm', fmt(metricTapDrill(d, p), 2) + ' mm', c[0] ? fmt(c[0], 1) : '', c[1] ? fmt(c[1], 1) : '', c[2] ? fmt(c[2], 1) : ''];
          }).filter((r) => !q || r[0].toLowerCase().replace(/\s/g, '').replace('×', 'x').replace(',', '.').startsWith(q));
          note = 'Tap drill ≈ diameter − pitch (about 75 % thread), as the standard charts give it. Clearance holes from ISO 273 (close, medium, loose fit).';
        } else if (cur === 'un') {
          head = ['Thread', 'Series', 'Tap drill', 'Drill in', 'Drill mm'];
          rows = UNIFIED.map(([s, , tpi, series, drill]) => {
            const inch = drillInches(drill);
            return [`${s}-${tpi}`, series, drill, fmt(inch, 4) + '"', fmt(inch * 25.4, 2)];
          }).filter((r) => !q || r[0].toLowerCase().startsWith(q) || r[1].toLowerCase() === q);
          note = 'Standard 75 % thread tap drills.';
        } else {
          head = ['Size', 'BSP TPI', 'BSP parallel (G)', 'BSP taper (R)', 'NPT TPI', 'NPT tap drill'];
          rows = PIPE.map(([s, tpi, g, r, npt, ntpi]) => [s, tpi ? String(tpi) : '—', g ? fmt(g, 2) + ' mm' : '—', r ? fmt(r, 2) + ' mm' : '—', String(ntpi), npt + (/^\d/.test(npt) ? `" (${fmt(num(npt) * 25.4, 1)} mm)` : '')])
            .filter((r) => !q || ['bsp', 'npt'].includes(q) || r[0].replace(/\s/g, '') === q || r[0].replace(/\s/g, '').startsWith(q));
          note = 'Pipe sizes are nominal names, not diameters. BSP taper (R) and NPT holes are usually reamed with a taper reamer for a tight seal.';
        }
        $('tap-table').innerHTML = rows.length ? tableHtml(head, rows) : '<div class="wb-empty">No matching thread.</div>';
        $('tap-note').textContent = note;
      };
      wireSeg(el, 'tap-kind', (v) => { cur = v; store.set('tap-kind', v); draw(); });
      if (query) {
        const q = query.trim().toLowerCase();
        if (/^m/.test(q)) { cur = /x|×/.test(q) ? 'mf' : 'mc'; }
        else if (/^(#|\d+\/)/.test(q) || /^un[cf]$/.test(q)) cur = 'un';
        else if (/^(bsp|npt)$/.test(q)) cur = 'pipe';
        el.querySelectorAll('#tap-kind button').forEach((b) => b.classList.toggle('active', b.dataset.v === cur));
        $('tap-find').value = /^(unc|unf)$/.test(q) ? q : (/^(bsp|npt)$/.test(q) ? '' : query.trim());
      }
      $('tap-find').addEventListener('input', draw);
      draw();
    },
  });

  tool({
    id: 'drills', group: 'Threads & drills', name: 'Drill sizes', desc: 'Number, letter and fractional drills ↔ mm',
    keys: 'drill bit number letter fractional size mm inch',
    render(el) {
      el.innerHTML = `
        <div class="wb-row">
          <input id="dr-find" class="wb-input" autocomplete="off" placeholder="6.5 mm, 0.257, #7, F, 1/4" aria-label="Find a drill">
        </div>
        <div class="wb-results" id="dr-hit"></div>
        <div id="dr-table"></div>`;
      const drills = allDrills();
      const calc = () => {
        const q = $('dr-find').value.trim();
        let target = NaN;
        const mm = /^(\d+(?:[.,]\d+)?)\s*mm$/i.exec(q);
        if (mm) target = num(mm[1]) / 25.4;
        else if (/^#\d+$/.test(q) || /^[A-Za-z]$/.test(q)) target = drillInches(q.toUpperCase());
        else if (q) {
          const v = num(q.replace(/["″]|in(ch)?$/i, ''));
          if (isFinite(v)) target = v > 1.1 && !/["″]|in/i.test(q) ? v / 25.4 : v;
        }
        let hit = -1;
        if (isFinite(target)) {
          let best = Infinity;
          drills.forEach((d, i) => { const e = Math.abs(d[1] - target); if (e < best) { best = e; hit = i; } });
          const d = drills[hit];
          $('dr-hit').innerHTML = rowsHtml([[`${fmt(target * 25.4, 3)} mm = ${fmt(target, 4)}"`, `nearest: ${d[0]}`, `${fmt(d[1], 4)}" / ${fmt(d[1] * 25.4, 3)} mm`, true]]);
        } else {
          $('dr-hit').innerHTML = '';
        }
        $('dr-table').innerHTML = tableHtml(['Drill', 'Inch', 'mm'], drills.map((d) => [d[0], fmt(d[1], 4), fmt(d[1] * 25.4, 3)]), hit);
        if (hit >= 0) {
          const tr = $('dr-table').querySelectorAll('tbody tr')[hit];
          if (tr) tr.scrollIntoView({ block: 'center' });
        }
      };
      persist(this, el);
      el.addEventListener('input', calc);
      calc();
    },
  });

  // ---------------- Shop ----------------
  const len = () => (imperial() ? 'in' : 'mm');
  const lenToMm = (v) => (imperial() ? v * 25.4 : v);

  tool({
    id: 'weight', group: 'Shop', name: 'Weight of a part', desc: 'From shape, size and material',
    keys: 'weight mass part steel aluminium brass plate bar tube hex sheet kg lb material density',
    render(el) {
      const shapes = [['plate', 'Plate / block / sheet'], ['round', 'Round bar'], ['tube', 'Round tube'], ['square', 'Square bar'], ['hex', 'Hex bar'], ['sqtube', 'Square tube']];
      el.innerHTML = `
        <div class="wb-grid">
          ${field('w-shape', 'Shape', { options: shapes })}
          ${field('w-mat', 'Material', { options: MATERIALS.map((m) => [m[0], `${m[1]} (${fmt(m[2], 2)} g/cm³)`]) })}
        </div>
        <div class="wb-grid" id="w-dims"></div>
        <div class="wb-grid">${field('w-len', `Length (${len()})`)}${field('w-qty', 'How many')}</div>
        <div class="wb-results" id="w-out"></div>`;
      const dimsFor = {
        plate: [['w-a', 'Width'], ['w-b', 'Thickness']],
        round: [['w-a', 'Diameter']],
        tube: [['w-a', 'Outside diameter'], ['w-b', 'Wall thickness']],
        square: [['w-a', 'Side']],
        hex: [['w-a', 'Across flats']],
        sqtube: [['w-a', 'Outside side'], ['w-b', 'Wall thickness']],
      };
      const drawDims = () => {
        const old = {};
        el.querySelectorAll('#w-dims input').forEach((i) => { old[i.id] = i.value; });
        $('w-dims').innerHTML = dimsFor[$('w-shape').value].map(([id, l]) => field(id, `${l} (${len()})`)).join('');
        Object.entries(old).forEach(([id, v]) => { if ($(id)) $(id).value = v; });
      };
      const calc = () => {
        const shape = $('w-shape').value;
        const a = lenToMm(num($('w-a') && $('w-a').value));
        const b = lenToMm(num($('w-b') && $('w-b').value));
        const l = lenToMm(num($('w-len').value));
        const qty = num($('w-qty').value) || 1;
        let area = NaN; // mm²
        if (shape === 'plate') area = a * b;
        else if (shape === 'round') area = Math.PI * a * a / 4;
        else if (shape === 'tube') area = b * 2 < a ? Math.PI * (a * a - Math.pow(a - 2 * b, 2)) / 4 : NaN;
        else if (shape === 'square') area = a * a;
        else if (shape === 'hex') area = Math.sqrt(3) / 2 * a * a;
        else if (shape === 'sqtube') area = b * 2 < a ? a * a - Math.pow(a - 2 * b, 2) : NaN;
        const dens = MATERIALS.find((m) => m[0] === $('w-mat').value)[2];
        if (![area, l].every(isFinite) || area <= 0) { $('w-out').innerHTML = ''; return; }
        const kg = area * l / 1000 * dens / 1000; // mm³ → cm³ → g → kg
        const total = kg * qty;
        const per = imperial() ? `${fmt(kg * 2.20462262, 3)} lb per piece` : `${fmt(kg, 3)} kg per piece`;
        $('w-out').innerHTML = rowsHtml([
          ['Weight', imperial() ? `${fmt(total * 2.20462262, 3)} lb` : `${fmt(total, 3)} kg`, imperial() ? `${fmt(total, 3)} kg` : `${fmt(total * 2.20462262, 3)} lb`, true],
          ['Per piece', per],
          ['Weight per metre', `${fmt(area * dens / 1000, 3)} kg/m`, `${fmt(area * dens / 1000 * 0.671968975, 3)} lb/ft`],
          ['Cross-section', `${fmt(area, 2)} mm²`],
        ]);
      };
      persist(this, el);
      drawDims();
      persist(this, el);
      $('w-shape').addEventListener('change', () => { drawDims(); calc(); });
      el.addEventListener('input', calc);
      el.addEventListener('change', calc);
      calc();
    },
  });

  tool({
    id: 'shapes', group: 'Shop', name: 'Area and volume', desc: 'Circles, rectangles, cylinders, spheres…',
    keys: 'area volume circle rectangle triangle cylinder sphere cone box surface circumference',
    render(el) {
      const shapes = [['circle', 'Circle'], ['rect', 'Rectangle'], ['triangle', 'Triangle'], ['ring', 'Ring'], ['cylinder', 'Cylinder'], ['box', 'Box'], ['sphere', 'Sphere'], ['cone', 'Cone']];
      const dims = { circle: ['Diameter'], rect: ['Width', 'Length'], triangle: ['Base', 'Height'], ring: ['Outside diameter', 'Inside diameter'], cylinder: ['Diameter', 'Height'], box: ['Width', 'Length', 'Height'], sphere: ['Diameter'], cone: ['Base diameter', 'Height'] };
      el.innerHTML = `
        ${field('s-shape', 'Shape', { options: shapes })}
        <div class="wb-grid" id="s-dims"></div>
        <div class="wb-results" id="s-out"></div>`;
      const u = len();
      const drawDims = () => {
        $('s-dims').innerHTML = dims[$('s-shape').value].map((l, i) => field('s-' + i, `${l} (${u})`)).join('');
      };
      const calc = () => {
        const s = $('s-shape').value;
        const v = dims[s].map((_, i) => num($('s-' + i).value));
        if (!v.every(isFinite)) { $('s-out').innerHTML = ''; return; }
        const [a, b, c] = v;
        const rows = [];
        const area = (x) => rows.push(['Area', `${fmt(x, 4)} ${u}²`, '', true]);
        const vol = (x) => rows.push(['Volume', `${fmt(x, 4)} ${u}³`, u === 'mm' ? `${fmt(x / 1e6, 4)} litre` : `${fmt(x * 0.016387064, 4)} litre`, true]);
        if (s === 'circle') { area(Math.PI * a * a / 4); rows.push(['Circumference', `${fmt(Math.PI * a, 4)} ${u}`]); }
        if (s === 'rect') { area(a * b); rows.push(['Perimeter', `${fmt(2 * (a + b), 4)} ${u}`], ['Diagonal', `${fmt(Math.hypot(a, b), 4)} ${u}`]); }
        if (s === 'triangle') area(a * b / 2);
        if (s === 'ring') area(Math.PI * (a * a - b * b) / 4);
        if (s === 'cylinder') { vol(Math.PI * a * a / 4 * b); rows.push(['Surface', `${fmt(Math.PI * a * b + Math.PI * a * a / 2, 4)} ${u}²`]); }
        if (s === 'box') { vol(a * b * c); rows.push(['Surface', `${fmt(2 * (a * b + a * c + b * c), 4)} ${u}²`]); }
        if (s === 'sphere') { vol(Math.PI * a * a * a / 6); rows.push(['Surface', `${fmt(Math.PI * a * a, 4)} ${u}²`]); }
        if (s === 'cone') { vol(Math.PI * a * a / 4 * b / 3); rows.push(['Slant height', `${fmt(Math.hypot(a / 2, b), 4)} ${u}`]); }
        $('s-out').innerHTML = rowsHtml(rows);
      };
      persist(this, el);
      drawDims();
      persist(this, el);
      $('s-shape').addEventListener('change', () => { drawDims(); calc(); });
      el.addEventListener('input', calc);
      calc();
    },
  });

  tool({
    id: 'speeds', group: 'Shop', name: 'Speeds (RPM)', desc: 'Drill, lathe and mill speed from material and size',
    keys: 'speeds feeds rpm drill lathe mill cutting speed surface sfm vc',
    render(el) {
      el.innerHTML = `
        <div class="wb-grid">
          ${field('sp-mat', 'Material', { options: CUT_SPEEDS.map((m) => [m[0], m[1]]) })}
          ${field('sp-tool', 'Tool', { options: [['hss', 'HSS (high-speed steel)'], ['carbide', 'Carbide']] })}
          ${field('sp-d', `Diameter: drill, work or cutter (${len()})`)}
        </div>
        <div class="wb-results" id="sp-out"></div>
        <div class="wb-hint">Starting points only. Reduce for deep holes, poor rigidity or no coolant; small drills often need a lower speed than the formula gives.</div>`;
      const calc = () => {
        const d = lenToMm(num($('sp-d').value));
        if (!isFinite(d) || d <= 0) { $('sp-out').innerHTML = ''; return; }
        const vc = CUT_SPEEDS.find((m) => m[0] === $('sp-mat').value)[2] * ($('sp-tool').value === 'carbide' ? 3 : 1);
        const rpm = vc * 1000 / (Math.PI * d);
        $('sp-out').innerHTML = rowsHtml([['Spindle speed', `${fmt(Math.round(rpm), 0)} rpm`, '', true], ['Cutting speed', `${fmt(vc, 0)} m/min`, `${fmt(vc * 3.28084, 0)} ft/min (SFM)`], ['Range', `${fmt(Math.round(rpm * 0.75), 0)} – ${fmt(Math.round(rpm * 1.25), 0)} rpm`]]);
      };
      persist(this, el);
      el.addEventListener('input', calc);
      el.addEventListener('change', calc);
      calc();
    },
  });

  tool({
    id: 'gears', group: 'Shop', name: 'Gear and pulley ratio', desc: 'Ratio, output speed and torque',
    keys: 'gear ratio pulley belt sprocket chain teeth rpm torque',
    render(el) {
      el.innerHTML = `
        <div class="wb-grid">${field('g-a', 'Driving gear: teeth or pulley diameter')}${field('g-b', 'Driven gear: teeth or pulley diameter')}${field('g-rpm', 'Input speed (rpm, optional)')}${field('g-tq', 'Input torque (optional)')}</div>
        <div class="wb-results" id="g-out"></div>`;
      const calc = () => {
        const a = num($('g-a').value);
        const b = num($('g-b').value);
        if (![a, b].every(isFinite) || a <= 0 || b <= 0) { $('g-out').innerHTML = ''; return; }
        const ratio = b / a;
        const rows = [['Ratio', `${fmt(ratio, 4)} : 1`, ratio > 1 ? 'slower, more torque' : ratio < 1 ? 'faster, less torque' : 'same speed', true]];
        const rpm = num($('g-rpm').value);
        if (isFinite(rpm)) rows.push(['Output speed', `${fmt(rpm / ratio, 2)} rpm`]);
        const tq = num($('g-tq').value);
        if (isFinite(tq)) rows.push(['Output torque', fmt(tq * ratio, 3), 'before friction losses, same unit as the input']);
        $('g-out').innerHTML = rowsHtml(rows);
      };
      persist(this, el);
      el.addEventListener('input', calc);
      calc();
    },
  });

  tool({
    id: 'lever', group: 'Shop', name: 'Lever', desc: 'Force needed, mechanical advantage',
    keys: 'lever force effort load arm fulcrum moment mechanical advantage',
    render(el) {
      el.innerHTML = `
        <div class="wb-grid">${field('lv-load', 'Load (force or weight)')}${field('lv-la', 'Load distance from pivot')}${field('lv-ea', 'Effort distance from pivot')}</div>
        <div class="wb-results" id="lv-out"></div>
        <div class="wb-hint">Use the same unit for both distances; the force comes out in the unit of the load.</div>`;
      const calc = () => {
        const load = num($('lv-load').value);
        const la = num($('lv-la').value);
        const ea = num($('lv-ea').value);
        if (![load, la, ea].every(isFinite) || ea <= 0) { $('lv-out').innerHTML = ''; return; }
        $('lv-out').innerHTML = rowsHtml([['Effort needed', fmt(load * la / ea, 4), '', true], ['Mechanical advantage', `${fmt(ea / la, 4)} : 1`]]);
      };
      persist(this, el);
      el.addEventListener('input', calc);
      calc();
    },
  });

  tool({
    id: 'slope', group: 'Shop', name: 'Slope and angle', desc: 'Percent, degrees, 1:x',
    keys: 'slope angle grade incline percent degrees ramp pitch roof rise run',
    render(el) {
      el.innerHTML = `
        <div class="wb-grid">${field('sl-rise', 'Rise')}${field('sl-run', 'Run (horizontal)')}${field('sl-deg', 'Or: angle (°)')}${field('sl-pct', 'Or: slope (%)')}</div>
        <div class="wb-results" id="sl-out"></div>`;
      const calc = () => {
        const rise = num($('sl-rise').value);
        const run = num($('sl-run').value);
        const deg = num($('sl-deg').value);
        const pct = num($('sl-pct').value);
        let g = NaN; // rise / run
        if ([rise, run].every(isFinite) && run !== 0) g = rise / run;
        else if (isFinite(deg)) g = Math.tan(deg * Math.PI / 180);
        else if (isFinite(pct)) g = pct / 100;
        if (!isFinite(g)) { $('sl-out').innerHTML = ''; return; }
        const rows = [['Slope', `${fmt(g * 100, 3)} %`, '', true], ['Angle', `${fmt(Math.atan(g) * 180 / Math.PI, 3)}°`], ['Ratio', g ? `1 : ${fmt(1 / g, 3)}` : '—']];
        if ([rise, run].every(isFinite)) rows.push(['Length along the slope', fmt(Math.hypot(rise, run), 4)]);
        $('sl-out').innerHTML = rowsHtml(rows);
      };
      persist(this, el);
      el.addEventListener('input', calc);
      calc();
    },
  });

  // ==========================================================================
  // Navigation: list, search, pins
  // ==========================================================================
  let current = null;

  function pins() { return store.json('pins', []); }

  function score(t, q) {
    if (!q) return 1;
    const hay = `${t.name} ${t.desc} ${t.keys}`.toLowerCase();
    if (t.takes && t.takes.test(q)) return 100;
    const words = q.toLowerCase().split(/\s+/).filter(Boolean);
    let s = 0;
    for (const w of words) {
      if (t.name.toLowerCase().includes(w)) s += 10;
      else if (hay.includes(w)) s += 4;
      else return 0;
    }
    return s;
  }

  function showList() {
    current = null;
    $('wb-back').hidden = true;
    $('wb-title-row').hidden = true;
    const q = $('wb-search').value.trim();
    const p = pins();
    const matches = TOOLS.map((t) => [t, score(t, q)]).filter(([, s]) => s > 0);
    const card = (t) => `<button type="button" class="wb-tool" data-tool="${t.id}"><b>${p.includes(t.id) ? '★ ' : ''}${esc(t.name)}</b><span>${esc(t.desc)}</span></button>`;
    let html = '';
    if (q) {
      matches.sort((a, b) => b[1] - a[1]);
      html = matches.length ? `<div class="wb-tools">${matches.map(([t]) => card(t)).join('')}</div>` : '<div class="wb-empty">No tool matches. Try "awg", "m6", "psi" or "ohm".</div>';
    } else {
      const pinned = TOOLS.filter((t) => p.includes(t.id));
      if (pinned.length) html += `<div class="wb-group">Pinned</div><div class="wb-tools">${pinned.map(card).join('')}</div>`;
      for (const g of [...new Set(TOOLS.map((t) => t.group))]) {
        html += `<div class="wb-group">${esc(g)}</div><div class="wb-tools">${TOOLS.filter((t) => t.group === g).map(card).join('')}</div>`;
      }
    }
    $('wb-main').innerHTML = html;
    $('wb-main').scrollTop = 0;
  }

  function openTool(id, query) {
    const t = TOOLS.find((x) => x.id === id);
    if (!t) return showList();
    current = t;
    $('wb-back').hidden = false;
    $('wb-title-row').hidden = false;
    $('wb-title').textContent = t.name;
    const pinned = pins().includes(t.id);
    $('wb-pin').textContent = pinned ? '★' : '☆';
    $('wb-pin').classList.toggle('on', pinned);
    $('wb-pin').title = pinned ? 'Unpin' : 'Pin to the top';
    const main = $('wb-main');
    main.innerHTML = '';
    const box = document.createElement('div');
    box.className = 'wb-tool-body';
    box.style.display = 'flex';
    box.style.flexDirection = 'column';
    box.style.gap = '10px';
    main.appendChild(box);
    t.render(box, query);
    main.scrollTop = 0;
    store.set('last', t.id);
  }

  let started = false;
  function start() {
    if (started) return;
    started = true;
    $('wb-search').addEventListener('input', () => { if (current) { current = null; } showList(); });
    $('wb-search').addEventListener('keydown', (e) => {
      if (e.key === 'Enter') {
        const q = $('wb-search').value.trim();
        const best = TOOLS.map((t) => [t, score(t, q)]).filter(([, s]) => s > 0).sort((a, b) => b[1] - a[1])[0];
        if (best) {
          const t = best[0];
          openTool(t.id, t.takes && t.takes.test(q) ? q : undefined);
          $('wb-search').value = '';
        }
      } else if (e.key === 'Escape') {
        $('wb-search').value = '';
        showList();
      }
    });
    $('wb-back').addEventListener('click', () => { $('wb-search').value = ''; showList(); });
    $('wb-pin').addEventListener('click', () => {
      if (!current) return;
      const p = pins();
      const i = p.indexOf(current.id);
      if (i >= 0) p.splice(i, 1); else p.push(current.id);
      store.set('pins', JSON.stringify(p));
      openTool(current.id);
    });
    document.addEventListener('click', (e) => {
      const t = e.target.closest('[data-tool]');
      if (t) { $('wb-search').value = ''; openTool(t.dataset.tool); return; }
      const c = e.target.closest('[data-copy]');
      if (c && c.dataset.copy && c.dataset.copy !== '—') copy(c.dataset.copy);
    });
    showList();
  }

  // Opened outside Brum (no bridge): start anyway
  setTimeout(() => { if (!readyDone) start(); }, 600);

  // Test hook
  window.Workbench = { metricTapDrill, num, fmt, parseOhms, ohmsText, nearestE, eAtLeast, bandsFor, awgDiameter, awgArea, toBase, fromBase, drillInches, allDrills, fracName, METRIC_COARSE, UNIFIED, CLEARANCE, E12, E24, E96, TOOLS, setRegion: (r) => { region = r; } };
})();
