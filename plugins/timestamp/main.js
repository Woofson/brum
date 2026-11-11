// ==========================================================================
// Timestamp Converter: Unix time and dates both ways, time zones, durations
// ==========================================================================
(function () {
  'use strict';

  // ---------------- Bridge to Brum ----------------
  let hostReqId = 0;
  const pending = new Map();
  let saved = {};
  let region = {};
  let readyDone = false;
  let started = false;

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
      const reqId = 'timestamp-' + (++hostReqId);
      pending.set(reqId, { resolve, reject });
      window.parent.postMessage({ type: 'BRUM_REQ', reqId, action, payload }, '*');
    });
  }

  const store = {
    get: (key) => (Object.prototype.hasOwnProperty.call(saved, key) ? saved[key] : null),
    set: (key, value) => { saved[key] = String(value); host('storage.set', { key, value: String(value) }).catch(() => {}); },
    json: (key, dflt) => { try { const v = JSON.parse(store.get(key) || 'null'); return v === null ? dflt : v; } catch (_) { return dflt; } },
  };

  function copy(text) {
    host('clipboard.writeText', { text })
      .then(() => host('ui.notify', { message: 'Copied ' + text, opts: { type: 'success' } }).catch(() => {}))
      .catch((err) => host('ui.notify', { message: 'Could not copy: ' + err.message, opts: { type: 'warning' } }).catch(() => {}));
  }

  const $ = (id) => document.getElementById(id);
  const esc = (s) => String(s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
  const pad = (n, w = 2) => String(n).padStart(w, '0');
  const debounce = (fn, ms) => { let t; return (...a) => { clearTimeout(t); t = setTimeout(() => fn(...a), ms); }; };

  function fmtNum(n) {
    const seps = { space_comma: ' ', dot_comma: '.', comma_dot: ',', space_dot: ' ', apostrophe_dot: '’' }[region.number];
    const s = typeof n === 'bigint' ? n.toString() : String(Math.round(n));
    const neg = s.startsWith('-');
    const body = (neg ? s.slice(1) : s).replace(/\B(?=(\d{3})+(?!\d))/g, seps || ',');
    return (neg ? '-' : '') + body;
  }

  // ---------------- Time zones ----------------
  const LOCAL_ZONE = (() => { try { return Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC'; } catch (_) { return 'UTC'; } })();
  const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
  const DAYS = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
  const partsCache = new Map();

  function zoneValid(tz) {
    try { new Intl.DateTimeFormat('en-US', { timeZone: tz }); return true; } catch (_) { return false; }
  }

  function allZones() {
    try { if (Intl.supportedValuesOf) return Intl.supportedValuesOf('timeZone'); } catch (_) { /* older browser */ }
    return ['UTC', 'Europe/London', 'Europe/Paris', 'Europe/Berlin', 'Europe/Oslo', 'Europe/Stockholm', 'Europe/Helsinki', 'Europe/Moscow', 'America/New_York', 'America/Chicago', 'America/Denver', 'America/Los_Angeles', 'America/Sao_Paulo', 'Asia/Dubai', 'Asia/Kolkata', 'Asia/Shanghai', 'Asia/Singapore', 'Asia/Tokyo', 'Australia/Sydney', 'Pacific/Auckland'];
  }

  // Wall-clock fields of a moment in a zone
  function partsIn(ms, tz) {
    let f = partsCache.get(tz);
    if (!f) {
      f = new Intl.DateTimeFormat('en-US', { timeZone: tz, hourCycle: 'h23', year: 'numeric', month: 'numeric', day: 'numeric', hour: 'numeric', minute: 'numeric', second: 'numeric', weekday: 'short' });
      partsCache.set(tz, f);
    }
    const o = {};
    for (const p of f.formatToParts(new Date(ms))) o[p.type] = p.value;
    const year = Number(o.year) * (o.era === 'BC' ? -1 : 1);
    return { y: year, mo: Number(o.month), d: Number(o.day), h: Number(o.hour) % 24, mi: Number(o.minute), s: Number(o.second), wd: DAYS.indexOf(o.weekday) };
  }

  // Minutes the zone is ahead of UTC at that moment
  function offsetMin(ms, tz) {
    const p = partsIn(ms, tz);
    const asUtc = Date.UTC(p.y, p.mo - 1, p.d, p.h, p.mi, p.s);
    return Math.round((asUtc - Math.floor(ms / 1000) * 1000) / 60000);
  }

  // A wall-clock time in a zone → milliseconds since 1970. Times skipped by a clock change move forward.
  function fromZoned(y, mo, d, h, mi, s, msPart, tz) {
    const guess = Date.UTC(y, mo - 1, d, h, mi, s, msPart);
    let t = guess - offsetMin(guess, tz) * 60000;
    const t2 = guess - offsetMin(t, tz) * 60000;
    if (t2 !== t) t = Math.max(t, t2);
    return t;
  }

  function offsetText(min, colon = true) {
    const sign = min < 0 ? '-' : '+';
    const a = Math.abs(min);
    return `${sign}${pad(Math.floor(a / 60))}${colon ? ':' : ''}${pad(a % 60)}`;
  }

  function zoneAbbr(ms, tz) {
    try {
      const p = new Intl.DateTimeFormat('en-US', { timeZone: tz, timeZoneName: 'short' }).formatToParts(new Date(ms)).find((x) => x.type === 'timeZoneName');
      // "GMT+1" only repeats the offset shown beside it
      return p && !/^(GMT|UTC)[+-]/.test(p.value) ? p.value : '';
    } catch (_) { return ''; }
  }

  function zoneLabel(tz) {
    if (tz === 'UTC' || tz === 'Etc/UTC') return 'UTC';
    const city = tz.split('/').pop().replace(/_/g, ' ');
    return city;
  }

  // ---------------- Region & Formats ----------------
  function fmtDate(p) {
    const [y, m, d] = [p.y, pad(p.mo), pad(p.d)];
    switch (region.date) {
      case 'dmy_dot': return `${d}.${m}.${y}`;
      case 'dmy_slash': return `${d}/${m}/${y}`;
      case 'mdy': return `${m}/${d}/${y}`;
      case 'd_mon_y': return `${p.d} ${MONTHS[p.mo - 1]} ${y}`;
      default: return `${y}-${m}-${d}`;
    }
  }

  function fmtTime(p, seconds = true) {
    const sec = seconds ? `:${pad(p.s)}` : '';
    if (region.time === '12') return `${p.h % 12 || 12}:${pad(p.mi)}${sec} ${p.h < 12 ? 'AM' : 'PM'}`;
    return `${pad(p.h)}:${pad(p.mi)}${sec}`;
  }

  // ---------------- Reading input ----------------
  const MON_RE = '(Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)[a-z]*';
  const monthNum = (s) => MONTHS.findIndex((m) => m.toLowerCase() === s.slice(0, 3).toLowerCase()) + 1;
  const NAMED_ZONES = { UT: 0, UTC: 0, GMT: 0, Z: 0, EST: -300, EDT: -240, CST: -360, CDT: -300, MST: -420, MDT: -360, PST: -480, PDT: -420, CET: 60, CEST: 120 };

  function zoneOffset(z) {
    if (!z) return null;
    const u = z.toUpperCase();
    if (u in NAMED_ZONES) return NAMED_ZONES[u];
    const m = u.match(/^([+-])(\d{2}):?(\d{2})?$/);
    if (!m) return undefined;
    return (m[1] === '-' ? -1 : 1) * (Number(m[2]) * 60 + Number(m[3] || 0));
  }

  // Fraction digits → nanoseconds as BigInt
  const fracNs = (f) => BigInt(((f || '') + '000000000').slice(0, 9));

  // Builds a moment from fields; `zone` is an offset in minutes, or null for the chosen input zone
  function moment(y, mo, d, h, mi, s, frac, zone, inZone) {
    if (mo < 1 || mo > 12 || d < 1 || d > 31 || h > 24 || mi > 59 || s > 60) return { error: 'That date or time does not exist' };
    const check = new Date(Date.UTC(y, mo - 1, d));
    if (check.getUTCDate() !== d) return { error: `${MONTHS[mo - 1]} ${y} has no day ${d}` };
    const fns = fracNs(frac);
    let ms;
    if (zone === null || zone === undefined) ms = fromZoned(y, mo, d, h, mi, Math.min(s, 59), 0, inZone);
    else ms = Date.UTC(y, mo - 1, d, h, mi, Math.min(s, 59)) - zone * 60000;
    return { ns: BigInt(ms) * 1000000n + fns, zoned: zone !== null && zone !== undefined };
  }

  const DIGITS_FOR = { s: 'seconds', ms: 'milliseconds', us: 'microseconds', ns: 'nanoseconds' };

  function unitFor(intDigits) {
    if (intDigits <= 11) return 's';
    if (intDigits <= 14) return 'ms';
    if (intDigits <= 17) return 'us';
    return 'ns';
  }

  // Patterns tried on the whole text first, then searched for inside it (a log line)
  function patterns(inZone) {
    const yearNow = partsIn(Date.now(), inZone).y;
    return [
      {
        name: 'ISO 8601 / RFC 3339',
        re: /(\d{4})-(\d{2})-(\d{2})(?:[T ](\d{2}):(\d{2})(?::(\d{2})(?:[.,](\d{1,9}))?)?)?\s*(Z|[+-]\d{2}(?::?\d{2})?|UTC|GMT)?/i,
        make: (m) => moment(+m[1], +m[2], +m[3], +(m[4] || 0), +(m[5] || 0), +(m[6] || 0), m[7], zoneOffset(m[8]), inZone),
      },
      {
        name: 'ISO 8601 (basic form)',
        re: /(\d{4})(\d{2})(\d{2})T(\d{2})(\d{2})(\d{2})(?:[.,](\d{1,9}))?(Z|[+-]\d{4})?/i,
        make: (m) => moment(+m[1], +m[2], +m[3], +m[4], +m[5], +m[6], m[7], zoneOffset(m[8]), inZone),
      },
      {
        name: 'RFC 2822 / HTTP date',
        re: new RegExp(`(?:(?:Mon|Tue|Wed|Thu|Fri|Sat|Sun)[a-z]*,?\\s*)?(\\d{1,2})\\s+${MON_RE}\\s+(\\d{4})\\s+(\\d{1,2}):(\\d{2})(?::(\\d{2}))?\\s*([+-]\\d{4}|UT|UTC|GMT|Z|[ECMP][SD]T|CEST|CET)?`, 'i'),
        make: (m) => moment(+m[3], monthNum(m[2]), +m[1], +m[4], +m[5], +(m[6] || 0), '', zoneOffset(m[7]), inZone),
      },
      {
        name: 'Web server log',
        re: new RegExp(`(\\d{2})/${MON_RE}/(\\d{4}):(\\d{2}):(\\d{2}):(\\d{2})(?:\\s*([+-]\\d{4}))?`, 'i'),
        make: (m) => moment(+m[3], monthNum(m[2]), +m[1], +m[4], +m[5], +m[6], '', zoneOffset(m[7]), inZone),
      },
      {
        name: 'Day.month.year',
        re: /(\d{1,2})\.(\d{1,2})\.(\d{4})(?:,?\s+(\d{1,2}):(\d{2})(?::(\d{2}))?)?/,
        make: (m) => moment(+m[3], +m[2], +m[1], +(m[4] || 0), +(m[5] || 0), +(m[6] || 0), '', null, inZone),
      },
      {
        name: 'Year/month/day',
        re: /(\d{4})\/(\d{1,2})\/(\d{1,2})(?:[ T](\d{1,2}):(\d{2})(?::(\d{2}))?)?/,
        make: (m) => moment(+m[1], +m[2], +m[3], +(m[4] || 0), +(m[5] || 0), +(m[6] || 0), '', null, inZone),
      },
      {
        name: region.date === 'mdy' ? 'Month/day/year (from Region & Formats)' : 'Day/month/year (from Region & Formats)',
        re: /(\d{1,2})\/(\d{1,2})\/(\d{4})(?:,?\s+(\d{1,2}):(\d{2})(?::(\d{2}))?)?/,
        make: (m) => (region.date === 'mdy'
          ? moment(+m[3], +m[1], +m[2], +(m[4] || 0), +(m[5] || 0), +(m[6] || 0), '', null, inZone)
          : moment(+m[3], +m[2], +m[1], +(m[4] || 0), +(m[5] || 0), +(m[6] || 0), '', null, inZone)),
      },
      {
        name: 'System log (no year: the most recent such day is used)',
        re: new RegExp(`${MON_RE}\\s+(\\d{1,2})\\s+(\\d{2}):(\\d{2}):(\\d{2})(?:\\.(\\d{1,9}))?`, 'i'),
        make: (m) => {
          let r = moment(yearNow, monthNum(m[1]), +m[2], +m[3], +m[4], +m[5], m[6], null, inZone);
          if (!r.error && r.ns > BigInt(Date.now() + 86400000) * 1000000n) r = moment(yearNow - 1, monthNum(m[1]), +m[2], +m[3], +m[4], +m[5], m[6], null, inZone);
          return r;
        },
      },
    ];
  }

  // Text → { ns: BigInt, how: string, zoned: bool } or { error }
  function parse(text, unit, inZone) {
    const t = String(text || '').trim();
    if (!t) return null;
    if (/^now$/i.test(t)) return { ns: BigInt(Date.now()) * 1000000n, how: 'The current time', zoned: true };
    const num = t.replace(/[\s_’',](?=\d{3}\b)/g, '');
    const nm = num.match(/^(-?)(\d+)(?:\.(\d+))?$/);
    if (nm) {
      const u = unit === 'auto' ? unitFor(nm[2].length) : unit;
      const per = { s: 1000000000n, ms: 1000000n, us: 1000n, ns: 1n }[u];
      const digits = { s: 9, ms: 6, us: 3, ns: 0 }[u];
      let ns = BigInt(nm[2]) * per + (digits ? BigInt(((nm[3] || '') + '000000000').slice(0, digits)) : 0n);
      if (nm[1]) ns = -ns;
      const ms = Number(ns / 1000000n);
      if (Math.abs(ms) > 8.64e15) return { error: 'That number is outside the dates a computer can show (about 270,000 years either way)' };
      return { ns, how: `Unix time in ${DIGITS_FOR[u]}${unit === 'auto' ? ' (guessed from the number of digits)' : ''}`, zoned: true };
    }
    const list = patterns(inZone);
    for (const anchored of [true, false]) {
      for (const p of list) {
        const re = anchored ? new RegExp(`^\\s*\\[?${p.re.source}\\]?\\s*$`, p.re.flags) : p.re;
        const m = t.match(re);
        if (!m) continue;
        const r = p.make(m);
        if (r.error) return r;
        return { ns: r.ns, how: p.name + (anchored ? '' : ', found inside the text'), zoned: r.zoned };
      }
    }
    return { error: 'Not recognised. Try Unix time (1794384073), ISO 8601 (2026-11-11T09:03:16Z), RFC 2822 (Wed, 11 Nov 2026 09:03:16 +0100) or paste a log line.' };
  }

  // ---------------- Output formats ----------------
  const msOf = (ns) => Number((ns >= 0n ? ns : ns - 999999n) / 1000000n);
  const divFloor = (a, b) => (a >= 0n ? a / b : -((-a + b - 1n) / b));

  function fracText(ns, places) {
    const rem = ((ns % 1000000000n) + 1000000000n) % 1000000000n;
    if (!places) return '';
    const s = pad(rem.toString(), 9).slice(0, places);
    return '.' + s;
  }

  // Fewest fraction digits that keep the moment exact (0, 3, 6 or 9)
  function places(ns) {
    const rem = ((ns % 1000000000n) + 1000000000n) % 1000000000n;
    if (rem === 0n) return 0;
    if (rem % 1000000n === 0n) return 3;
    if (rem % 1000n === 0n) return 6;
    return 9;
  }

  function isoIn(ns, tz, colon = true) {
    const ms = msOf(ns);
    const p = partsIn(ms, tz);
    const off = offsetMin(ms, tz);
    const y = p.y < 0 ? '-' + pad(-p.y, 6) : pad(p.y, 4);
    const base = `${y}-${pad(p.mo)}-${pad(p.d)}T${pad(p.h)}:${pad(p.mi)}:${pad(p.s)}${fracText(ns, places(ns))}`;
    return base + (tz === 'UTC' ? 'Z' : offsetText(off, colon));
  }

  function rfc2822(ms, tz) {
    const p = partsIn(ms, tz);
    return `${DAYS[p.wd]}, ${pad(p.d)} ${MONTHS[p.mo - 1]} ${p.y} ${pad(p.h)}:${pad(p.mi)}:${pad(p.s)} ${offsetText(offsetMin(ms, tz), false)}`;
  }

  function httpDate(ms) {
    const p = partsIn(ms, 'UTC');
    return `${DAYS[p.wd]}, ${pad(p.d)} ${MONTHS[p.mo - 1]} ${p.y} ${pad(p.h)}:${pad(p.mi)}:${pad(p.s)} GMT`;
  }

  function webLog(ms, tz) {
    const p = partsIn(ms, tz);
    return `${pad(p.d)}/${MONTHS[p.mo - 1]}/${p.y}:${pad(p.h)}:${pad(p.mi)}:${pad(p.s)} ${offsetText(offsetMin(ms, tz), false)}`;
  }

  function regionLine(ms, tz) {
    const p = partsIn(ms, tz);
    return `${DAYS[p.wd]} ${fmtDate(p)} ${fmtTime(p)}`;
  }

  function isoWeek(p) {
    const d = new Date(Date.UTC(p.y, p.mo - 1, p.d));
    const day = (d.getUTCDay() + 6) % 7;
    d.setUTCDate(d.getUTCDate() - day + 3);
    const wy = d.getUTCFullYear();
    const first = new Date(Date.UTC(wy, 0, 4));
    const week = 1 + Math.round(((d - first) / 86400000 - 3 + ((first.getUTCDay() + 6) % 7)) / 7);
    return `${wy}-W${pad(week)}`;
  }

  function dayOfYear(p) {
    return Math.round((Date.UTC(p.y, p.mo - 1, p.d) - Date.UTC(p.y, 0, 1)) / 86400000) + 1;
  }

  const plural = (n, one, many) => `${fmtNum(n)} ${n === 1 ? one : many}`;

  // "3 hours 12 minutes ago", "in 2 days 4 hours"
  function relative(ms, now = Date.now()) {
    const diff = ms - now;
    const a = Math.abs(diff);
    if (a < 1000) return 'right now';
    const units = [[365.2425 * 86400000, 'year', 'years'], [86400000, 'day', 'days'], [3600000, 'hour', 'hours'], [60000, 'minute', 'minutes'], [1000, 'second', 'seconds']];
    const parts = [];
    let rest = a;
    for (const [size, one, many] of units) {
      const n = Math.floor(rest / size);
      if (n > 0 || parts.length) {
        if (n > 0) parts.push(plural(n, one, many));
        rest -= n * size;
      }
      if (parts.length === 2) break;
    }
    const text = parts.join(' ') || 'less than a second';
    return diff < 0 ? `${text} ago` : `in ${text}`;
  }

  function row(label, value, copyValue, extra) {
    const c = copyValue === undefined ? value : copyValue;
    return `<div class="ts-label">${esc(label)}</div><div class="ts-value" data-copy="${esc(c)}">${esc(value)}${extra ? ` <small>${esc(extra)}</small>` : ''}</div>`;
  }

  function momentRows(ns, opts = {}) {
    const ms = msOf(ns);
    const local = partsIn(ms, LOCAL_ZONE);
    const lo = offsetMin(ms, LOCAL_ZONE);
    const rows = [];
    rows.push(row('Your time', regionLine(ms, LOCAL_ZONE), undefined, `${zoneAbbr(ms, LOCAL_ZONE)} (UTC${offsetText(lo)})`.trim()));
    rows.push(row('UTC', regionLine(ms, 'UTC')));
    if (opts.zone && opts.zone !== LOCAL_ZONE && opts.zone !== 'UTC') {
      rows.push(row(zoneLabel(opts.zone), regionLine(ms, opts.zone), undefined, `${zoneAbbr(ms, opts.zone)} (UTC${offsetText(offsetMin(ms, opts.zone))})`.trim()));
    }
    rows.push(`<div class="ts-label">Relative</div><div class="ts-value" data-copy="${esc(relative(ms))}" data-relative="${ms}">${esc(relative(ms))}</div>`);
    rows.push(row('Unix seconds', divFloor(ns, 1000000000n).toString() + fracText(ns, places(ns) ? Math.max(places(ns), 3) : 0)));
    if (opts.full !== false) {
      rows.push(row('Unix milliseconds', divFloor(ns, 1000000n).toString()));
      rows.push(row('Unix microseconds', divFloor(ns, 1000n).toString()));
      rows.push(row('Unix nanoseconds', ns.toString()));
    }
    rows.push(row('ISO 8601 (UTC)', isoIn(ns, 'UTC')));
    rows.push(row('ISO 8601 (your zone)', isoIn(ns, LOCAL_ZONE), undefined, 'also RFC 3339'));
    if (opts.full !== false) {
      rows.push(row('RFC 2822', rfc2822(ms, LOCAL_ZONE), undefined, 'email'));
      rows.push(row('HTTP date', httpDate(ms)));
      rows.push(row('Web server log', webLog(ms, LOCAL_ZONE)));
      rows.push(row('Week / day of year', `${isoWeek(local)} · day ${dayOfYear(local)}`, isoWeek(local)));
    }
    return rows.join('');
  }

  // ---------------- Convert tab ----------------
  let current = null;

  function inputZone() { return $('ts-zone').value || LOCAL_ZONE; }

  function renderConvert() {
    const text = $('ts-input').value;
    const r = parse(text, $('ts-unit').value, inputZone());
    const err = $('ts-error');
    const note = $('ts-note');
    current = r && !r.error ? r : null;
    err.hidden = !(r && r.error);
    err.textContent = r && r.error ? r.error : '';
    note.hidden = !current;
    if (current) {
      const zoneUsed = !current.zoned && inputZone() !== LOCAL_ZONE ? ` · read as ${zoneLabel(inputZone())} time` : (!current.zoned ? ' · read as your time' : '');
      note.innerHTML = `<span class="ts-tag">${esc(current.how)}</span>${esc(zoneUsed)}`;
    }
    $('ts-results').innerHTML = current ? momentRows(current.ns, { zone: current.zoned ? null : inputZone() }) : '';
    renderZones();
  }

  function remember(text) {
    const t = String(text || '').trim();
    if (!t || /^now$/i.test(t)) return;
    const list = store.json('recent', []).filter((x) => x !== t);
    list.unshift(t);
    store.set('recent', JSON.stringify(list.slice(0, 6)));
    renderRecent();
  }

  function renderRecent() {
    const list = store.json('recent', []);
    $('ts-recent').innerHTML = list.map((t) => `<button type="button" class="ts-chip" data-recent="${esc(t)}" title="${esc(t)}">${esc(t.length > 28 ? t.slice(0, 27) + '…' : t)}</button>`).join('');
  }

  function fillZoneSelect() {
    const sel = $('ts-zone');
    const zones = [LOCAL_ZONE, 'UTC', ...favourites().filter((z) => z !== LOCAL_ZONE && z !== 'UTC')];
    const keep = store.get('inZone') || LOCAL_ZONE;
    sel.innerHTML = zones.map((z) => `<option value="${esc(z)}">${esc(z === LOCAL_ZONE ? `your zone (${zoneLabel(z)})` : zoneLabel(z))}</option>`).join('');
    sel.value = zones.includes(keep) ? keep : LOCAL_ZONE;
  }

  // ---------------- Time zones tab ----------------
  const DEFAULT_ZONES = ['UTC', 'Europe/London', 'America/New_York', 'America/Los_Angeles', 'Asia/Tokyo', 'Australia/Sydney'];

  function favourites() {
    const z = store.json('zones', null);
    const list = Array.isArray(z) ? z : [LOCAL_ZONE, ...DEFAULT_ZONES.filter((x) => x !== LOCAL_ZONE)];
    return list.filter(zoneValid);
  }

  function zonesMoment() {
    return current ? msOf(current.ns) : Date.now();
  }

  function renderZones() {
    const ms = zonesMoment();
    $('ts-zones-moment').textContent = current ? 'Showing the moment from Convert. Clear the box there to follow the current time.' : 'Showing the current time. Enter a moment in Convert to compare it across zones.';
    const here = partsIn(ms, LOCAL_ZONE);
    const hereDay = Date.UTC(here.y, here.mo - 1, here.d);
    const hereOff = offsetMin(ms, LOCAL_ZONE);
    $('ts-zones').innerHTML = favourites().map((tz) => {
      const p = partsIn(ms, tz);
      const off = offsetMin(ms, tz);
      const dayDiff = Math.round((Date.UTC(p.y, p.mo - 1, p.d) - hereDay) / 86400000);
      const day = dayDiff === 0 ? '' : dayDiff === 1 ? 'tomorrow' : dayDiff === -1 ? 'yesterday' : `${dayDiff > 0 ? '+' : ''}${dayDiff} days`;
      const delta = off - hereOff;
      const deltaText = delta === 0 ? 'same as you' : `${delta > 0 ? '+' : '-'}${Math.floor(Math.abs(delta) / 60)}${Math.abs(delta) % 60 ? ':' + pad(Math.abs(delta) % 60) : ''} h from you`;
      const line = `${DAYS[p.wd]} ${fmtDate(p)} ${fmtTime(p, false)}`;
      return `<div class="ts-zone${tz === LOCAL_ZONE ? ' here' : ''}">
        <div class="ts-zone-name">${esc(zoneLabel(tz))}<small>${esc(tz)}</small></div>
        <div class="ts-zone-time" data-copy="${esc(isoIn(BigInt(Math.floor(ms / 1000) * 1000) * 1000000n, tz))}" title="Click to copy as ISO 8601">${esc(line)}</div>
        <div class="ts-zone-sub">${esc([zoneAbbr(ms, tz), 'UTC' + offsetText(off), deltaText].filter(Boolean).join(' · '))}${day ? `<span class="ts-zone-day">${esc(day)}</span>` : ''}</div>
        <button type="button" class="ts-x" data-remove-zone="${esc(tz)}" title="Remove">×</button>
      </div>`;
    }).join('') || '<div class="ts-hint">No zones. Add one above.</div>';
  }

  function findZone(text) {
    const t = String(text || '').trim();
    if (!t) return null;
    if (/^(utc|gmt|z)$/i.test(t)) return 'UTC';
    const zones = allZones();
    const norm = (s) => s.toLowerCase().replace(/[\s_]+/g, '_');
    const exact = zones.find((z) => norm(z) === norm(t));
    if (exact) return exact;
    const city = zones.find((z) => norm(z.split('/').pop()) === norm(t));
    if (city) return city;
    const part = zones.filter((z) => norm(z).includes(norm(t)));
    if (part.length === 1) return part[0];
    if (zoneValid(t)) return t;
    // Browsers list some zones under old names (Asia/Calcutta for Kolkata) but know the current ones
    const name = t.trim().split(/[\s_]+/).map((w) => w.charAt(0).toUpperCase() + w.slice(1).toLowerCase()).join('_');
    for (const area of ['Europe', 'Asia', 'America', 'Africa', 'Australia', 'Pacific', 'Atlantic', 'Indian', 'Antarctica']) {
      if (zoneValid(`${area}/${name}`)) return `${area}/${name}`;
    }
    return part[0] || null;
  }

  function addZone() {
    const input = $('ts-zone-add');
    const err = $('ts-zone-error');
    const tz = findZone(input.value);
    if (!tz) {
      err.hidden = false;
      err.textContent = input.value.trim() ? `No time zone matches "${input.value.trim()}". Pick one from the list, like Europe/Oslo or Asia/Tokyo.` : 'Type a city or zone name first.';
      return;
    }
    err.hidden = true;
    const list = favourites();
    if (!list.includes(tz)) list.push(tz);
    store.set('zones', JSON.stringify(list));
    input.value = '';
    fillZoneSelect();
    renderZones();
  }

  function removeZone(tz) {
    store.set('zones', JSON.stringify(favourites().filter((z) => z !== tz)));
    fillZoneSelect();
    renderConvert();
  }

  // ---------------- Durations tab ----------------
  function parseOrNow(text) {
    const t = String(text || '').trim();
    if (!t) return { ns: BigInt(Date.now()) * 1000000n, how: 'now' };
    return parse(t, 'auto', LOCAL_ZONE);
  }

  function breakdown(msAbs) {
    const units = [[86400000, 'day', 'days'], [3600000, 'hour', 'hours'], [60000, 'minute', 'minutes'], [1000, 'second', 'seconds']];
    let rest = msAbs;
    const parts = [];
    for (const [size, one, many] of units) {
      const n = Math.floor(rest / size);
      rest -= n * size;
      if (n) parts.push(plural(n, one, many));
    }
    if (rest) parts.push(`${fmtNum(rest)} ms`);
    return parts.join(', ') || '0 seconds';
  }

  // Calendar difference in the local zone: years, months, days, then the clock
  function calendarDiff(a, b) {
    const [from, to] = a <= b ? [a, b] : [b, a];
    let months = 0;
    for (let step = 1200 * 12; step >= 1; step = Math.floor(step / 2)) {
      while (addMonths(from, months + step) <= to) months += step;
    }
    const mid = addMonths(from, months);
    const rest = to - mid;
    const y = Math.floor(months / 12);
    const m = months % 12;
    const out = [];
    if (y) out.push(plural(y, 'year', 'years'));
    if (m) out.push(plural(m, 'month', 'months'));
    return out.length ? `${out.join(', ')}${rest ? ', ' + breakdown(rest) : ''}` : null;
  }

  function addMonths(ms, n) {
    const p = partsIn(ms, LOCAL_ZONE);
    const total = p.y * 12 + (p.mo - 1) + n;
    const y = Math.floor(total / 12);
    const mo = (total % 12 + 12) % 12 + 1;
    const last = new Date(Date.UTC(y, mo, 0)).getUTCDate();
    return fromZoned(y, mo, Math.min(p.d, last), p.h, p.mi, p.s, ((ms % 1000) + 1000) % 1000, LOCAL_ZONE);
  }

  function addDays(ms, n) {
    const p = partsIn(ms, LOCAL_ZONE);
    const d = new Date(Date.UTC(p.y, p.mo - 1, p.d + n));
    return fromZoned(d.getUTCFullYear(), d.getUTCMonth() + 1, d.getUTCDate(), p.h, p.mi, p.s, ((ms % 1000) + 1000) % 1000, LOCAL_ZONE);
  }

  function renderBetween() {
    const a = parseOrNow($('ts-from').value);
    const b = parseOrNow($('ts-to').value);
    const err = $('ts-between-error');
    const bad = (a && a.error) || (b && b.error);
    err.hidden = !bad;
    err.textContent = bad ? ((a && a.error) ? 'From: ' + a.error : 'To: ' + b.error) : '';
    if (bad) { $('ts-between').innerHTML = ''; return; }
    const ams = msOf(a.ns);
    const bms = msOf(b.ns);
    const diff = bms - ams;
    const abs = Math.abs(diff);
    const rows = [];
    rows.push(row('From', regionLine(ams, LOCAL_ZONE), undefined, a.how === 'now' ? 'now' : ''));
    rows.push(row('To', regionLine(bms, LOCAL_ZONE), undefined, b.how === 'now' ? 'now' : ''));
    rows.push(row('Difference', breakdown(abs), undefined, diff < 0 ? 'To is before From' : ''));
    const cal = calendarDiff(ams, bms);
    if (cal) rows.push(row('In calendar terms', cal));
    rows.push(row('Total weeks', (abs / 604800000).toFixed(2).replace(/\.?0+$/, '')));
    rows.push(row('Total days', (abs / 86400000).toFixed(2).replace(/\.?0+$/, '')));
    rows.push(row('Total hours', (abs / 3600000).toFixed(2).replace(/\.?0+$/, '')));
    rows.push(row('Total minutes', fmtNum(Math.floor(abs / 60000))));
    rows.push(row('Total seconds', fmtNum(Math.floor(abs / 1000)), String(Math.floor(abs / 1000))));
    $('ts-between').innerHTML = rows.join('');
  }

  // "90d", "2h 30m", "1 year 2 months", "P1Y2M3DT4H5M6S", "-15m"
  function parsePeriod(text) {
    let t = String(text || '').trim();
    if (!t) return null;
    const out = { y: 0, mo: 0, w: 0, d: 0, h: 0, mi: 0, s: 0, ms: 0 };
    const iso = t.toUpperCase().match(/^P(?:(\d+(?:[.,]\d+)?)Y)?(?:(\d+(?:[.,]\d+)?)M)?(?:(\d+(?:[.,]\d+)?)W)?(?:(\d+(?:[.,]\d+)?)D)?(?:T(?:(\d+(?:[.,]\d+)?)H)?(?:(\d+(?:[.,]\d+)?)M)?(?:(\d+(?:[.,]\d+)?)S)?)?$/);
    if (iso && t.length > 1) {
      const n = (v) => Number(String(v || 0).replace(',', '.'));
      [out.y, out.mo, out.w, out.d, out.h, out.mi, out.s] = [n(iso[1]), n(iso[2]), n(iso[3]), n(iso[4]), n(iso[5]), n(iso[6]), n(iso[7])];
      return out;
    }
    let sign = 1;
    if (/^[-+]/.test(t)) { sign = t[0] === '-' ? -1 : 1; t = t.slice(1); }
    const re = /(\d+(?:[.,]\d+)?)\s*(years?|yrs?|y|months?|mo|weeks?|wk|w|days?|d|hours?|hrs?|h|minutes?|mins?|m|seconds?|secs?|s|milliseconds?|ms)\b/gi;
    let m;
    let any = false;
    while ((m = re.exec(t))) {
      const v = Number(m[1].replace(',', '.')) * sign;
      const u = m[2].toLowerCase();
      if (u === 'ms' || u.startsWith('milli')) out.ms += v;
      else if (u.startsWith('y')) out.y += v;
      else if (u === 'mo' || u.startsWith('month')) out.mo += v;
      else if (u.startsWith('w')) out.w += v;
      else if (u.startsWith('d')) out.d += v;
      else if (u.startsWith('h')) out.h += v;
      else if (u === 'm' || u.startsWith('min')) out.mi += v;
      else out.s += v;
      any = true;
    }
    const rest = t.replace(re, '').replace(/[\s,]|and/gi, '');
    if (!any || rest) return { error: 'Not recognised. Use numbers with units like 90d, 2h 30m, 1 year 2 months, or ISO 8601 like P1DT12H.' };
    return out;
  }

  function renderAdd() {
    const base = parseOrNow($('ts-base').value);
    const per = parsePeriod($('ts-period').value);
    const err = $('ts-add-error');
    const problem = base && base.error ? 'From: ' + base.error : per && per.error ? per.error : '';
    err.hidden = !problem;
    err.textContent = problem;
    if (problem || !per) { $('ts-add').innerHTML = ''; return; }
    const sign = Number($('ts-op').value);
    let ms = msOf(base.ns);
    const sub = Number(((base.ns % 1000000n) + 1000000n) % 1000000n);
    // Years, months, weeks and days follow the calendar (same clock time, month ends respected)
    const months = Math.trunc((per.y * 12 + per.mo) * sign);
    if (months) ms = addMonths(ms, months);
    const wholeDays = Math.trunc((per.w * 7 + per.d) * sign);
    if (wholeDays) ms = addDays(ms, wholeDays);
    const fracDays = ((per.w * 7 + per.d) * sign) - wholeDays;
    const fracMonths = ((per.y * 12 + per.mo) * sign) - months;
    ms += Math.round(fracDays * 86400000 + fracMonths * 30.436875 * 86400000);
    ms += Math.round((per.h * 3600000 + per.mi * 60000 + per.s * 1000 + per.ms) * sign);
    const ns = BigInt(ms) * 1000000n + BigInt(sub);
    $('ts-add').innerHTML = row('Start', regionLine(msOf(base.ns), LOCAL_ZONE), undefined, base.how === 'now' ? 'now' : '') + momentRows(ns, { full: false });
  }

  function showDur(mode) {
    const m = mode === 'add' ? 'add' : 'between';
    document.querySelectorAll('.ts-seg-btn').forEach((b) => b.classList.toggle('active', b.dataset.dur === m));
    document.querySelectorAll('.ts-dur-mode').forEach((p) => { p.hidden = p.dataset.durPanel !== m; });
    store.set('durMode', m);
  }

  // ---------------- Live clock and relative times ----------------
  function tick() {
    const now = Date.now();
    const p = partsIn(now, LOCAL_ZONE);
    const clock = $('ts-clock');
    clock.dataset.copy = String(Math.floor(now / 1000));
    clock.innerHTML = `Now <b>${esc(Math.floor(now / 1000))}</b> · ${esc(DAYS[p.wd])} ${esc(fmtDate(p))} ${esc(fmtTime(p))} ${esc(zoneAbbr(now, LOCAL_ZONE))}`;
    document.querySelectorAll('[data-relative]').forEach((el) => {
      const t = relative(Number(el.dataset.relative), now);
      el.textContent = t;
      el.dataset.copy = t;
    });
    if (!current && !document.querySelector('[data-panel="zones"]').hidden) renderZones();
    const durOpen = !document.querySelector('[data-panel="dur"]').hidden;
    if (durOpen && (!$('ts-from').value.trim() || !$('ts-to').value.trim()) && document.activeElement?.tagName !== 'INPUT') renderBetween();
  }

  // ---------------- Tabs and start ----------------
  function showTab(tab) {
    const t = ['convert', 'zones', 'dur'].includes(tab) ? tab : 'convert';
    document.querySelectorAll('.ts-tab').forEach((b) => b.classList.toggle('active', b.dataset.tab === t));
    document.querySelectorAll('.ts-panel').forEach((p) => { p.hidden = p.dataset.panel !== t; });
    store.set('tab', t);
    if (t === 'zones') renderZones();
  }

  function start() {
    if (started) return;
    started = true;
    region = Object.assign({ date: 'iso', time: '24' }, region);
    $('ts-zone-list').innerHTML = allZones().map((z) => `<option value="${esc(z)}"></option>`).join('');
    fillZoneSelect();
    $('ts-unit').value = store.get('unit') || 'auto';
    const rememberInput = debounce(() => { if (current) remember($('ts-input').value); }, 1500);
    $('ts-input').addEventListener('input', () => { renderConvert(); store.set('input', $('ts-input').value); rememberInput(); });
    $('ts-unit').addEventListener('change', () => { store.set('unit', $('ts-unit').value); renderConvert(); });
    $('ts-zone').addEventListener('change', () => { store.set('inZone', $('ts-zone').value); renderConvert(); });
    $('ts-now').addEventListener('click', () => {
      $('ts-input').value = String(Math.floor(Date.now() / 1000));
      $('ts-input').dispatchEvent(new Event('input'));
      $('ts-input').focus();
    });
    $('ts-zone-add-btn').addEventListener('click', addZone);
    $('ts-zone-add').addEventListener('keydown', (e) => { if (e.key === 'Enter') addZone(); });
    document.querySelectorAll('.ts-tab').forEach((b) => b.addEventListener('click', () => showTab(b.dataset.tab)));
    document.querySelectorAll('.ts-seg-btn').forEach((b) => b.addEventListener('click', () => showDur(b.dataset.dur)));
    ['ts-from', 'ts-to'].forEach((id) => $(id).addEventListener('input', () => { store.set(id, $(id).value); renderBetween(); }));
    ['ts-base', 'ts-period'].forEach((id) => $(id).addEventListener('input', () => { store.set(id, $(id).value); renderAdd(); }));
    $('ts-op').addEventListener('change', () => { store.set('op', $('ts-op').value); renderAdd(); });
    // One click copies any result
    document.addEventListener('click', (e) => {
      const r = e.target.closest('[data-recent]');
      if (r) {
        $('ts-input').value = r.dataset.recent;
        $('ts-input').dispatchEvent(new Event('input'));
        return;
      }
      const x = e.target.closest('[data-remove-zone]');
      if (x) { removeZone(x.dataset.removeZone); return; }
      const c = e.target.closest('[data-copy]');
      if (c && c.dataset.copy) copy(c.dataset.copy);
    });
    $('ts-input').value = store.get('input') || '';
    ['ts-from', 'ts-to', 'ts-base', 'ts-period'].forEach((id) => { $(id).value = store.get(id) || ''; });
    $('ts-op').value = store.get('op') || '1';
    renderRecent();
    renderConvert();
    renderBetween();
    renderAdd();
    showDur(store.get('durMode') || 'between');
    showTab(store.get('tab') || 'convert');
    tick();
    setInterval(tick, 1000);
  }

  // Opened outside Brum (no bridge): start anyway
  setTimeout(() => { if (!readyDone) start(); }, 600);

  // Test hook
  window.TimestampConv = { parse, parsePeriod, isoIn, rfc2822, httpDate, webLog, fromZoned, offsetMin, partsIn, relative, isoWeek, addMonths, findZone, msOf };
})();
