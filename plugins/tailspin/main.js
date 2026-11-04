// ==========================================================================
// Tailspin: decision wheel, yes or no, and dice (D&D and Yatzy)
// ==========================================================================
(function () {
  'use strict';

  // ---------------- Bridge to Brum ----------------
  // Chewtoys run sandboxed: saved lists and settings are kept by Brum.
  let hostReqId = 0;
  const pending = new Map();
  let saved = {};
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
      readyDone = true;
      start();
    }
  });

  function host(action, payload) {
    return new Promise((resolve, reject) => {
      const reqId = 'tailspin-' + (++hostReqId);
      pending.set(reqId, { resolve, reject });
      window.parent.postMessage({ type: 'BRUM_REQ', reqId, action, payload }, '*');
    });
  }

  const store = {
    get: (key) => (Object.prototype.hasOwnProperty.call(saved, key) ? saved[key] : null),
    set: (key, value) => { saved[key] = String(value); host('storage.set', { key, value: String(value) }).catch(() => {}); },
  };

  // ---------------- Fair randomness ----------------
  // The browser's secure generator, with rejection so every outcome is equally likely
  function randInt(n) {
    if (!(n >= 1)) return 0;
    const limit = Math.floor(0x100000000 / n) * n;
    const buf = new Uint32Array(1);
    for (;;) {
      crypto.getRandomValues(buf);
      if (buf[0] < limit) return buf[0] % n;
    }
  }
  const randFloat = () => randInt(1 << 30) / (1 << 30);
  const rollDie = (sides) => randInt(sides) + 1;

  const reducedMotion = () => window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches;

  // ---------------- Sound (off unless chosen) ----------------
  let soundOn = false;
  let audioCtx = null;
  let lastTick = 0;

  function blip(freq, duration, volume, type) {
    if (!soundOn) return;
    try {
      audioCtx = audioCtx || new (window.AudioContext || window.webkitAudioContext)();
      if (audioCtx.state === 'suspended') audioCtx.resume();
      const t = audioCtx.currentTime;
      const osc = audioCtx.createOscillator();
      const gain = audioCtx.createGain();
      osc.type = type || 'triangle';
      osc.frequency.setValueAtTime(freq, t);
      gain.gain.setValueAtTime(volume, t);
      gain.gain.exponentialRampToValueAtTime(0.0001, t + duration);
      osc.connect(gain).connect(audioCtx.destination);
      osc.start(t);
      osc.stop(t + duration + 0.02);
    } catch (_) { /* no audio available */ }
  }
  const tick = () => {
    const now = performance.now();
    if (now - lastTick < 35) return;
    lastTick = now;
    blip(1400, 0.025, 0.05, 'square');
  };
  const diceSound = () => { blip(520, 0.05, 0.08); setTimeout(() => blip(380, 0.06, 0.06), 45); };
  const doneSound = () => blip(880, 0.12, 0.06, 'sine');

  // ---------------- Elements ----------------
  const $ = (id) => document.getElementById(id);
  const els = {
    sound: $('ts-sound'),
    canvas: $('ts-wheel'),
    wheelBox: $('ts-wheel-box'),
    wheelResult: $('ts-wheel-result'),
    spin: $('ts-spin'),
    listSelect: $('ts-list-select'),
    listNew: $('ts-list-new'),
    listDelete: $('ts-list-delete'),
    listName: $('ts-list-name'),
    entries: $('ts-entries'),
    entryCount: $('ts-entry-count'),
    removeWinner: $('ts-remove-winner'),
    yesno: $('ts-yesno'),
    yesnoResult: $('ts-yesno-result'),
    dieButtons: $('ts-die-buttons'),
    notation: $('ts-notation'),
    rollNotation: $('ts-roll-notation'),
    advantage: $('ts-advantage'),
    disadvantage: $('ts-disadvantage'),
    rpgResult: $('ts-rpg-result'),
    rpgHistory: $('ts-rpg-history'),
    yatzyDice: $('ts-yatzy-dice'),
    yatzyResult: $('ts-yatzy-result'),
    yatzyRoll: $('ts-yatzy-roll'),
  };

  // ---------------- Tabs ----------------
  let activeTab = 'wheel';

  function showTab(tab) {
    activeTab = ['wheel', 'yesno', 'dice'].includes(tab) ? tab : 'wheel';
    document.querySelectorAll('.ts-tab').forEach((b) => b.classList.toggle('active', b.dataset.tab === activeTab));
    document.querySelectorAll('.ts-panel').forEach((p) => { p.hidden = p.dataset.panel !== activeTab; });
    store.set('tab', activeTab);
    if (activeTab === 'wheel') requestAnimationFrame(resizeWheel);
  }

  document.querySelectorAll('.ts-tab').forEach((b) => b.addEventListener('click', () => showTab(b.dataset.tab)));

  function setSound(on) {
    soundOn = !!on;
    els.sound.classList.toggle('on', soundOn);
    els.sound.setAttribute('aria-pressed', String(soundOn));
    els.sound.title = soundOn ? 'Sound on' : 'Sound off';
    store.set('sound', soundOn ? '1' : '0');
  }
  els.sound.addEventListener('click', () => { setSound(!soundOn); if (soundOn) tick(); });

  // ---------------- Wheel lists ----------------
  const MAX_ENTRIES = 500;
  const MAX_ENTRY_LEN = 80;
  const DEFAULT_LISTS = [
    { name: 'Lunch', entries: ['Pizza', 'Sushi', 'Burger', 'Salad', 'Tacos', 'Noodles'] },
    { name: 'Yes / Maybe / No', entries: ['Yes', 'Maybe', 'No'] },
  ];
  let lists = [];
  let currentList = 0;

  function loadLists() {
    try {
      const data = JSON.parse(store.get('lists') || 'null');
      if (data && Array.isArray(data.lists) && data.lists.length) {
        lists = data.lists
          .filter((l) => l && typeof l.name === 'string' && Array.isArray(l.entries))
          .map((l) => ({ name: l.name.slice(0, 40), entries: l.entries.map(String).slice(0, MAX_ENTRIES) }));
        currentList = Math.min(Math.max(0, data.current | 0), lists.length - 1);
      }
    } catch (_) { /* start fresh */ }
    if (!lists.length) {
      lists = DEFAULT_LISTS.map((l) => ({ name: l.name, entries: l.entries.slice() }));
      currentList = 0;
    }
  }

  function saveLists() {
    store.set('lists', JSON.stringify({ current: currentList, lists }));
  }

  function parseEntries(text) {
    return text.split('\n')
      .map((s) => s.trim().slice(0, MAX_ENTRY_LEN))
      .filter(Boolean)
      .slice(0, MAX_ENTRIES);
  }

  const list = () => lists[currentList];

  function renderListControls() {
    els.listSelect.innerHTML = '';
    lists.forEach((l, i) => {
      const opt = document.createElement('option');
      opt.value = String(i);
      opt.textContent = l.name || 'Untitled';
      els.listSelect.appendChild(opt);
    });
    els.listSelect.value = String(currentList);
    els.listName.value = list().name;
    els.entries.value = list().entries.join('\n');
    els.listDelete.disabled = lists.length < 2;
    updateCount();
  }

  function updateCount() {
    if (!lists.length) return;
    const n = list().entries.length;
    const raw = els.entries.value.split('\n').filter((s) => s.trim()).length;
    els.entryCount.textContent = raw > MAX_ENTRIES
      ? `${MAX_ENTRIES} entries (only the first ${MAX_ENTRIES} are used)`
      : `${n} ${n === 1 ? 'entry' : 'entries'}`;
    els.spin.disabled = spinning || n < 1;
  }

  let saveTimer = null;
  els.entries.addEventListener('input', () => {
    list().entries = parseEntries(els.entries.value);
    updateCount();
    drawWheel();
    clearTimeout(saveTimer);
    saveTimer = setTimeout(saveLists, 400);
  });

  els.listName.addEventListener('input', () => {
    list().name = els.listName.value.slice(0, 40);
    const opt = els.listSelect.options[currentList];
    if (opt) opt.textContent = list().name || 'Untitled';
    clearTimeout(saveTimer);
    saveTimer = setTimeout(saveLists, 400);
  });

  els.listSelect.addEventListener('change', () => {
    currentList = Math.min(Math.max(0, parseInt(els.listSelect.value, 10) || 0), lists.length - 1);
    rotation = 0;
    els.wheelResult.innerHTML = '&nbsp;';
    renderListControls();
    drawWheel();
    saveLists();
  });

  els.listNew.addEventListener('click', () => {
    let n = lists.length + 1;
    while (lists.some((l) => l.name === `List ${n}`)) n++;
    lists.push({ name: `List ${n}`, entries: [] });
    currentList = lists.length - 1;
    rotation = 0;
    els.wheelResult.innerHTML = '&nbsp;';
    renderListControls();
    drawWheel();
    saveLists();
    els.entries.focus();
  });

  // Deleting asks once: a second click within 3 seconds confirms
  let deleteArmed = null;
  els.listDelete.addEventListener('click', () => {
    if (lists.length < 2) return;
    if (!deleteArmed) {
      els.listDelete.title = `Click again to delete "${list().name}"`;
      els.listDelete.classList.add('armed');
      els.listDelete.style.color = '#ef4444';
      deleteArmed = setTimeout(() => {
        deleteArmed = null;
        els.listDelete.title = 'Delete this list';
        els.listDelete.style.color = '';
      }, 3000);
      return;
    }
    clearTimeout(deleteArmed);
    deleteArmed = null;
    els.listDelete.title = 'Delete this list';
    els.listDelete.style.color = '';
    lists.splice(currentList, 1);
    currentList = Math.max(0, currentList - 1);
    rotation = 0;
    els.wheelResult.innerHTML = '&nbsp;';
    renderListControls();
    drawWheel();
    saveLists();
  });

  els.removeWinner.addEventListener('change', () => store.set('removeWinner', els.removeWinner.checked ? '1' : '0'));

  // ---------------- Wheel drawing ----------------
  const COLOURS = ['#f59e0b', '#3b82f6', '#22c55e', '#ef4444', '#a855f7', '#14b8a6', '#ec4899', '#84cc16', '#f97316', '#6366f1', '#06b6d4', '#eab308'];
  let rotation = 0; // radians; slice 0 starts at the top and runs clockwise
  let wheelSize = 0;

  function sliceColour(i, n) {
    let c = COLOURS[i % COLOURS.length];
    // Never two equal colours side by side where the wheel closes
    if (i === n - 1 && n > 1 && n % COLOURS.length === 1) c = COLOURS[(i + 3) % COLOURS.length];
    return c;
  }

  function resizeWheel() {
    const box = els.wheelBox;
    const size = Math.max(80, Math.floor(Math.min(box.clientWidth, box.clientHeight)) - 6);
    if (!box.clientWidth || !box.clientHeight) return;
    const dpr = window.devicePixelRatio || 1;
    wheelSize = size;
    els.canvas.style.width = size + 'px';
    els.canvas.style.height = size + 'px';
    els.canvas.width = Math.round(size * dpr);
    els.canvas.height = Math.round(size * dpr);
    drawWheel();
  }

  function drawWheel() {
    if (!lists.length) return; // not started yet
    const canvas = els.canvas;
    const ctx = canvas.getContext('2d');
    if (!ctx || !wheelSize) return;
    const dpr = window.devicePixelRatio || 1;
    const size = wheelSize;
    const r = size / 2 - 3;
    const cx = size / 2;
    const cy = size / 2;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, size, size);

    const entries = list().entries;
    const n = entries.length;
    if (!n) {
      ctx.beginPath();
      ctx.arc(cx, cy, r, 0, Math.PI * 2);
      ctx.fillStyle = '#0d0e12';
      ctx.fill();
      ctx.strokeStyle = 'rgba(255,255,255,0.15)';
      ctx.lineWidth = 2;
      ctx.stroke();
      ctx.fillStyle = '#71717a';
      ctx.font = '600 13px ' + getComputedStyle(document.body).fontFamily;
      ctx.textAlign = 'center';
      ctx.textBaseline = 'middle';
      ctx.fillText('Add entries to spin', cx, cy);
      return;
    }

    const step = (Math.PI * 2) / n;
    // Canvas angle 0 is to the right; the top is -PI/2
    const base = -Math.PI / 2 + rotation;
    for (let i = 0; i < n; i++) {
      const a0 = base + i * step;
      ctx.beginPath();
      ctx.moveTo(cx, cy);
      ctx.arc(cx, cy, r, a0, a0 + step);
      ctx.closePath();
      ctx.fillStyle = sliceColour(i, n);
      ctx.fill();
      if (n > 1 && n <= 120) {
        ctx.strokeStyle = 'rgba(24,24,27,0.55)';
        ctx.lineWidth = 1;
        ctx.stroke();
      }
    }

    // Labels where there is room for them
    const arcLen = step * r * 0.62;
    const fontPx = Math.max(9, Math.min(15, arcLen * 0.62, size / 22));
    if (arcLen >= 9) {
      ctx.font = `600 ${fontPx}px ` + getComputedStyle(document.body).fontFamily;
      ctx.textAlign = 'right';
      ctx.textBaseline = 'middle';
      const maxW = r * 0.78;
      for (let i = 0; i < n; i++) {
        const mid = base + (i + 0.5) * step;
        ctx.save();
        ctx.translate(cx, cy);
        // Labels on the left half are turned the other way so they never read upside down
        const leftHalf = Math.cos(mid) < 0;
        ctx.rotate(leftHalf ? mid + Math.PI : mid);
        ctx.textAlign = leftHalf ? 'left' : 'right';
        let label = entries[i];
        if (ctx.measureText(label).width > maxW) {
          while (label.length > 1 && ctx.measureText(label + '…').width > maxW) label = label.slice(0, -1);
          label += '…';
        }
        ctx.fillStyle = '#18181b';
        ctx.fillText(label, leftHalf ? -(r - 10) : r - 10, 0);
        ctx.restore();
      }
    }

    // Hub
    ctx.beginPath();
    ctx.arc(cx, cy, Math.max(8, r * 0.08), 0, Math.PI * 2);
    ctx.fillStyle = '#18181b';
    ctx.fill();
    ctx.strokeStyle = '#f4f4f5';
    ctx.lineWidth = 2;
    ctx.stroke();
  }

  if (window.ResizeObserver) new ResizeObserver(() => resizeWheel()).observe(els.wheelBox);
  window.addEventListener('resize', resizeWheel);

  // ---------------- Spinning ----------------
  let spinning = false;

  // The slice under the pointer for a given rotation
  function sliceAt(rot, n) {
    const step = (Math.PI * 2) / n;
    const a = (((-rot) % (Math.PI * 2)) + Math.PI * 2) % (Math.PI * 2);
    return Math.floor(a / step) % n;
  }

  function spin() {
    if (spinning || !lists.length) return;
    const entries = list().entries;
    const n = entries.length;
    if (!n) return;
    spinning = true;
    els.spin.disabled = true;
    els.entries.disabled = true;
    els.wheelResult.innerHTML = '&nbsp;';

    // Pick the winner first, then land anywhere inside its slice
    const winner = randInt(n);
    const step = (Math.PI * 2) / n;
    const inside = (0.15 + 0.7 * randFloat()) * step;
    const targetAngle = winner * step + inside; // where on the wheel the pointer should end up
    const current = (((-rotation) % (Math.PI * 2)) + Math.PI * 2) % (Math.PI * 2);
    // Clockwise: the wheel turns so the pointer moves back through the slices
    let delta = current - targetAngle;
    if (delta < 0) delta += Math.PI * 2;
    const quick = reducedMotion();
    const turns = quick ? 0 : 4 + randInt(3);
    const total = delta + turns * Math.PI * 2;
    const duration = quick ? 450 : 3600 + randInt(900);
    const startRot = rotation;
    const t0 = performance.now();
    let lastSlice = sliceAt(rotation, n);

    function frame(now) {
      const t = Math.min(1, (now - t0) / duration);
      const eased = 1 - Math.pow(1 - t, 4);
      rotation = startRot + total * eased;
      const s = sliceAt(rotation, n);
      if (s !== lastSlice) { lastSlice = s; tick(); }
      drawWheel();
      if (t < 1) {
        requestAnimationFrame(frame);
      } else {
        rotation = rotation % (Math.PI * 2);
        finish(entries[winner]);
      }
    }
    requestAnimationFrame(frame);
  }

  function finish(name) {
    spinning = false;
    els.wheelResult.textContent = name;
    els.wheelResult.title = name;
    doneSound();
    if (els.removeWinner.checked) {
      const idx = list().entries.indexOf(name);
      if (idx >= 0) list().entries.splice(idx, 1);
      els.entries.value = list().entries.join('\n');
      saveLists();
      drawWheel();
    }
    els.entries.disabled = false;
    updateCount();
  }

  els.spin.addEventListener('click', spin);
  els.canvas.addEventListener('click', spin);

  // ---------------- Yes / No ----------------
  function askYesNo() {
    const yes = randInt(2) === 1;
    els.yesnoResult.textContent = yes ? 'Yes' : 'No';
    els.yesnoResult.className = 'ts-big-answer ' + (yes ? 'yes' : 'no');
    doneSound();
  }
  els.yesno.addEventListener('click', askYesNo);

  // ---------------- Dice: D&D ----------------
  const RPG_DICE = [4, 6, 8, 10, 12, 20, 100];
  const MAX_DICE = 100;
  const history = [];

  RPG_DICE.forEach((sides) => {
    const b = document.createElement('button');
    b.type = 'button';
    b.className = 'ts-btn';
    b.textContent = 'd' + sides;
    b.title = 'Roll one d' + sides;
    b.addEventListener('click', () => rollNotation('1d' + sides));
    els.dieButtons.appendChild(b);
  });

  // "2d6+3", "d20", "1d8 + 1d6 + 2", "4d6-1"
  function parseNotation(text) {
    const clean = String(text || '').toLowerCase().replace(/\s+/g, '');
    if (!clean || !/^[+-]?(\d*d\d+|\d+)([+-](\d*d\d+|\d+))*$/.test(clean)) return null;
    const terms = [];
    let diceCount = 0;
    for (const m of clean.matchAll(/([+-]?)(\d*d\d+|\d+)/g)) {
      const sign = m[1] === '-' ? -1 : 1;
      if (m[2].includes('d')) {
        const [c, s] = m[2].split('d');
        const count = c === '' ? 1 : parseInt(c, 10);
        const sides = parseInt(s, 10);
        if (count < 1 || sides < 2 || sides > 1000) return null;
        diceCount += count;
        terms.push({ sign, count, sides });
      } else {
        const value = parseInt(m[2], 10);
        if (value > 100000) return null;
        terms.push({ sign, value });
      }
    }
    if (diceCount < 1 || diceCount > MAX_DICE) return null;
    return terms;
  }

  function showRpg(label, detail, total) {
    els.rpgResult.innerHTML = '';
    const head = document.createElement('div');
    head.className = 'ts-dim';
    head.textContent = label + (detail ? ': ' + detail : '');
    const tot = document.createElement('div');
    tot.className = 'ts-total';
    tot.textContent = String(total);
    els.rpgResult.append(head, tot);
    history.unshift(`${label} = ${total}${detail ? '  (' + detail + ')' : ''}`);
    history.length = Math.min(history.length, 30);
    els.rpgHistory.innerHTML = '';
    history.slice(1).forEach((h) => {
      const li = document.createElement('li');
      li.textContent = h;
      els.rpgHistory.appendChild(li);
    });
    diceSound();
  }

  function rollNotation(text) {
    const terms = parseNotation(text);
    if (!terms) {
      els.rpgResult.innerHTML = '';
      const err = document.createElement('div');
      err.className = 'ts-dim';
      err.textContent = `Try something like 2d6+3, d20 or 1d8+1d6 (up to ${MAX_DICE} dice)`;
      els.rpgResult.appendChild(err);
      return;
    }
    let total = 0;
    const parts = [];
    for (const t of terms) {
      if (t.sides) {
        const rolls = Array.from({ length: t.count }, () => rollDie(t.sides));
        const sum = rolls.reduce((a, b) => a + b, 0);
        total += t.sign * sum;
        parts.push((t.sign < 0 ? '−' : '') + '[' + rolls.join(', ') + ']');
      } else {
        total += t.sign * t.value;
        parts.push((t.sign < 0 ? '− ' : '+ ') + t.value);
      }
    }
    const label = String(text).replace(/\s+/g, '');
    showRpg(label, parts.join(' '), total);
  }

  function rollD20Twice(keepHigher) {
    const a = rollDie(20);
    const b = rollDie(20);
    const kept = keepHigher ? Math.max(a, b) : Math.min(a, b);
    showRpg(keepHigher ? 'd20 advantage' : 'd20 disadvantage', `${a} and ${b}`, kept);
  }

  els.rollNotation.addEventListener('click', () => {
    store.set('notation', els.notation.value.slice(0, 60));
    rollNotation(els.notation.value);
  });
  els.notation.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') { e.preventDefault(); els.rollNotation.click(); }
  });
  els.advantage.addEventListener('click', () => rollD20Twice(true));
  els.disadvantage.addEventListener('click', () => rollD20Twice(false));

  // ---------------- Dice: Yatzy ----------------
  // Dot positions on a 100 x 100 die face
  const PIP_XY = { tl: [27, 27], tr: [73, 27], ml: [27, 50], c: [50, 50], mr: [73, 50], bl: [27, 73], br: [73, 73] };
  const PIPS = { 1: ['c'], 2: ['tr', 'bl'], 3: ['tr', 'c', 'bl'], 4: ['tl', 'tr', 'bl', 'br'], 5: ['tl', 'tr', 'c', 'bl', 'br'], 6: ['tl', 'tr', 'ml', 'mr', 'bl', 'br'] };
  const SVG_NS = 'http://www.w3.org/2000/svg';
  const yatzy = { values: [0, 0, 0, 0, 0], held: [false, false, false, false, false], rolls: 0 };

  function dieFace(value) {
    const svg = document.createElementNS(SVG_NS, 'svg');
    svg.setAttribute('viewBox', '0 0 100 100');
    svg.setAttribute('aria-hidden', 'true');
    (PIPS[value] || []).forEach((pos) => {
      const dot = document.createElementNS(SVG_NS, 'circle');
      dot.setAttribute('cx', PIP_XY[pos][0]);
      dot.setAttribute('cy', PIP_XY[pos][1]);
      dot.setAttribute('r', 9);
      dot.setAttribute('class', 'ts-pip');
      svg.appendChild(dot);
    });
    return svg;
  }

  function renderYatzy() {
    els.yatzyDice.innerHTML = '';
    yatzy.values.forEach((v, i) => {
      const die = document.createElement('button');
      die.type = 'button';
      die.className = 'ts-die' + (v ? '' : ' blank') + (yatzy.held[i] ? ' held' : '');
      die.setAttribute('aria-label', v ? `Die ${i + 1}: ${v}${yatzy.held[i] ? ', held' : ''}` : `Die ${i + 1}: not rolled`);
      die.setAttribute('aria-pressed', String(yatzy.held[i]));
      die.appendChild(dieFace(v));
      die.addEventListener('click', () => {
        if (!v || yatzy.rolls === 0 || yatzy.rolls >= 3) return;
        yatzy.held[i] = !yatzy.held[i];
        renderYatzy();
      });
      els.yatzyDice.appendChild(die);
    });
    const sum = yatzy.values.reduce((a, b) => a + b, 0);
    els.yatzyResult.innerHTML = '';
    if (yatzy.rolls > 0) {
      const head = document.createElement('div');
      head.className = 'ts-dim';
      head.textContent = yatzy.rolls >= 3 ? 'Turn over' : `Roll ${yatzy.rolls} of 3`;
      const tot = document.createElement('div');
      tot.className = 'ts-total';
      tot.textContent = 'Sum ' + sum;
      els.yatzyResult.append(head, tot);
    } else {
      els.yatzyResult.innerHTML = '&nbsp;';
    }
    els.yatzyRoll.textContent = yatzy.rolls >= 3 ? 'New turn' : (yatzy.rolls === 0 ? 'Roll' : `Roll again (${3 - yatzy.rolls} left)`);
  }

  function rollYatzy() {
    if (yatzy.rolls >= 3) {
      yatzy.values = [0, 0, 0, 0, 0];
      yatzy.held = [false, false, false, false, false];
      yatzy.rolls = 0;
      renderYatzy();
      return;
    }
    yatzy.values = yatzy.values.map((v, i) => (yatzy.held[i] && v ? v : rollDie(6)));
    yatzy.rolls++;
    if (yatzy.rolls >= 3) yatzy.held = [false, false, false, false, false];
    renderYatzy();
    diceSound();
  }
  els.yatzyRoll.addEventListener('click', rollYatzy);

  // ---------------- Dice: one die ----------------
  const oneDie = $('ts-one-die');
  const oneResult = $('ts-one-result');
  const oneRollBtn = $('ts-one-roll');
  let oneRolling = false;

  function showOne(value) {
    oneDie.innerHTML = '';
    oneDie.appendChild(dieFace(value));
    oneDie.setAttribute('aria-label', value ? `Die shows ${value}. Roll again` : 'Roll the die');
  }

  function rollOne() {
    if (oneRolling) return;
    const value = rollDie(6); // decided first; the tumble only shows faces on the way
    if (reducedMotion()) {
      showOne(value);
      oneResult.innerHTML = '';
      const tot = document.createElement('div');
      tot.className = 'ts-total';
      tot.textContent = String(value);
      oneResult.appendChild(tot);
      diceSound();
      return;
    }
    oneRolling = true;
    oneRollBtn.disabled = true;
    oneResult.innerHTML = '&nbsp;';
    oneDie.classList.remove('rolling');
    void oneDie.offsetWidth; // restart the animation
    oneDie.classList.add('rolling');
    let shown = 0;
    let frame = 0;
    const faces = setInterval(() => {
      // A different face each step while it tumbles
      let next = rollDie(6);
      if (next === shown) next = (next % 6) + 1;
      shown = next;
      showOne(shown);
      if (frame++ % 2 === 0) tick();
    }, 75);
    setTimeout(() => {
      clearInterval(faces);
      showOne(value);
      oneDie.classList.remove('rolling');
      oneResult.innerHTML = '';
      const tot = document.createElement('div');
      tot.className = 'ts-total';
      tot.textContent = String(value);
      oneResult.appendChild(tot);
      diceSound();
      oneRolling = false;
      oneRollBtn.disabled = false;
    }, 650);
  }

  oneDie.addEventListener('click', rollOne);
  oneRollBtn.addEventListener('click', rollOne);

  // ---------------- Dice mode switch ----------------
  let diceMode = 'one';
  function showDiceMode(mode) {
    diceMode = ['one', 'rpg', 'yatzy'].includes(mode) ? mode : 'one';
    document.querySelectorAll('.ts-seg-btn').forEach((b) => b.classList.toggle('active', b.dataset.diceMode === diceMode));
    document.querySelectorAll('.ts-dice-mode').forEach((p) => { p.hidden = p.dataset.dicePanel !== diceMode; });
    store.set('diceMode', diceMode);
  }
  document.querySelectorAll('.ts-seg-btn').forEach((b) => b.addEventListener('click', () => showDiceMode(b.dataset.diceMode)));

  // ---------------- Keyboard ----------------
  // Space or Enter does the main thing on the open tab (not while typing)
  document.addEventListener('keydown', (e) => {
    if (e.key !== ' ' && e.key !== 'Enter') return;
    const t = e.target;
    if (t && (t.tagName === 'TEXTAREA' || t.tagName === 'INPUT' || t.tagName === 'SELECT')) return;
    if (t && t.tagName === 'BUTTON' && !t.classList.contains('ts-tab') && !t.classList.contains('ts-seg-btn')) return; // the button handles it
    e.preventDefault();
    if (activeTab === 'wheel') spin();
    else if (activeTab === 'yesno') askYesNo();
    else if (diceMode === 'yatzy') rollYatzy();
    else if (diceMode === 'one') rollOne();
    else els.rollNotation.click();
  });

  // ---------------- Start ----------------
  let started = false;
  function start() {
    if (started) return;
    started = true;
    loadLists();
    renderListControls();
    els.removeWinner.checked = store.get('removeWinner') === '1';
    setSound(store.get('sound') === '1');
    const notation = store.get('notation');
    if (notation) els.notation.value = notation;
    renderYatzy();
    showOne(0);
    showDiceMode(store.get('diceMode') || 'one');
    showTab(store.get('tab') || 'wheel');
    resizeWheel();
  }

  // Outside Brum (opened directly) there is no bridge: start anyway after a moment
  setTimeout(() => { if (!readyDone) start(); }, 600);

  // Test hook
  window.Tailspin = {
    parseNotation,
    randInt,
    sliceAt,
    // The entry under the pointer right now
    pointing: () => (lists.length && list().entries.length ? list().entries[sliceAt(rotation, list().entries.length)] : null),
  };
})();
