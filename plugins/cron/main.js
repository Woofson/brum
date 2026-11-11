// ==========================================================================
// Cron Builder: schedules in plain words, cron lines read back, next runs
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
      const reqId = 'cron-' + (++hostReqId);
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
  const pad = (n) => String(n).padStart(2, '0');
  const debounce = (fn, ms) => { let t; return (...a) => { clearTimeout(t); t = setTimeout(() => fn(...a), ms); }; };

  // ---------------- Names and words ----------------
  const MONTHS = ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December'];
  const DAYS = ['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday'];
  const SD_DAYS = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
  const ORD = (n) => n + (n % 10 === 1 && n % 100 !== 11 ? 'st' : n % 10 === 2 && n % 100 !== 12 ? 'nd' : n % 10 === 3 && n % 100 !== 13 ? 'rd' : 'th');

  function andList(items) {
    if (items.length <= 1) return items.join('');
    return items.slice(0, -1).join(', ') + ' and ' + items[items.length - 1];
  }

  function fmtTime(h, m, s) {
    const sec = s ? `:${pad(s)}` : '';
    if (region.time === '12') return `${h % 12 || 12}:${pad(m)}${sec} ${h < 12 ? 'AM' : 'PM'}`;
    return `${pad(h)}:${pad(m)}${sec}`;
  }

  function fmtDate(y, mo, d) {
    switch (region.date) {
      case 'dmy_dot': return `${pad(d)}.${pad(mo)}.${y}`;
      case 'dmy_slash': return `${pad(d)}/${pad(mo)}/${y}`;
      case 'mdy': return `${pad(mo)}/${pad(d)}/${y}`;
      case 'd_mon_y': return `${d} ${MONTHS[mo - 1].slice(0, 3)} ${y}`;
      default: return `${y}-${pad(mo)}-${pad(d)}`;
    }
  }

  // Sorted numbers → runs of consecutive values: [[1,5],[7,7]]
  function runs(values) {
    const out = [];
    for (const v of values) {
      const last = out[out.length - 1];
      if (last && v === last[1] + 1) last[1] = v;
      else out.push([v, v]);
    }
    return out;
  }

  // ---------------- Reading a cron line ----------------
  const FIELDS = [
    { key: 'sec', name: 'second', min: 0, max: 59 },
    { key: 'min', name: 'minute', min: 0, max: 59 },
    { key: 'hour', name: 'hour', min: 0, max: 23 },
    { key: 'dom', name: 'day of month', min: 1, max: 31 },
    { key: 'mon', name: 'month', min: 1, max: 12, names: ['JAN', 'FEB', 'MAR', 'APR', 'MAY', 'JUN', 'JUL', 'AUG', 'SEP', 'OCT', 'NOV', 'DEC'] },
    { key: 'dow', name: 'weekday', min: 0, max: 7, names: ['SUN', 'MON', 'TUE', 'WED', 'THU', 'FRI', 'SAT'] },
  ];

  const SHORTCUTS = {
    '@yearly': '0 0 1 1 *', '@annually': '0 0 1 1 *', '@monthly': '0 0 1 * *', '@weekly': '0 0 * * 0',
    '@daily': '0 0 * * *', '@midnight': '0 0 * * *', '@hourly': '0 * * * *',
  };

  function parseValue(tok, f) {
    const up = tok.toUpperCase();
    if (f.names) {
      const i = f.names.indexOf(up.slice(0, 3));
      if (i >= 0 && /^[A-Z]+$/.test(up)) return f.key === 'mon' ? i + 1 : i;
    }
    if (!/^\d+$/.test(tok)) {
      if (/^[LW#]|[LW#]$|#/.test(up)) throw new Error(`"${tok}" in the ${f.name} part uses L, W or #. Those belong to Quartz and some other schedulers, not to standard cron.`);
      throw new Error(`"${tok}" in the ${f.name} part is not a number${f.names ? ' or a name like ' + f.names[1] : ''}.`);
    }
    const n = Number(tok);
    if (n < f.min || n > f.max) throw new Error(`${f.name.charAt(0).toUpperCase() + f.name.slice(1)} ${n} is out of range (${f.min}–${f.key === 'dow' ? '6, or 7 for Sunday' : f.max}).`);
    return n;
  }

  function parseField(raw, f) {
    const set = new Set();
    const lo = f.min;
    const hi = f.key === 'dow' ? 6 : f.max;
    for (const part of raw.split(',')) {
      if (!part) throw new Error(`The ${f.name} part has an empty item (two commas in a row?).`);
      const [rangeText, stepText, extra] = part.split('/');
      if (extra !== undefined) throw new Error(`"${part}" in the ${f.name} part has more than one "/".`);
      let step = 1;
      if (stepText !== undefined) {
        if (!/^\d+$/.test(stepText) || Number(stepText) < 1) throw new Error(`The step "/${stepText}" in the ${f.name} part must be a whole number of 1 or more.`);
        step = Number(stepText);
      }
      let a;
      let b;
      if (rangeText === '*' || rangeText === '?') {
        [a, b] = [lo, hi];
      } else if (rangeText.includes('-')) {
        const [x, y] = rangeText.split('-');
        a = parseValue(x, f);
        b = parseValue(y, f);
        if (b < a) throw new Error(`The range ${rangeText} in the ${f.name} part goes backwards. Write the smaller value first.`);
      } else {
        a = parseValue(rangeText, f);
        b = stepText !== undefined ? hi : a;
      }
      for (let v = a; v <= b; v += step) set.add(f.key === 'dow' ? v % 7 : v);
    }
    const star = raw.startsWith('*') || raw.startsWith('?');
    return { raw, set: [...set].sort((x, y) => x - y), star, all: set.size === hi - lo + 1 };
  }

  // Text → { fields, hasSeconds, shortcut, text } | { reboot } | { error }
  function parseCron(text) {
    let t = String(text || '').trim().replace(/\s+/g, ' ');
    if (!t) return null;
    const lower = t.toLowerCase();
    if (lower === '@reboot') return { reboot: true, text: t };
    let shortcut = null;
    if (lower.startsWith('@')) {
      if (!SHORTCUTS[lower]) return { error: `"${t}" is not a cron shortcut. Known ones: @yearly, @monthly, @weekly, @daily, @hourly and @reboot.` };
      shortcut = lower;
      t = SHORTCUTS[lower];
    }
    let parts = t.split(' ');
    // A whole crontab line: keep the schedule, leave the command out
    let command = '';
    const NAME = /^(JAN|FEB|MAR|APR|MAY|JUN|JUL|AUG|SEP|OCT|NOV|DEC|SUN|MON|TUE|WED|THU|FRI|SAT)$/i;
    const cronish = (tok) => /^[\d*?,/\-LW#]+$/i.test(tok) || tok.split(',').every((x) => x.split(/[-/]/).every((y) => NAME.test(y) || /^\d+$/.test(y)));
    const firstOther = parts.findIndex((tok) => !cronish(tok));
    if (firstOther >= 5) {
      command = parts.slice(firstOther).join(' ');
      parts = parts.slice(0, firstOther);
    }
    if (parts.length === 7) return { error: 'This line has 7 parts. A year part is a Quartz extension; standard cron has 5 parts (minute, hour, day of month, month, weekday), or 6 with seconds first.' };
    if (parts.length !== 5 && parts.length !== 6) {
      return { error: `A cron line has 5 parts (minute, hour, day of month, month, weekday), or 6 with seconds first. This one has ${parts.length}.${parts.length > 6 ? ' If a command follows the schedule, paste only the schedule.' : ''}` };
    }
    const hasSeconds = parts.length === 6;
    const list = hasSeconds ? parts : ['0', ...parts];
    const fields = {};
    try {
      FIELDS.forEach((f, i) => { fields[f.key] = parseField(list[i], f); });
    } catch (e) {
      return { error: e.message };
    }
    return { fields, hasSeconds, shortcut, command, text: parts.join(' ') };
  }

  // ---------------- One schedule, whatever made it ----------------
  // { hours, minutes, seconds, dayMatch(y,mo,d,wd), words, cron, cronNote, systemd[], fields? }

  function daysInMonth(y, mo) { return new Date(Date.UTC(y, mo, 0)).getUTCDate(); }

  function scheduleFromCron(c) {
    const F = c.fields;
    const domSet = new Set(F.dom.set);
    const monSet = new Set(F.mon.set);
    const dowSet = new Set(F.dow.set);
    // Classic cron: when both day parts are limited, a day matching EITHER one runs
    const either = !F.dom.star && !F.dow.star;
    const dayMatch = (y, mo, d, wd) => {
      if (!monSet.has(mo)) return false;
      if (either) return domSet.has(d) || dowSet.has(wd);
      return domSet.has(d) && dowSet.has(wd);
    };
    return {
      hours: F.hour.set, minutes: F.min.set, seconds: F.sec.set,
      dayMatch,
      words: describe(c),
      cron: c.text,
      fields: F,
      hasSeconds: c.hasSeconds,
      either,
      systemd: systemdFromCron(c),
    };
  }

  // ---------------- Plain words ----------------
  function stepOf(field) {
    const m = field.raw.match(/^\*\/(\d+)$/);
    return m ? Number(m[1]) : null;
  }

  function listText(values, name) {
    return andList(runs(values).map(([a, b]) => (a === b ? name(a) : b === a + 1 ? `${name(a)}, ${name(b)}` : `${name(a)} to ${name(b)}`)));
  }

  function describeTime(F, hasSeconds) {
    const { sec, min, hour } = F;
    let secText = '';
    if (hasSeconds && !(sec.set.length === 1 && sec.set[0] === 0)) {
      const st = stepOf(sec);
      if (sec.all) secText = 'every second';
      else if (st) secText = `every ${st} seconds`;
      else secText = `at second ${listText(sec.set, String)}`;
    }
    const minStep = stepOf(min);
    const hourStep = stepOf(hour);
    const hourWindow = () => {
      const r = runs(hour.set);
      return andList(r.map(([a, b]) => `${fmtTime(a, 0)} and ${fmtTime(b, 59)}`).map((x) => `between ${x}`));
    };
    let text;
    if (min.all && hour.all) {
      text = secText || 'every minute';
      secText = '';
    } else if (min.all) {
      text = `${secText || 'every minute'}, ${hourWindow()}`;
      secText = '';
    } else if (minStep && hour.all) {
      text = `every ${minStep} minutes`;
    } else if (minStep) {
      text = `every ${minStep} minutes, ${hourWindow()}`;
    } else if (min.set.length === 1 && hour.all) {
      text = min.set[0] === 0 ? 'every hour, on the hour' : `every hour at ${min.set[0]} minutes past`;
    } else if (min.set.length === 1 && hourStep && hourStep > 1) {
      const shown = hour.set.slice(0, 4).map((h) => fmtTime(h, min.set[0])).join(', ') + (hour.set.length > 4 ? ', …' : '');
      text = `every ${hourStep} hours at ${min.set[0] === 0 ? 'the full hour' : min.set[0] + ' minutes past'} (${shown})`;
    } else if (min.set.length * hour.set.length <= 8) {
      const times = [];
      for (const h of hour.set) for (const m of min.set) times.push(fmtTime(h, m, hasSeconds && sec.set.length === 1 ? sec.set[0] : 0));
      if (hasSeconds && sec.set.length === 1) secText = '';
      text = `at ${andList(times)}`;
    } else if (min.set.length === 1) {
      text = `at ${min.set[0]} minutes past the hour, ${hourWindow()}`;
    } else {
      text = `at minutes ${listText(min.set, String)} past each hour, ${hourWindow()}`;
    }
    return secText ? `${secText}, ${text}` : text;
  }

  function describeDays(F) {
    const { dom, mon, dow } = F;
    const dayName = (d) => DAYS[d];
    const dowText = () => {
      const s = dow.set.join(',');
      if (dow.all) return 'every day';
      if (s === '1,2,3,4,5') return 'on weekdays (Monday to Friday)';
      if (s === '0,6') return 'on weekends';
      // Week order Monday first reads naturally: Saturday to Monday is written Sat, Sun, Mon
      return 'on ' + andList(dow.set.map(dayName));
    };
    const domText = () => {
      const st = stepOf(dom);
      if (st) return `every ${ORD(st)} day of the month (${dom.set.slice(0, 4).join(', ')}…)`;
      return `on the ${listText(dom.set, ORD)} of the month`;
    };
    let text;
    if (dom.star && dow.star) text = 'every day';
    else if (dom.star) text = dowText();
    else if (dow.star) text = domText();
    else text = `${domText()}, and also ${dowText().replace(/^on /, 'on every ')}`;
    if (!mon.all) {
      const r = runs(mon.set);
      const monthText = r.length === 1 && r[0][1] - r[0][0] >= 2 ? `from ${MONTHS[r[0][0] - 1]} to ${MONTHS[r[0][1] - 1]}` : `in ${listText(mon.set, (m) => MONTHS[m - 1])}`;
      text += `, ${monthText}`;
    }
    return text;
  }

  function describe(c) {
    const time = describeTime(c.fields, c.hasSeconds);
    const days = describeDays(c.fields);
    const sentence = days === 'every day' && /^every (second|minute|hour|\d+ (seconds|minutes|hours))/.test(time) ? time : `${time}, ${days}`;
    return sentence.charAt(0).toUpperCase() + sentence.slice(1);
  }

  // ---------------- systemd OnCalendar ----------------
  function sdField(f, values, lo, hi, name) {
    if (values.length === hi - lo + 1) return '*';
    const st = stepOf(f);
    if (st && values[0] === lo) return `${name ? name(lo) : pad(lo)}/${st}`;
    return runs(values).map(([a, b]) => {
      const n = (v) => (name ? name(v) : pad(v));
      if (a === b) return n(a);
      if (b === a + 1) return `${n(a)},${n(b)}`;
      return `${n(a)}..${n(b)}`;
    }).join(',');
  }

  function systemdLine(dowPart, mon, dom, hour, min, sec) {
    return `${dowPart ? dowPart + ' ' : ''}*-${mon}-${dom} ${hour}:${min}:${sec}`;
  }

  function systemdFromCron(c) {
    const F = c.fields;
    const mon = sdField(F.mon, F.mon.set, 1, 12);
    const dom = sdField(F.dom, F.dom.set, 1, 31);
    const hour = sdField(F.hour, F.hour.set, 0, 23);
    const min = sdField(F.min, F.min.set, 0, 59);
    const sec = sdField(F.sec, F.sec.set, 0, 59);
    // systemd weeks start on Monday, so Sunday goes last in its lists
    const dowVals = F.dow.set.map((d) => (d === 0 ? 7 : d)).sort((a, b) => a - b);
    const dow = F.dow.all ? '' : runs(dowVals).map(([a, b]) => {
      const n = (v) => SD_DAYS[v % 7];
      return a === b ? n(a) : b === a + 1 ? `${n(a)},${n(b)}` : `${n(a)}..${n(b)}`;
    }).join(',');
    if (!F.dom.star && !F.dow.star) {
      // systemd needs BOTH to match, cron EITHER: two timer lines give cron's meaning
      return [systemdLine('', mon, dom, hour, min, sec), systemdLine(dow, mon, '*', hour, min, sec)];
    }
    return [systemdLine(F.dow.star ? '' : dow, mon, F.dom.star ? '*' : dom, hour, min, sec)];
  }

  // ---------------- Time zones and next runs ----------------
  const LOCAL_ZONE = (() => { try { return Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC'; } catch (_) { return 'UTC'; } })();
  const partsCache = new Map();

  function partsIn(ms, tz) {
    let f = partsCache.get(tz);
    if (!f) {
      f = new Intl.DateTimeFormat('en-US', { timeZone: tz, hourCycle: 'h23', year: 'numeric', month: 'numeric', day: 'numeric', hour: 'numeric', minute: 'numeric', second: 'numeric', weekday: 'short' });
      partsCache.set(tz, f);
    }
    const o = {};
    for (const p of f.formatToParts(new Date(ms))) o[p.type] = p.value;
    return { y: Number(o.year), mo: Number(o.month), d: Number(o.day), h: Number(o.hour) % 24, mi: Number(o.minute), s: Number(o.second), wd: SD_DAYS.indexOf(o.weekday) };
  }

  function offsetMin(ms, tz) {
    const p = partsIn(ms, tz);
    return Math.round((Date.UTC(p.y, p.mo - 1, p.d, p.h, p.mi, p.s) - Math.floor(ms / 1000) * 1000) / 60000);
  }

  // Wall clock in a zone → ms, or null when that clock time does not exist (skipped by a clock change)
  function fromZoned(y, mo, d, h, mi, s, tz) {
    const guess = Date.UTC(y, mo - 1, d, h, mi, s);
    let t = guess - offsetMin(guess, tz) * 60000;
    const t2 = guess - offsetMin(t, tz) * 60000;
    if (t2 !== t) t = Math.min(t, t2);
    const p = partsIn(t, tz);
    if (p.h !== h || p.mi !== mi || p.d !== d) return null;
    return t;
  }

  // The next `count` runs after `fromMs`, looking up to 28 years ahead (29 February on a Monday, say)
  function nextRuns(sched, fromMs, tz, count = 5) {
    const out = [];
    const start = partsIn(fromMs, tz);
    const day0 = Date.UTC(start.y, start.mo - 1, start.d);
    for (let i = 0; i < 366 * 28 && out.length < count; i++) {
      const dd = new Date(day0 + i * 86400000);
      const y = dd.getUTCFullYear();
      const mo = dd.getUTCMonth() + 1;
      const d = dd.getUTCDate();
      if (!sched.dayMatch(y, mo, d, dd.getUTCDay())) continue;
      for (const h of sched.hours) {
        for (const m of sched.minutes) {
          for (const s of sched.seconds) {
            const t = fromZoned(y, mo, d, h, m, s, tz);
            if (t === null || t <= fromMs) continue;
            out.push(t);
            if (out.length >= count) return out;
          }
        }
      }
    }
    return out;
  }

  function relative(ms, now) {
    const a = Math.max(0, ms - now);
    if (a < 60000) return `in ${Math.max(1, Math.round(a / 1000))} s`;
    if (a < 3600000) return `in ${Math.round(a / 60000)} min`;
    if (a < 86400000 * 2) {
      const h = Math.floor(a / 3600000);
      const m = Math.round((a % 3600000) / 60000);
      return `in ${h} h${m ? ' ' + m + ' min' : ''}`;
    }
    if (a < 86400000 * 60) return `in ${Math.round(a / 86400000)} days`;
    return `in ${(a / (86400000 * 365.2425)).toFixed(1).replace(/\.0$/, '')} years`;
  }

  function shownZone() { return $('cr-zone').value || LOCAL_ZONE; }

  // ---------------- Showing a schedule ----------------
  function row(label, value, opts = {}) {
    return `<div class="cr-label">${esc(label)}</div><div class="cr-value${opts.multi ? ' multi' : ''}" data-copy="${esc(opts.copy === undefined ? value : opts.copy)}">${esc(value)}${opts.extra ? ` <small>${esc(opts.extra)}</small>` : ''}</div>`;
  }

  function shortcutFor(text) {
    const hit = Object.entries(SHORTCUTS).find(([k, v]) => v === text && k !== '@annually' && k !== '@midnight');
    return hit ? hit[0] : null;
  }

  function renderSchedule(sched, el) {
    const html = [];
    const now = Date.now();
    if (sched.cron) {
      html.push(`<div class="cr-line" data-copy="${esc(sched.cron)}" title="Click to copy"><code>${esc(sched.cron)}</code><small>click to copy</small></div>`);
    }
    html.push(`<div class="cr-words">${esc(sched.words)}</div>`);
    if (sched.cronNote) html.push(`<div class="cr-note">${sched.cronNote}</div>`);
    if (sched.either) html.push('<div class="cr-note"><b>Either day counts.</b> When both the day of month and the weekday are set, cron runs on days matching <b>either</b> one, not only days matching both. The systemd form below therefore needs two lines.</div>');
    const rows = [];
    if (sched.cron) {
      const sc = !sched.hasSeconds && shortcutFor(sched.cron);
      if (sc) rows.push(row('Shortcut', sc, { extra: 'same schedule' }));
      if (!sched.hasSeconds) rows.push(row('With seconds', '0 ' + sched.cron, { extra: '6 parts' }));
      rows.push(row('crontab line', `${sched.cron} /path/to/command`, { extra: 'replace the command' }));
    }
    if (sched.systemd && sched.systemd.length) {
      const lines = sched.systemd.map((l) => `OnCalendar=${l}`).join('\n');
      rows.push(row('systemd timer', lines, { multi: true, extra: sched.systemd.length > 1 ? 'both lines' : '' }));
    }
    if (rows.length) html.push(`<div class="cr-results">${rows.join('')}</div>`);
    if (sched.reboot) {
      html.push('<div class="cr-note">Runs once each time the computer starts, so there are no run times to show. With systemd, use <code>OnBootSec=</code> in the timer instead of <code>OnCalendar=</code>.</div>');
    } else {
      const tz = shownZone();
      const list = nextRuns(sched, now, tz);
      html.push(`<div class="cr-section">Next runs${tz !== LOCAL_ZONE ? ` (${esc(tz)})` : ''}</div>`);
      if (!list.length) {
        html.push('<div class="cr-note">This schedule never runs: no date in the next 28 years matches it (for example the 31st of a month that has 30 days).</div>');
      } else {
        html.push(`<div class="cr-runs">${list.map((t) => {
          const p = partsIn(t, tz);
          const line = `${DAYS[p.wd].slice(0, 3)} ${fmtDate(p.y, p.mo, p.d)} ${fmtTime(p.h, p.mi, sched.hasSeconds ? p.s : 0)}`;
          return `<div data-copy="${esc(line)}">${esc(line)}</div><div class="rel">${esc(relative(t, now))}</div>`;
        }).join('')}</div>`);
      }
      html.push('<div class="cr-hint">Cron uses the clock and time zone of the machine it runs on. Times skipped when clocks go forward are left out.</div>');
    }
    if (sched.fields) html.push(breakdown(sched.fields, sched.hasSeconds));
    el.innerHTML = html.join('');
  }

  function breakdown(F, hasSeconds) {
    const meaning = (key, f) => {
      if (f.all) return key === 'dow' ? 'every weekday' : key === 'mon' ? 'every month' : key === 'dom' ? 'every day' : `every ${FIELDS.find((x) => x.key === key).name}`;
      if (key === 'mon') return listText(f.set, (m) => MONTHS[m - 1]);
      if (key === 'dow') return listText(f.set, (d) => DAYS[d]);
      return listText(f.set, String);
    };
    const keys = hasSeconds ? ['sec', 'min', 'hour', 'dom', 'mon', 'dow'] : ['min', 'hour', 'dom', 'mon', 'dow'];
    const label = { sec: 'Second', min: 'Minute', hour: 'Hour', dom: 'Day of month', mon: 'Month', dow: 'Weekday' };
    return `<div class="cr-section">Part by part</div><table class="cr-table"><tbody>${keys.map((k) => `<tr><th>${label[k]}</th><td>${esc(F[k].raw)}</td><td>${esc(meaning(k, F[k]))}</td></tr>`).join('')}</tbody></table>`;
  }

  // ---------------- Read tab ----------------
  function renderRead() {
    const text = $('cr-input').value;
    const c = parseCron(text);
    const err = $('cr-error');
    err.hidden = !(c && c.error);
    err.textContent = c && c.error ? c.error : '';
    const out = $('cr-read-out');
    if (!c || c.error) { out.innerHTML = ''; return false; }
    if (c.reboot) {
      renderSchedule({ reboot: true, cron: '@reboot', words: 'Once, when the computer starts' }, out);
      return true;
    }
    const sched = scheduleFromCron(c);
    if (c.shortcut) sched.words += ` (${c.shortcut} is short for ${SHORTCUTS[c.shortcut]})`;
    if (c.command) sched.cronNote = `Only the schedule is read; the command after it (<code>${esc(c.command)}</code>) is left out.`;
    renderSchedule(sched, out);
    return true;
  }

  function remember(text) {
    const t = String(text || '').trim().replace(/\s+/g, ' ');
    if (!t) return;
    const list = store.json('recent', []).filter((x) => x !== t);
    list.unshift(t);
    store.set('recent', JSON.stringify(list.slice(0, 6)));
    renderRecent();
  }

  function renderRecent() {
    $('cr-recent').innerHTML = store.json('recent', []).map((t) => `<button type="button" class="cr-chip" data-recent="${esc(t)}">${esc(t)}</button>`).join('');
  }

  // ---------------- Build tab ----------------
  const MIN_STEPS = [1, 2, 3, 4, 5, 6, 10, 12, 15, 20, 30];
  const HOUR_STEPS = [1, 2, 3, 4, 6, 8, 12];
  let buildDays = [1];

  function weekOrder() {
    return region.week_start === 'sun' ? [0, 1, 2, 3, 4, 5, 6] : [1, 2, 3, 4, 5, 6, 0];
  }

  function option(value, label) { return `<option value="${esc(value)}">${esc(label)}</option>`; }

  function fillBuildForm() {
    $('cr-every-min').innerHTML = MIN_STEPS.map((n) => option(n, n)).join('');
    $('cr-every-hour').innerHTML = HOUR_STEPS.map((n) => option(n, n)).join('');
    $('cr-at-min').innerHTML = Array.from({ length: 60 }, (_, i) => option(i, pad(i))).join('');
    $('cr-at-minute').innerHTML = $('cr-at-min').innerHTML;
    // Hours in the Region & Formats style ("2 AM" or "02")
    $('cr-at-hour').innerHTML = Array.from({ length: 24 }, (_, i) => option(i, region.time === '12' ? `${i % 12 || 12} ${i < 12 ? 'AM' : 'PM'}` : pad(i))).join('');
    const hours = Array.from({ length: 24 }, (_, i) => option(i, fmtTime(i, 0)));
    $('cr-from-hour').innerHTML = hours.join('');
    $('cr-to-hour').innerHTML = Array.from({ length: 24 }, (_, i) => option(i, fmtTime(i, 59))).join('');
    $('cr-dom').innerHTML = Array.from({ length: 31 }, (_, i) => option(i + 1, ORD(i + 1))).join('') + option('L', 'last day');
    $('cr-nth-day').innerHTML = weekOrder().map((d) => option(d, DAYS[d])).join('');
    $('cr-year-day').innerHTML = Array.from({ length: 31 }, (_, i) => option(i + 1, i + 1)).join('');
    $('cr-year-month').innerHTML = MONTHS.map((m, i) => option(i + 1, m)).join('');
    $('cr-days').innerHTML = weekOrder().map((d) => `<button type="button" class="cr-day" data-day="${d}" aria-pressed="false">${DAYS[d].slice(0, 3)}</button>`).join('');
  }

  const FORM_IDS = ['cr-freq', 'cr-every-min', 'cr-every-hour', 'cr-at-min', 'cr-from-hour', 'cr-to-hour', 'cr-dom', 'cr-nth', 'cr-nth-day', 'cr-year-day', 'cr-year-month', 'cr-at-hour', 'cr-at-minute'];
  const FORM_CHECKS = ['cr-window', 'cr-weekdays-only'];

  function readForm() {
    const f = {};
    FORM_IDS.forEach((id) => { f[id] = $(id).value; });
    FORM_CHECKS.forEach((id) => { f[id] = $(id).checked; });
    f.days = buildDays.slice();
    f['cr-time'] = `${pad(f['cr-at-hour'])}:${pad(f['cr-at-minute'])}`;
    return f;
  }

  function writeForm(f) {
    FORM_IDS.forEach((id) => { if (f[id] !== undefined) $(id).value = f[id]; });
    FORM_CHECKS.forEach((id) => { if (f[id] !== undefined) $(id).checked = !!f[id]; });
    if (Array.isArray(f.days)) buildDays = f.days.map(Number);
    if (f['cr-time']) {
      const [h, m] = f['cr-time'].split(':').map(Number);
      $('cr-at-hour').value = String(h);
      $('cr-at-minute').value = String(m);
    }
  }

  // A cron line from the form, or a special schedule cron cannot write in one line
  function buildSchedule(f) {
    const [hh, mm] = (f['cr-time'] || '00:00').split(':').map(Number);
    const at = `${mm} ${hh}`;
    const fromCron = (line) => scheduleFromCron(parseCron(line));
    const freq = f['cr-freq'];
    const windowPart = () => {
      if (!f['cr-window']) return '*';
      const a = Number(f['cr-from-hour']);
      const b = Number(f['cr-to-hour']);
      if (a === b) return String(a);
      if (a < b) return `${a}-${b}`;
      return `${a}-23,0-${b}`;
    };
    const dowPart = f['cr-weekdays-only'] ? '1-5' : '*';
    switch (freq) {
      case 'minutes': {
        const n = Number(f['cr-every-min']);
        return fromCron(`${n === 1 ? '*' : '*/' + n} ${windowPart()} * * ${dowPart}`);
      }
      case 'hours': {
        const n = Number(f['cr-every-hour']);
        const hours = windowPart();
        let hourPart;
        if (hours === '*') hourPart = n === 1 ? '*' : `*/${n}`;
        else if (n === 1) hourPart = hours;
        else if (/^\d+-\d+$/.test(hours)) hourPart = `${hours}/${n}`;
        else {
          // Window across midnight: list the hours
          const a = Number(f['cr-from-hour']);
          const b = Number(f['cr-to-hour']);
          const list = [];
          for (let h = a, i = 0; i < 24; h = (h + n) % 24, i += n) {
            const inside = a <= b ? h >= a && h <= b : h >= a || h <= b;
            if (!inside) break;
            list.push(h);
          }
          hourPart = list.sort((x, y) => x - y).join(',');
        }
        return fromCron(`${f['cr-at-min']} ${hourPart} * * ${dowPart}`);
      }
      case 'daily': return fromCron(`${at} * * *`);
      case 'weekdays': return fromCron(`${at} * * 1-5`);
      case 'weekly': {
        const days = (f.days.length ? f.days : [1]).slice().sort((a, b) => a - b);
        const text = runs(days).map(([a, b]) => (a === b ? String(a) : b === a + 1 ? `${a},${b}` : `${a}-${b}`)).join(',');
        return fromCron(`${at} * * ${days.length === 7 ? '*' : text}`);
      }
      case 'monthly': {
        if (f['cr-dom'] !== 'L') {
          const s = fromCron(`${at} ${f['cr-dom']} * *`);
          if (Number(f['cr-dom']) > 28) s.cronNote = `Months without a ${ORD(Number(f['cr-dom']))} day are skipped${Number(f['cr-dom']) === 29 ? ' (February, except in leap years)' : ''}. Choose <b>last day</b> to run at the end of every month.`;
          return s;
        }
        return lastDaySchedule(hh, mm);
      }
      case 'nth': return nthSchedule(f['cr-nth'], Number(f['cr-nth-day']), hh, mm);
      case 'yearly': {
        const d = Number(f['cr-year-day']);
        const mo = Number(f['cr-year-month']);
        if (d > daysInMonth(2024, mo)) return { error: `${MONTHS[mo - 1]} has no day ${d}.` };
        const s = fromCron(`${at} ${d} ${mo} *`);
        if (mo === 2 && d === 29) s.cronNote = 'Runs only in leap years.';
        return s;
      }
      default: return fromCron(`${at} * * *`);
    }
  }

  function timeWords(h, m) { return `at ${fmtTime(h, m)}`; }

  // The last day of every month: not one cron line; a guard on days 28–31 does it
  function lastDaySchedule(h, m) {
    const guard = `${m} ${h} 28-31 * * [ "$(date -d tomorrow +\\%d)" = 01 ] && /path/to/command`;
    return {
      hours: [h], minutes: [m], seconds: [0],
      dayMatch: (y, mo, d) => d === daysInMonth(y, mo),
      words: `${timeWords(h, m).replace(/^a/, 'A')}, on the last day of every month`,
      cron: null,
      cronNote: `<b>Standard cron cannot say "last day of the month" in one line.</b> The usual way is to run on days 28–31 and let the command check that tomorrow is the 1st:<br><code>${esc(guard)}</code><br>The systemd timer below handles it directly.`,
      cronCopy: guard,
      systemd: [`*-*~01 ${pad(h)}:${pad(m)}:00`],
    };
  }

  // First/second/…/last <weekday> of the month
  function nthSchedule(nth, wd, h, m) {
    const which = { 1: 'first', 2: 'second', 3: 'third', 4: 'fourth', L: 'last' }[nth];
    const dayName = DAYS[wd];
    const isLast = nth === 'L';
    const n = Number(nth);
    const range = isLast ? '' : `${(n - 1) * 7 + 1}-${n * 7}`;
    const cronDow = wd === 0 ? 7 : wd;
    const guard = isLast
      ? `${m} ${h} * * ${wd} [ "$(date -d '+7 days' +\\%m)" != "$(date +\\%m)" ] && /path/to/command`
      : `${m} ${h} ${range} * * [ "$(date +\\%u)" = ${cronDow} ] && /path/to/command`;
    const sdDay = SD_DAYS[wd];
    const systemd = isLast ? `${sdDay} *-*~07/1 ${pad(h)}:${pad(m)}:00` : `${sdDay} *-*-${pad((n - 1) * 7 + 1)}..${pad(n * 7)} ${pad(h)}:${pad(m)}:00`;
    return {
      hours: [h], minutes: [m], seconds: [0],
      dayMatch: (y, mo, d, w) => w === wd && (isLast ? d + 7 > daysInMonth(y, mo) : Math.ceil(d / 7) === n),
      words: `${timeWords(h, m).replace(/^a/, 'A')}, on the ${which} ${dayName} of every month`,
      cron: null,
      cronNote: `<b>Standard cron cannot say "the ${which} ${dayName}" in one line.</b> Cron runs a day when <i>either</i> the date or the weekday matches, so <code>${esc(isLast ? `${m} ${h} 22-31 * ${wd}` : `${m} ${h} ${range} * ${wd}`)}</code> would also run on every ${dayName}. The usual way is to ${isLast ? `run every ${dayName} and let the command check that a week later is next month` : `run on days ${range.replace('-', '–')} and let the command check the weekday`}:<br><code>${esc(guard)}</code><br>The systemd timer below handles it directly.`,
      cronCopy: guard,
      systemd: [systemd],
    };
  }

  function renderBuild() {
    const f = readForm();
    store.set('form', JSON.stringify(f));
    const freq = f['cr-freq'];
    document.querySelectorAll('.cr-form-row[data-for]').forEach((r) => { r.hidden = !r.dataset.for.split(' ').includes(freq); });
    const win = f['cr-window'];
    $('cr-from-hour').disabled = !win;
    $('cr-to-hour').disabled = !win;
    document.querySelectorAll('.cr-day').forEach((b) => {
      const on = buildDays.includes(Number(b.dataset.day));
      b.classList.toggle('on', on);
      b.setAttribute('aria-pressed', on ? 'true' : 'false');
    });
    const out = $('cr-build-out');
    const sched = buildSchedule(f);
    if (sched.error) { out.innerHTML = `<div class="cr-error">${esc(sched.error)}</div>`; return; }
    renderSchedule(sched, out);
    if (!sched.cron && sched.cronCopy) {
      out.insertAdjacentHTML('afterbegin', `<div class="cr-line" data-copy="${esc(sched.cronCopy)}" title="Click to copy the crontab line with its check"><code>${esc(sched.cronCopy.split(' [')[0])} …</code><small>click to copy with check</small></div>`);
    }
  }

  // ---------------- Presets ----------------
  const PRESETS = [
    ['Every minute', { 'cr-freq': 'minutes', 'cr-every-min': '1' }],
    ['Every 5 minutes', { 'cr-freq': 'minutes', 'cr-every-min': '5' }],
    ['Every 15 minutes', { 'cr-freq': 'minutes', 'cr-every-min': '15' }],
    ['Every hour', { 'cr-freq': 'hours', 'cr-every-hour': '1', 'cr-at-min': '0' }],
    ['Every night at 02:30', { 'cr-freq': 'daily', 'cr-time': '02:30' }],
    ['Weekdays at 09:00', { 'cr-freq': 'weekdays', 'cr-time': '09:00' }],
    ['Sundays at 03:00', { 'cr-freq': 'weekly', days: [0], 'cr-time': '03:00' }],
    ['1st of the month at 04:00', { 'cr-freq': 'monthly', 'cr-dom': '1', 'cr-time': '04:00' }],
    ['First Monday at 09:00', { 'cr-freq': 'nth', 'cr-nth': '1', 'cr-nth-day': '1', 'cr-time': '09:00' }],
    ['New Year at midnight', { 'cr-freq': 'yearly', 'cr-year-day': '1', 'cr-year-month': '1', 'cr-time': '00:00' }],
  ];

  function applyPreset(i) {
    const p = PRESETS[i];
    if (!p) return;
    writeForm(Object.assign({ 'cr-window': false, 'cr-weekdays-only': false }, p[1]));
    renderBuild();
  }

  // ---------------- Tabs and start ----------------
  function showTab(tab) {
    const t = tab === 'read' ? 'read' : 'build';
    document.querySelectorAll('.cr-tab').forEach((b) => b.classList.toggle('active', b.dataset.tab === t));
    document.querySelectorAll('.cr-panel').forEach((p) => { p.hidden = p.dataset.panel !== t; });
    store.set('tab', t);
  }

  function renderAll() {
    renderBuild();
    renderRead();
  }

  function start() {
    if (started) return;
    started = true;
    region = Object.assign({ date: 'iso', time: '24', week_start: 'mon' }, region);
    fillBuildForm();
    const zones = [LOCAL_ZONE, 'UTC'].filter((z, i, a) => a.indexOf(z) === i);
    $('cr-zone').innerHTML = zones.map((z) => option(z, z === LOCAL_ZONE ? `your zone (${z})` : z)).join('');
    $('cr-zone').value = zones.includes(store.get('zone')) ? store.get('zone') : LOCAL_ZONE;
    $('cr-presets').innerHTML = PRESETS.map((p, i) => `<button type="button" class="cr-chip" data-preset="${i}">${esc(p[0])}</button>`).join('');
    const savedForm = store.json('form', null);
    if (savedForm) writeForm(savedForm);
    else writeForm({ 'cr-freq': 'daily', 'cr-time': '02:30', 'cr-every-min': '15', 'cr-from-hour': '8', 'cr-to-hour': '17' });
    FORM_IDS.concat(FORM_CHECKS).forEach((id) => {
      $(id).addEventListener('input', renderBuild);
      $(id).addEventListener('change', renderBuild);
    });
    const rememberLine = debounce(() => { if (parseCron($('cr-input').value) && !parseCron($('cr-input').value).error) remember($('cr-input').value); }, 1500);
    $('cr-input').addEventListener('input', () => { renderRead(); store.set('input', $('cr-input').value); rememberLine(); });
    $('cr-zone').addEventListener('change', () => { store.set('zone', $('cr-zone').value); renderAll(); });
    document.querySelectorAll('.cr-tab').forEach((b) => b.addEventListener('click', () => showTab(b.dataset.tab)));
    document.addEventListener('click', (e) => {
      const day = e.target.closest('[data-day]');
      if (day) {
        const d = Number(day.dataset.day);
        buildDays = buildDays.includes(d) ? buildDays.filter((x) => x !== d) : buildDays.concat(d);
        renderBuild();
        return;
      }
      const pre = e.target.closest('[data-preset]');
      if (pre) { applyPreset(Number(pre.dataset.preset)); return; }
      const r = e.target.closest('[data-recent]');
      if (r) {
        $('cr-input').value = r.dataset.recent;
        $('cr-input').dispatchEvent(new Event('input'));
        return;
      }
      const c = e.target.closest('[data-copy]');
      if (c && c.dataset.copy) copy(c.dataset.copy);
    });
    $('cr-input').value = store.get('input') || '*/15 9-17 * * 1-5';
    renderRecent();
    renderAll();
    showTab(store.get('tab') || 'build');
    // Keep "in 3 min" and the next runs current
    setInterval(() => { if (document.activeElement?.tagName !== 'SELECT') renderAll(); }, 30000);
  }

  // Opened outside Brum (no bridge): start anyway
  setTimeout(() => { if (!readyDone) start(); }, 600);

  // Test hook
  window.CronBuilder = { parseCron, scheduleFromCron, describe, systemdFromCron, nextRuns, buildSchedule, partsIn };
})();
