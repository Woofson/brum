// ==========================================================================
// Subnet Calculator: IPv4 and IPv6 subnets, splitting, combining, reference
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
      const reqId = 'subnet-' + (++hostReqId);
      pending.set(reqId, { resolve, reject });
      window.parent.postMessage({ type: 'BRUM_REQ', reqId, action, payload }, '*');
    });
  }

  const store = {
    get: (key) => (Object.prototype.hasOwnProperty.call(saved, key) ? saved[key] : null),
    set: (key, value) => { saved[key] = String(value); host('storage.set', { key, value: String(value) }).catch(() => {}); },
  };

  function copy(text) {
    host('clipboard.writeText', { text })
      .then(() => host('ui.notify', { message: 'Copied ' + text, opts: { type: 'success' } }).catch(() => {}))
      .catch((err) => host('ui.notify', { message: 'Could not copy: ' + err.message, opts: { type: 'warning' } }).catch(() => {}));
  }

  // Numbers in the user's style (Settings → Region & Formats)
  function fmtNum(n) {
    const seps = { space_comma: ' ', dot_comma: '.', comma_dot: ',', space_dot: ' ', apostrophe_dot: '’' }[region.number];
    const s = typeof n === 'bigint' ? n.toString() : String(Math.round(n));
    if (!seps) return typeof n === 'bigint' ? n.toLocaleString() : Number(n).toLocaleString();
    return s.replace(/\B(?=(\d{3})+(?!\d))/g, seps);
  }

  const $ = (id) => document.getElementById(id);
  const esc = (s) => String(s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));

  // ---------------- IPv4 ----------------
  function parseV4(s) {
    const m = /^\s*(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})\s*$/.exec(s || '');
    if (!m) return null;
    const parts = m.slice(1).map(Number);
    if (parts.some((p) => p > 255)) return null;
    return ((parts[0] << 24) >>> 0) + (parts[1] << 16) + (parts[2] << 8) + parts[3];
  }

  const v4 = (n) => [n >>> 24, (n >>> 16) & 255, (n >>> 8) & 255, n & 255].join('.');
  const maskOf = (p) => (p === 0 ? 0 : (0xFFFFFFFF << (32 - p)) >>> 0);

  function prefixOfMask(m) {
    for (let p = 0; p <= 32; p++) if (maskOf(p) === m) return p;
    return -1;
  }

  // "192.168.1.10/24", "10.0.0.1 255.255.0.0", "10.0.0.1/255.255.0.0", "10.0.0.1" (= /32)
  function parseV4Net(s) {
    const t = (s || '').trim();
    if (!t) return { empty: true };
    const m = /^([\d.]+)\s*(?:[/\s]\s*([\d.]+))?$/.exec(t);
    if (!m) return { error: 'Type an address like 192.168.1.10/24' };
    const ip = parseV4(m[1]);
    if (ip === null) return { error: `${m[1]} is not a valid IPv4 address` };
    let prefix = 32;
    if (m[2] !== undefined) {
      if (/^\d{1,2}$/.test(m[2])) {
        prefix = Number(m[2]);
        if (prefix > 32) return { error: 'The prefix must be between 0 and 32' };
      } else {
        const mask = parseV4(m[2]);
        prefix = mask === null ? -1 : prefixOfMask(mask);
        if (prefix < 0) return { error: `${m[2]} is not a valid netmask (the 1-bits must be at the start)` };
      }
    }
    return { ip, prefix };
  }

  const V4_RANGES = [
    ['0.0.0.0/8', 'This network', 'Only valid as a source address'],
    ['10.0.0.0/8', 'Private', 'Home and company networks (RFC 1918)'],
    ['100.64.0.0/10', 'Shared (CGNAT)', 'Carrier-grade NAT; also used by Tailscale'],
    ['127.0.0.0/8', 'Loopback', 'This computer'],
    ['169.254.0.0/16', 'Link-local', 'Automatic address when there is no DHCP'],
    ['172.16.0.0/12', 'Private', 'Home and company networks (RFC 1918); Docker uses 172.17.0.0/16'],
    ['192.0.0.0/24', 'IETF protocol assignments', ''],
    ['192.0.2.0/24', 'Documentation', 'TEST-NET-1, for examples'],
    ['192.168.0.0/16', 'Private', 'Home and company networks (RFC 1918)'],
    ['198.18.0.0/15', 'Benchmarking', 'Network test equipment'],
    ['198.51.100.0/24', 'Documentation', 'TEST-NET-2, for examples'],
    ['203.0.113.0/24', 'Documentation', 'TEST-NET-3, for examples'],
    ['224.0.0.0/4', 'Multicast', ''],
    ['240.0.0.0/4', 'Reserved', 'Not used on the internet'],
    ['255.255.255.255/32', 'Broadcast', 'Limited broadcast'],
  ];

  function v4Kind(ip) {
    let best = null;
    for (const [cidr, name] of V4_RANGES) {
      const n = parseV4Net(cidr);
      if (((ip & maskOf(n.prefix)) >>> 0) === n.ip && (!best || n.prefix > best.prefix)) best = { prefix: n.prefix, name };
    }
    return best ? best.name : 'Public';
  }

  function v4Info(ip, prefix) {
    const mask = maskOf(prefix);
    const network = (ip & mask) >>> 0;
    const broadcast = (network | (~mask >>> 0)) >>> 0;
    const total = 2 ** (32 - prefix);
    let first = network + 1;
    let last = broadcast - 1;
    let usable = total - 2;
    if (prefix === 32) { first = last = network; usable = 1; }
    if (prefix === 31) { first = network; last = broadcast; usable = 2; }
    return { ip, prefix, mask, network, broadcast, total, first, last, usable, wildcard: (~mask) >>> 0 };
  }

  function row(label, value, note) {
    return `<div class="sn-label">${esc(label)}</div><div class="sn-value" data-copy="${esc(value)}" title="Click to copy">${esc(value)}${note ? `<small>${esc(note)}</small>` : ''}</div>`;
  }

  function renderV4() {
    const parsed = parseV4Net($('sn-v4-input').value);
    const err = $('sn-v4-error');
    const out = $('sn-v4-results');
    const bin = $('sn-v4-binary');
    err.hidden = true;
    if (parsed.empty || parsed.error) {
      out.innerHTML = '';
      bin.innerHTML = '';
      if (parsed.error) { err.textContent = parsed.error; err.hidden = false; }
      v4Current = null;
      renderContains();
      return;
    }
    const i = v4Info(parsed.ip, parsed.prefix);
    v4Current = i;
    const kind = v4Kind(i.network);
    out.innerHTML = [
      row('Network', `${v4(i.network)}/${i.prefix}`),
      row('Netmask', v4(i.mask), `/${i.prefix}`),
      row('Wildcard mask', v4(i.wildcard), 'for ACLs'),
      row('Broadcast', i.prefix >= 31 ? '—' : v4(i.broadcast), i.prefix === 31 ? 'none on a point-to-point /31' : ''),
      row('First usable', v4(i.first)),
      row('Last usable', v4(i.last)),
      row('Usable hosts', fmtNum(i.usable)),
      row('Addresses in total', fmtNum(i.total)),
      `<div class="sn-label">Type</div><div class="sn-value" data-copy="${esc(kind)}"><span class="sn-tag">${esc(kind)}</span></div>`,
      row('Address as a number', String(i.ip >>> 0), 'decimal'),
      row('Hex', '0x' + (i.ip >>> 0).toString(16).toUpperCase().padStart(8, '0')),
    ].join('');
    const bits = (n) => (n >>> 0).toString(2).padStart(32, '0');
    const colour = (b) => b.split('').map((c, k) => `<span class="${k < i.prefix ? 'n' : 'h'}">${c}</span>${k % 8 === 7 && k < 31 ? '.' : ''}`).join('');
    bin.innerHTML = `<div><span class="k">Address</span>${colour(bits(i.ip))}</div><div><span class="k">Netmask</span>${colour(bits(i.mask))}</div><div><span class="k"></span><span class="n">network bits</span> · <span class="h">host bits</span></div>`;
    renderContains();
  }

  let v4Current = null;

  function renderContains() {
    const el = $('sn-v4-contains-result');
    const ip = parseV4($('sn-v4-contains').value);
    el.className = 'sn-answer';
    if (!$('sn-v4-contains').value.trim()) { el.textContent = ''; return; }
    if (ip === null) { el.textContent = 'Not a valid IPv4 address'; el.classList.add('no'); return; }
    if (!v4Current) { el.textContent = 'Enter a network above first'; return; }
    const inside = ((ip & v4Current.mask) >>> 0) === v4Current.network;
    el.textContent = inside ? `Yes, ${v4(ip)} is inside ${v4(v4Current.network)}/${v4Current.prefix}` : `No, ${v4(ip)} is outside ${v4(v4Current.network)}/${v4Current.prefix}`;
    el.classList.add(inside ? 'yes' : 'no');
  }

  // ---------------- Split ----------------
  function renderSplit() {
    const err = $('sn-split-error');
    const sum = $('sn-split-summary');
    const table = $('sn-split-table');
    err.hidden = true;
    sum.textContent = '';
    table.innerHTML = '';
    const by = $('sn-split-by').value;
    $('sn-split-unit').textContent = { count: 'parts', prefix: '(prefix, e.g. 26 or /26)', hosts: 'hosts each' }[by];
    const parsed = parseV4Net($('sn-split-net').value);
    if (parsed.empty) return;
    const fail = (m) => { err.textContent = m; err.hidden = false; };
    if (parsed.error) return fail(parsed.error);
    const base = v4Info(parsed.ip, parsed.prefix);
    const raw = $('sn-split-value').value.trim().replace(/^\//, '');
    const val = Number(raw);
    if (!raw || !Number.isInteger(val) || val < 1) return fail('Enter a whole number');
    let newPrefix;
    if (by === 'count') {
      newPrefix = base.prefix + Math.ceil(Math.log2(val));
    } else if (by === 'prefix') {
      newPrefix = val;
    } else {
      // Smallest network that holds this many usable hosts
      newPrefix = 32;
      while (newPrefix > 0 && v4Info(0, newPrefix).usable < val) newPrefix--;
    }
    if (newPrefix > 32) return fail('That is more parts than the network has addresses');
    if (newPrefix < base.prefix) return fail(`The new networks must be smaller than /${base.prefix}`);
    const count = 2 ** (newPrefix - base.prefix);
    const each = v4Info(0, newPrefix);
    sum.textContent = `${fmtNum(count)} networks of /${newPrefix} (${fmtNum(each.usable)} usable hosts each)` + (by === 'count' && count !== val ? `; ${val} rounded up to ${count}, since subnets come in powers of two` : '');
    const shown = Math.min(count, 1024);
    const rows = [];
    for (let k = 0; k < shown; k++) {
      const net = (base.network + k * each.total) >>> 0;
      const i = v4Info(net, newPrefix);
      rows.push(`<tr><td data-copy="${v4(net)}/${newPrefix}">${v4(net)}/${newPrefix}</td><td data-copy="${v4(i.first)} - ${v4(i.last)}">${v4(i.first)} - ${v4(i.last)}</td><td data-copy="${v4(i.broadcast)}">${newPrefix >= 31 ? '—' : v4(i.broadcast)}</td></tr>`);
    }
    table.innerHTML = `<thead><tr><th>Network</th><th>Usable range</th><th>Broadcast</th></tr></thead><tbody>${rows.join('')}${count > shown ? `<tr><td colspan="3">… and ${fmtNum(count - shown)} more</td></tr>` : ''}</tbody>`;
    store.set('splitNet', $('sn-split-net').value);
  }

  // ---------------- Combine ----------------
  // Exact cover: the fewest networks that contain exactly the given ones
  function rangesToCidrs(start, end) {
    const out = [];
    let s = start;
    while (s <= end) {
      let p = 32;
      while (p > 0) {
        const bigger = p - 1;
        const size = 2 ** (32 - bigger);
        if (s % size !== 0 || s + size - 1 > end) break;
        p = bigger;
      }
      out.push([s, p]);
      s += 2 ** (32 - p);
    }
    return out;
  }

  function renderCombine() {
    const err = $('sn-combine-error');
    const out = $('sn-combine-result');
    err.hidden = true;
    out.innerHTML = '';
    const lines = $('sn-combine-input').value.split(/[\n,]+/).map((l) => l.trim()).filter(Boolean);
    if (!lines.length) return;
    const ranges = [];
    for (const l of lines) {
      const p = parseV4Net(l);
      if (p.error) { err.textContent = `${l}: ${p.error}`; err.hidden = false; return; }
      const i = v4Info(p.ip, p.prefix);
      ranges.push([i.network, i.broadcast]);
    }
    ranges.sort((a, b) => a[0] - b[0]);
    const merged = [];
    for (const r of ranges) {
      const last = merged[merged.length - 1];
      if (last && r[0] <= last[1] + 1) last[1] = Math.max(last[1], r[1]);
      else merged.push([r[0], r[1]]);
    }
    const exact = merged.flatMap(([s, e]) => rangesToCidrs(s, e));
    // The single smallest network covering everything (may include extra addresses)
    const lo = merged[0][0];
    const hi = merged[merged.length - 1][1];
    let sp = 32;
    while (sp > 0 && ((lo & maskOf(sp)) >>> 0) !== ((hi & maskOf(sp)) >>> 0)) sp--;
    const superNet = (lo & maskOf(sp)) >>> 0;
    const covered = merged.reduce((a, [s, e]) => a + (e - s + 1), 0);
    const superSize = 2 ** (32 - sp);
    out.innerHTML = `
      <div class="sn-card">
        <div class="sn-card-title">Fewest networks, exactly the same addresses (${exact.length})</div>
        <table class="sn-table"><tbody>${exact.map(([n, p]) => `<tr><td data-copy="${v4(n)}/${p}">${v4(n)}/${p}</td></tr>`).join('')}</tbody></table>
      </div>
      <div class="sn-card">
        <div class="sn-card-title">One network covering all of them</div>
        <table class="sn-table"><tbody><tr><td data-copy="${v4(superNet)}/${sp}">${v4(superNet)}/${sp}</td></tr></tbody></table>
        <div class="sn-hint">${superSize === covered ? 'Exactly the same addresses.' : `Includes ${fmtNum(superSize - covered)} addresses that were not in the list.`}</div>
      </div>`;
    store.set('combine', $('sn-combine-input').value.slice(0, 4000));
  }

  // ---------------- IPv6 ----------------
  function parseV6(s) {
    let t = (s || '').trim().toLowerCase();
    if (!t) return null;
    // Embedded IPv4 at the end (::ffff:192.0.2.1)
    const v4m = /(\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3})$/.exec(t);
    if (v4m) {
      const n = parseV4(v4m[1]);
      if (n === null) return null;
      t = t.slice(0, -v4m[1].length) + (n >>> 16).toString(16) + ':' + (n & 0xFFFF).toString(16);
    }
    const halves = t.split('::');
    if (halves.length > 2) return null;
    const part = (h) => (h ? h.split(':') : []);
    const head = part(halves[0]);
    const tail = halves.length === 2 ? part(halves[1]) : [];
    const missing = 8 - head.length - tail.length;
    if ((halves.length === 1 && head.length !== 8) || missing < 0 || (halves.length === 2 && missing < 1)) return null;
    const groups = [...head, ...Array(halves.length === 2 ? missing : 0).fill('0'), ...tail];
    let n = 0n;
    for (const g of groups) {
      if (!/^[0-9a-f]{1,4}$/.test(g)) return null;
      n = (n << 16n) + BigInt(parseInt(g, 16));
    }
    return n;
  }

  const MAX128 = (1n << 128n) - 1n;
  const mask6 = (p) => (p === 0 ? 0n : (MAX128 << BigInt(128 - p)) & MAX128);
  const groupsOf = (n) => Array.from({ length: 8 }, (_, k) => Number((n >> BigInt(112 - k * 16)) & 0xFFFFn));
  const expand6 = (n) => groupsOf(n).map((g) => g.toString(16).padStart(4, '0')).join(':');

  // Shortest form (RFC 5952): longest run of zero groups becomes ::
  function compress6(n) {
    const g = groupsOf(n);
    let best = [-1, 0];
    for (let k = 0; k < 8;) {
      if (g[k] !== 0) { k++; continue; }
      let e = k;
      while (e < 8 && g[e] === 0) e++;
      if (e - k > best[1] && e - k > 1) best = [k, e - k];
      k = e;
    }
    const hex = g.map((x) => x.toString(16));
    if (best[0] < 0) return hex.join(':');
    return `${hex.slice(0, best[0]).join(':')}::${hex.slice(best[0] + best[1]).join(':')}`;
  }

  const V6_RANGES = [
    ['::1/128', 'Loopback', 'This computer'],
    ['::/128', 'Unspecified', ''],
    ['::ffff:0:0/96', 'IPv4-mapped', 'An IPv4 address written as IPv6'],
    ['64:ff9b::/96', 'NAT64', 'IPv4 reached through NAT64'],
    ['2000::/3', 'Global unicast', 'Public internet addresses'],
    ['2001:db8::/32', 'Documentation', 'For examples'],
    ['fc00::/7', 'Unique local (ULA)', 'Private networks, like RFC 1918 for IPv4 (in practice fd00::/8)'],
    ['fe80::/10', 'Link-local', 'Every interface has one; not routed'],
    ['ff00::/8', 'Multicast', ''],
  ];

  function v6Kind(n) {
    let best = null;
    for (const [cidr, name] of V6_RANGES) {
      const [a, p] = cidr.split('/');
      const net = parseV6(a);
      const pr = Number(p);
      if ((n & mask6(pr)) === net && (!best || pr > best.prefix)) best = { prefix: pr, name };
    }
    return best ? best.name : 'Reserved / other';
  }

  function renderV6() {
    const raw = $('sn-v6-input').value.trim();
    const err = $('sn-v6-error');
    const out = $('sn-v6-results');
    err.hidden = true;
    out.innerHTML = '';
    if (!raw) return;
    const [a, p] = raw.split('/');
    const n = parseV6(a);
    if (n === null) { err.textContent = `${a} is not a valid IPv6 address`; err.hidden = false; return; }
    const prefix = p === undefined ? 128 : Number(p);
    if (!Number.isInteger(prefix) || prefix < 0 || prefix > 128) { err.textContent = 'The prefix must be between 0 and 128'; err.hidden = false; return; }
    const m = mask6(prefix);
    const net = n & m;
    const last = net | (~m & MAX128);
    const hostBits = 128 - prefix;
    const total = 1n << BigInt(hostBits);
    const kind = v6Kind(net);
    out.innerHTML = [
      row('Network', `${compress6(net)}/${prefix}`),
      row('Address (short)', compress6(n)),
      row('Address (full)', expand6(n)),
      row('First address', compress6(net)),
      row('Last address', compress6(last)),
      row('Addresses', hostBits <= 64 ? fmtNum(total) : `2^${hostBits}`, hostBits > 64 ? `≈ ${Number(total).toExponential(2)}` : ''),
      row('Number of /64 networks', prefix <= 64 ? fmtNum(1n << BigInt(64 - prefix)) : 'less than one /64', prefix === 64 ? 'one LAN' : (prefix < 64 ? 'a /64 is one LAN' : '')),
      row('Prefix in netmask form', compress6(m)),
      `<div class="sn-label">Type</div><div class="sn-value" data-copy="${esc(kind)}"><span class="sn-tag">${esc(kind)}</span></div>`,
      row('Reverse DNS zone', reverseZone(net, prefix)),
    ].join('');
  }

  // ip6.arpa name for the network, to the nearest nibble
  function reverseZone(net, prefix) {
    const nibbles = Math.ceil(prefix / 4);
    const hex = expand6(net).replace(/:/g, '').slice(0, nibbles);
    return (hex.split('').reverse().join('.') + (hex ? '.' : '') + 'ip6.arpa');
  }

  // ---------------- Reference ----------------
  function renderReference() {
    const item = (cidr, name, note, v) => `<button type="button" class="sn-ref" data-open="${esc(cidr)}" data-v="${v}"><b>${esc(cidr)}</b><span>${esc(name)}</span>${note ? `<small>${esc(note)}</small>` : ''}</button>`;
    $('sn-ref-list').innerHTML = `<div class="sn-ref-group">IPv4</div>${V4_RANGES.map(([c, n, d]) => item(c, n, d, 4)).join('')}<div class="sn-ref-group">IPv6</div>${V6_RANGES.map(([c, n, d]) => item(c, n, d, 6)).join('')}`;
  }

  // ---------------- Recent ----------------
  function remember(kind, value) {
    const v = value.trim();
    if (!v) return;
    let list = [];
    try { list = JSON.parse(store.get('recent' + kind) || '[]'); } catch (_) { list = []; }
    list = [v, ...list.filter((x) => x !== v)].slice(0, 6);
    store.set('recent' + kind, JSON.stringify(list));
    renderRecent(kind);
  }

  function renderRecent(kind) {
    let list = [];
    try { list = JSON.parse(store.get('recent' + kind) || '[]'); } catch (_) { list = []; }
    $(`sn-${kind}-recent`).innerHTML = list.map((v) => `<button type="button" class="sn-chip" data-recent="${esc(v)}" data-kind="${kind}">${esc(v)}</button>`).join('');
  }

  // ---------------- Tabs and wiring ----------------
  function showTab(tab) {
    const t = ['v4', 'split', 'v6', 'ref'].includes(tab) ? tab : 'v4';
    document.querySelectorAll('.sn-tab').forEach((b) => b.classList.toggle('active', b.dataset.tab === t));
    document.querySelectorAll('.sn-panel').forEach((p) => { p.hidden = p.dataset.panel !== t; });
    store.set('tab', t);
  }

  function showSplitMode(mode) {
    const m = mode === 'combine' ? 'combine' : 'split';
    document.querySelectorAll('.sn-seg-btn').forEach((b) => b.classList.toggle('active', b.dataset.split === m));
    document.querySelectorAll('.sn-split-mode').forEach((p) => { p.hidden = p.dataset.splitPanel !== m; });
    store.set('splitMode', m);
  }

  // Wait until typing pauses before remembering an entry
  function debounce(fn, ms) {
    let t;
    return (...a) => { clearTimeout(t); t = setTimeout(() => fn(...a), ms); };
  }

  let started = false;
  function start() {
    if (started) return;
    started = true;
    document.querySelectorAll('.sn-tab').forEach((b) => b.addEventListener('click', () => showTab(b.dataset.tab)));
    document.querySelectorAll('.sn-seg-btn').forEach((b) => b.addEventListener('click', () => showSplitMode(b.dataset.split)));
    const rememberV4 = debounce(() => { if (v4Current) remember('v4', $('sn-v4-input').value); }, 1200);
    const rememberV6 = debounce(() => { if ($('sn-v6-results').innerHTML) remember('v6', $('sn-v6-input').value); }, 1200);
    $('sn-v4-input').addEventListener('input', () => { renderV4(); store.set('v4', $('sn-v4-input').value); rememberV4(); });
    $('sn-v4-contains').addEventListener('input', renderContains);
    $('sn-v6-input').addEventListener('input', () => { renderV6(); store.set('v6', $('sn-v6-input').value); rememberV6(); });
    ['sn-split-net', 'sn-split-value'].forEach((id) => $(id).addEventListener('input', renderSplit));
    $('sn-split-by').addEventListener('change', renderSplit);
    $('sn-combine-input').addEventListener('input', renderCombine);
    // One click copies any result
    document.addEventListener('click', (e) => {
      const c = e.target.closest('[data-copy]');
      if (c && c.dataset.copy && c.dataset.copy !== '—') { copy(c.dataset.copy); return; }
      const r = e.target.closest('[data-recent]');
      if (r) {
        const input = $(`sn-${r.dataset.kind}-input`);
        input.value = r.dataset.recent;
        input.dispatchEvent(new Event('input'));
        return;
      }
      const o = e.target.closest('[data-open]');
      if (o) {
        const isV6 = o.dataset.v === '6';
        const input = $(isV6 ? 'sn-v6-input' : 'sn-v4-input');
        input.value = o.dataset.open;
        input.dispatchEvent(new Event('input'));
        showTab(isV6 ? 'v6' : 'v4');
      }
    });
    $('sn-v4-input').value = store.get('v4') || '192.168.1.10/24';
    $('sn-v6-input').value = store.get('v6') || '';
    $('sn-split-net').value = store.get('splitNet') || '';
    $('sn-combine-input').value = store.get('combine') || '';
    renderReference();
    renderRecent('v4');
    renderRecent('v6');
    renderV4();
    renderV6();
    renderSplit();
    renderCombine();
    showSplitMode(store.get('splitMode') || 'split');
    showTab(store.get('tab') || 'v4');
  }

  // Opened outside Brum (no bridge): start anyway
  setTimeout(() => { if (!readyDone) start(); }, 600);

  // Test hook
  window.SubnetCalc = { parseV4Net, v4Info, v4, parseV6, compress6, expand6, rangesToCidrs, v6Kind, v4Kind };
})();
