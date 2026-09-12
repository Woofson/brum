// ==========================================================================
// 🕹️ TETRION: 60 FPS RETRO ARCADE PUZZLE CHEWTOY
// ==========================================================================

const ARCADE_PIECES = {
  I: {
    matrix: [
      [0, 0, 0, 0],
      [1, 1, 1, 1],
      [0, 0, 0, 0],
      [0, 0, 0, 0]
    ],
    color: '#06b6d4',
    glow: 'rgba(6, 182, 212, 0.7)'
  },
  J: {
    matrix: [
      [1, 0, 0],
      [1, 1, 1],
      [0, 0, 0]
    ],
    color: '#3b82f6',
    glow: 'rgba(59, 130, 246, 0.7)'
  },
  L: {
    matrix: [
      [0, 0, 1],
      [1, 1, 1],
      [0, 0, 0]
    ],
    color: '#f97316',
    glow: 'rgba(249, 115, 22, 0.7)'
  },
  O: {
    matrix: [
      [1, 1],
      [1, 1]
    ],
    color: '#eab308',
    glow: 'rgba(234, 179, 8, 0.7)'
  },
  S: {
    matrix: [
      [0, 1, 1],
      [1, 1, 0],
      [0, 0, 0]
    ],
    color: '#22c55e',
    glow: 'rgba(34, 197, 94, 0.7)'
  },
  T: {
    matrix: [
      [0, 1, 0],
      [1, 1, 1],
      [0, 0, 0]
    ],
    color: '#a855f7',
    glow: 'rgba(168, 85, 247, 0.7)'
  },
  Z: {
    matrix: [
      [1, 1, 0],
      [0, 1, 1],
      [0, 0, 0]
    ],
    color: '#ef4444',
    glow: 'rgba(239, 68, 68, 0.7)'
  }
};

const ARCADE_SRS_KICKS_JLSTZ = {
  '0->1': [[0, 0], [-1, 0], [-1, 1], [0, -2], [-1, -2]],
  '1->0': [[0, 0], [1, 0], [1, -1], [0, 2], [1, 2]],
  '1->2': [[0, 0], [1, 0], [1, -1], [0, 2], [1, 2]],
  '2->1': [[0, 0], [-1, 0], [-1, 1], [0, -2], [-1, -2]],
  '2->3': [[0, 0], [1, 0], [1, 1], [0, -2], [1, -2]],
  '3->2': [[0, 0], [-1, 0], [-1, -1], [0, 2], [-1, 2]],
  '3->0': [[0, 0], [-1, 0], [-1, -1], [0, 2], [-1, 2]],
  '0->3': [[0, 0], [1, 0], [1, 1], [0, -2], [1, -2]],
};

const ARCADE_SRS_KICKS_I = {
  '0->1': [[0, 0], [-2, 0], [1, 0], [-2, -1], [1, 2]],
  '1->0': [[0, 0], [2, 0], [-1, 0], [2, 1], [-1, -2]],
  '1->2': [[0, 0], [-1, 0], [2, 0], [-1, 2], [2, -1]],
  '2->1': [[0, 0], [1, 0], [-2, 0], [1, -2], [-2, 1]],
  '2->3': [[0, 0], [2, 0], [-1, 0], [2, 1], [-1, -2]],
  '3->2': [[0, 0], [-2, 0], [1, 0], [-2, -1], [1, 2]],
  '3->0': [[0, 0], [1, 0], [-2, 0], [1, -2], [-2, 1]],
  '0->3': [[0, 0], [-1, 0], [2, 0], [-1, 2], [2, -1]],
};

let arcadeState = {
  initialized: false,
  activeView: 'game',
  theme: localStorage.getItem('cd_arcade_theme') || localStorage.getItem('cd_tetradog_theme') || 'amber',
  soundEnabled: localStorage.getItem('cd_arcade_sound') !== '0',
  volume: parseInt(localStorage.getItem('cd_arcade_vol') || '60', 10),
  playerAlias: localStorage.getItem('cd_arcade_alias') || localStorage.getItem('cd_tetradog_alias') || '',
  config: {
    startLevel: parseInt(localStorage.getItem('cd_arcade_start_lvl') || '1', 10),
    rotationSystem: localStorage.getItem('cd_arcade_rotation') || 'srs',
    ghostPiece: localStorage.getItem('cd_arcade_ghost') !== '0',
    das: parseInt(localStorage.getItem('cd_arcade_das') || '133', 10),
    arr: parseInt(localStorage.getItem('cd_arcade_arr') || '16', 10),
    scanlines: localStorage.getItem('cd_arcade_scanlines') !== '0',
    touchControls: localStorage.getItem('cd_arcade_touch_controls') || 'auto',
  },
  game: {
    boardWidth: 10,
    boardHeight: 20,
    visibleHeight: 20,
    cellSize: 24,
    grid: [],
    currentPiece: null,
    nextQueue: [],
    holdPiece: null,
    canHold: true,
    ghostY: 0,
    status: 'ready', // 'ready' | 'playing' | 'paused' | 'gameover'
    mode: 'marathon',
    score: 0,
    lines: 0,
    level: 1,
    highScore: 0,
    b2b: false,
    combo: 0,
    startTime: 0,
    elapsedMs: 0,
    lastFrameTime: 0,
    dropTimer: 0,
    dropInterval: 1000,
    lockTimer: 0,
    lockDelay: 500,
    lockResets: 0,
    maxLockResets: 15,
    isLocking: false,
    animatingRows: [],
    animationTimer: 0,
    keyState: {},
    dasTimer: 0,
    arrTimer: 0,
    dasDir: 0,
    softDropActive: false,
    lastActionWasRotate: false
  },
  leaderboard: {
    mode: 'marathon',
    scope: 'all',
    list: [],
    userStats: null,
    isLoading: false
  }
};

let arcadeAudioCtx = null;
let arcadeRafId = null;

function getAuthToken() {
  try {
    if (window.parent && window.parent.App && window.parent.App.token) {
      return window.parent.App.token;
    }
  } catch (e) {}
  return localStorage.getItem('token') || localStorage.getItem('brum_token') || '';
}

function escapeHtml(str) {
  if (!str) return '';
  return String(str).replace(/[&<>"']/g, m => ({
    '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;'
  })[m]);
}

// ---------------- AUDIO SYNTHESIZER (WEB AUDIO API) ----------------
function initArcadeAudio() {
  if (!arcadeAudioCtx) {
    const AudioCtx = window.AudioContext || window.webkitAudioContext;
    if (AudioCtx) {
      arcadeAudioCtx = new AudioCtx();
    }
  }
  if (arcadeAudioCtx && arcadeAudioCtx.state === 'suspended') {
    arcadeAudioCtx.resume();
  }
}

function playArcadeSound(type) {
  if (!arcadeState.soundEnabled) return;
  try {
    initArcadeAudio();
    if (!arcadeAudioCtx) return;

    const masterVol = (arcadeState.volume / 100) * 0.15;
    const now = arcadeAudioCtx.currentTime;

    if (type === 'move') {
      const osc = arcadeAudioCtx.createOscillator();
      const gain = arcadeAudioCtx.createGain();
      osc.type = 'triangle';
      osc.frequency.setValueAtTime(220, now);
      osc.frequency.exponentialRampToValueAtTime(180, now + 0.035);
      gain.gain.setValueAtTime(masterVol * 0.6, now);
      gain.gain.exponentialRampToValueAtTime(0.001, now + 0.035);
      osc.connect(gain);
      gain.connect(arcadeAudioCtx.destination);
      osc.start(now);
      osc.stop(now + 0.035);
    } else if (type === 'rotate') {
      const osc = arcadeAudioCtx.createOscillator();
      const gain = arcadeAudioCtx.createGain();
      osc.type = 'square';
      osc.frequency.setValueAtTime(320, now);
      osc.frequency.exponentialRampToValueAtTime(640, now + 0.045);
      gain.gain.setValueAtTime(masterVol * 0.5, now);
      gain.gain.exponentialRampToValueAtTime(0.001, now + 0.045);
      osc.connect(gain);
      gain.connect(arcadeAudioCtx.destination);
      osc.start(now);
      osc.stop(now + 0.045);
    } else if (type === 'harddrop') {
      const osc = arcadeAudioCtx.createOscillator();
      const gain = arcadeAudioCtx.createGain();
      osc.type = 'triangle';
      osc.frequency.setValueAtTime(140, now);
      osc.frequency.exponentialRampToValueAtTime(35, now + 0.08);
      gain.gain.setValueAtTime(masterVol * 1.0, now);
      gain.gain.exponentialRampToValueAtTime(0.001, now + 0.08);
      osc.connect(gain);
      gain.connect(arcadeAudioCtx.destination);
      osc.start(now);
      osc.stop(now + 0.08);
    } else if (type === 'lock') {
      const osc = arcadeAudioCtx.createOscillator();
      const gain = arcadeAudioCtx.createGain();
      osc.type = 'triangle';
      osc.frequency.setValueAtTime(440, now);
      osc.frequency.exponentialRampToValueAtTime(110, now + 0.04);
      gain.gain.setValueAtTime(masterVol * 0.6, now);
      gain.gain.exponentialRampToValueAtTime(0.001, now + 0.04);
      osc.connect(gain);
      gain.connect(arcadeAudioCtx.destination);
      osc.start(now);
      osc.stop(now + 0.04);
    } else if (type === 'hold') {
      const osc = arcadeAudioCtx.createOscillator();
      const gain = arcadeAudioCtx.createGain();
      osc.type = 'sine';
      osc.frequency.setValueAtTime(350, now);
      osc.frequency.linearRampToValueAtTime(480, now + 0.06);
      gain.gain.setValueAtTime(masterVol * 0.7, now);
      gain.gain.exponentialRampToValueAtTime(0.001, now + 0.06);
      osc.connect(gain);
      gain.connect(arcadeAudioCtx.destination);
      osc.start(now);
      osc.stop(now + 0.06);
    } else if (type === 'clear') {
      [523.25, 659.25, 783.99].forEach((freq, idx) => {
        const osc = arcadeAudioCtx.createOscillator();
        const gain = arcadeAudioCtx.createGain();
        osc.type = 'square';
        osc.frequency.setValueAtTime(freq, now + idx * 0.04);
        gain.gain.setValueAtTime(masterVol * 0.6, now + idx * 0.04);
        gain.gain.exponentialRampToValueAtTime(0.001, now + idx * 0.04 + 0.09);
        osc.connect(gain);
        gain.connect(arcadeAudioCtx.destination);
        osc.start(now + idx * 0.04);
        osc.stop(now + idx * 0.04 + 0.09);
      });
    } else if (type === 'tetris') {
      [523.25, 659.25, 783.99, 1046.50].forEach((freq, idx) => {
        const osc = arcadeAudioCtx.createOscillator();
        const gain = arcadeAudioCtx.createGain();
        osc.type = 'square';
        osc.frequency.setValueAtTime(freq, now + idx * 0.05);
        gain.gain.setValueAtTime(masterVol * 0.8, now + idx * 0.05);
        gain.gain.exponentialRampToValueAtTime(0.001, now + idx * 0.05 + 0.22);
        osc.connect(gain);
        gain.connect(arcadeAudioCtx.destination);
        osc.start(now + idx * 0.05);
        osc.stop(now + idx * 0.05 + 0.22);
      });
    } else if (type === 'levelup') {
      [440, 554.37, 659.25, 880].forEach((freq, idx) => {
        const osc = arcadeAudioCtx.createOscillator();
        const gain = arcadeAudioCtx.createGain();
        osc.type = 'triangle';
        osc.frequency.setValueAtTime(freq, now + idx * 0.04);
        gain.gain.setValueAtTime(masterVol * 0.7, now + idx * 0.04);
        gain.gain.exponentialRampToValueAtTime(0.001, now + idx * 0.04 + 0.12);
        osc.connect(gain);
        gain.connect(arcadeAudioCtx.destination);
        osc.start(now + idx * 0.04);
        osc.stop(now + idx * 0.04 + 0.12);
      });
    } else if (type === 'gameover') {
      [440, 415.30, 392.00, 349.23].forEach((freq, idx) => {
        const osc = arcadeAudioCtx.createOscillator();
        const gain = arcadeAudioCtx.createGain();
        osc.type = 'sawtooth';
        osc.frequency.setValueAtTime(freq, now + idx * 0.09);
        gain.gain.setValueAtTime(masterVol * 0.6, now + idx * 0.09);
        gain.gain.exponentialRampToValueAtTime(0.001, now + idx * 0.09 + 0.16);
        osc.connect(gain);
        gain.connect(arcadeAudioCtx.destination);
        osc.start(now + idx * 0.09);
        osc.stop(now + idx * 0.09 + 0.16);
      });
    }
  } catch (e) {
    console.debug('ArcadeAudio error:', e);
  }
}

// ---------------- VIEW & THEME SWITCHING ----------------
function setArcadeView(viewName) {
  arcadeState.activeView = viewName;
  ['game', 'leaderboard', 'settings'].forEach(v => {
    const el = document.getElementById(`arcade-view-${v}`);
    const btn = document.getElementById(`btn-arcade-view-${v}`);
    if (el) el.style.display = (v === viewName) ? 'flex' : 'none';
    if (btn) btn.classList.toggle('active', v === viewName);
  });

  if (viewName === 'leaderboard') {
    loadArcadeLeaderboard(arcadeState.leaderboard.mode);
  } else if (viewName === 'game') {
    renderArcadeCanvas();
  }
}

function setArcadeTheme(theme) {
  arcadeState.theme = theme;
  localStorage.setItem('cd_arcade_theme', theme);
  applyArcadeTheme();
}

function applyArcadeTheme() {
  const win = document.getElementById('arcade-app-window');
  if (!win) return;
  win.classList.remove('arcade-theme-amber', 'arcade-theme-gameboy', 'arcade-theme-nes', 'arcade-theme-cyberpunk');
  win.classList.add(`arcade-theme-${arcadeState.theme}`);

  const themeSelect = document.getElementById('arcade-cfg-theme');
  if (themeSelect) themeSelect.value = arcadeState.theme;

  renderArcadeCanvas();
}

function toggleArcadeAudio() {
  arcadeState.soundEnabled = !arcadeState.soundEnabled;
  localStorage.setItem('cd_arcade_sound', arcadeState.soundEnabled ? '1' : '0');

  const icon = document.getElementById('icon-arcade-audio');
  const btn = document.getElementById('btn-arcade-audio');
  if (icon) {
    if (arcadeState.soundEnabled) {
      icon.innerHTML = `<svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"/><path d="M15.54 8.46a5 5 0 0 1 0 7.07"/><path d="M19.07 4.93a10 10 0 0 1 0 14.14"/></svg>`;
    } else {
      icon.innerHTML = `<svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"/><line x1="22" x2="16" y1="9" y2="15"/><line x1="16" x2="22" y1="9" y2="15"/></svg>`;
    }
  }
  if (btn) {
    btn.classList.toggle('muted', !arcadeState.soundEnabled);
    btn.title = arcadeState.soundEnabled ? 'Sound Effects (Click to Mute)' : 'Sound Muted (Click to Unmute)';
  }

  const chk = document.getElementById('arcade-cfg-sound');
  if (chk) chk.checked = arcadeState.soundEnabled;
}

function updateArcadeConfig(key, val) {
  arcadeState.config[key] = val;
  if (key === 'startLevel') {
    localStorage.setItem('cd_arcade_start_lvl', val);
    if (arcadeState.game.status === 'ready') {
      arcadeState.game.level = val;
      updateArcadeHeaderAndStats();
    }
  } else if (key === 'rotationSystem') {
    localStorage.setItem('cd_arcade_rotation', val);
  } else if (key === 'ghostPiece') {
    localStorage.setItem('cd_arcade_ghost', val ? '1' : '0');
    renderArcadeCanvas();
  } else if (key === 'das') {
    localStorage.setItem('cd_arcade_das', val);
  } else if (key === 'arr') {
    localStorage.setItem('cd_arcade_arr', val);
  } else if (key === 'scanlines') {
    localStorage.setItem('cd_arcade_scanlines', val ? '1' : '0');
    const scanlinesEl = document.getElementById('arcade-scanlines');
    if (scanlinesEl) scanlinesEl.style.display = val ? 'block' : 'none';
  } else if (key === 'volume') {
    arcadeState.volume = val;
    localStorage.setItem('cd_arcade_vol', val);
  } else if (key === 'touchControls') {
    arcadeState.config.touchControls = val;
    localStorage.setItem('cd_arcade_touch_controls', val);
    applyViewportAndControls();
  }
}

function applyViewportAndControls() {
  const win = document.getElementById('arcade-app-window');
  if (!win) return;

  const isTouch = (arcadeState.isTouch !== undefined)
    ? arcadeState.isTouch
    : (('ontouchstart' in window) || (navigator.maxTouchPoints > 0) || window.matchMedia('(pointer: coarse)').matches);

  const vp = arcadeState.viewport || (window.innerWidth < 600 ? 'phone' : (window.innerWidth <= 1024 ? 'tablet' : 'pc'));

  win.classList.remove('viewport-phone', 'viewport-tablet', 'viewport-pc', 'touch-device');
  win.classList.add(`viewport-${vp}`);
  if (isTouch) win.classList.add('touch-device');

  const setting = arcadeState.config.touchControls || 'auto';
  win.classList.remove('touch-controls-enabled', 'touch-controls-disabled');
  if (setting === 'show') {
    win.classList.add('touch-controls-enabled');
  } else if (setting === 'hide') {
    win.classList.add('touch-controls-disabled');
  } else {
    // auto: show only if touch is active on phone or tablet
    if (isTouch && (vp === 'phone' || vp === 'tablet')) {
      win.classList.add('touch-controls-enabled');
    } else {
      win.classList.add('touch-controls-disabled');
    }
  }

  const touchSelect = document.getElementById('arcade-cfg-touch-ctrl');
  if (touchSelect && touchSelect.value !== setting) {
    touchSelect.value = setting;
  }
}

// ---------------- GAME ENGINE IMPLEMENTATION ----------------
function initArcadeGame() {
  const g = arcadeState.game;
  g.boardWidth = 10;
  g.boardHeight = 20;
  g.visibleHeight = 20;
  g.grid = createEmptyArcadeGrid(g.boardHeight, g.boardWidth);
  g.nextQueue = [];
  g.holdPiece = null;
  g.canHold = true;
  g.score = 0;
  g.lines = 0;
  g.level = arcadeState.config.startLevel;
  g.b2b = false;
  g.combo = 0;
  g.status = 'ready';
  g.dropInterval = getArcadeGravityMs(g.level);

  while (g.nextQueue.length < 5) {
    g.nextQueue.push(...generateArcadeBag());
  }

  updateArcadeHeaderAndStats();
  renderArcadeCanvas();
  renderHoldCanvas();
  renderNextCanvas();

  if (!arcadeRafId) {
    g.lastFrameTime = performance.now();
    arcadeRafId = requestAnimationFrame(arcadeGameLoop);
  }
}

function createEmptyArcadeGrid(rows, cols) {
  const grid = [];
  for (let r = 0; r < rows; r++) {
    grid.push(new Array(cols).fill(0));
  }
  return grid;
}

function generateArcadeBag() {
  const bag = ['I', 'J', 'L', 'O', 'S', 'T', 'Z'];
  for (let i = bag.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [bag[i], bag[j]] = [bag[j], bag[i]];
  }
  return bag;
}

function getArcadeGravityMs(level) {
  const clampedLvl = Math.min(20, Math.max(1, level));
  const ms = Math.pow(0.8 - ((clampedLvl - 1) * 0.007), clampedLvl - 1) * 1000;
  return Math.max(20, Math.floor(ms));
}

function startArcadeGame() {
  if (!arcadeState.initialized) {
    applyArcadeTheme();
    initArcadeInput();
    arcadeState.initialized = true;
  }
  initArcadeAudio();
  const g = arcadeState.game;
  g.boardWidth = 10;
  g.boardHeight = 20;
  g.visibleHeight = 20;
  g.grid = createEmptyArcadeGrid(g.boardHeight, g.boardWidth);
  g.nextQueue = [];
  g.holdPiece = null;
  g.canHold = true;
  g.score = 0;
  g.lines = 0;
  g.level = arcadeState.config.startLevel;
  g.b2b = false;
  g.combo = 0;
  g.dropInterval = getArcadeGravityMs(g.level);
  g.dropTimer = 0;
  g.lockTimer = 0;
  g.lockResets = 0;
  g.isLocking = false;
  g.animatingRows = [];
  g.startTime = Date.now();
  g.elapsedMs = 0;
  g.status = 'playing';

  while (g.nextQueue.length < 5) {
    g.nextQueue.push(...generateArcadeBag());
  }

  spawnArcadePiece();

  const readyOverlay = document.getElementById('arcade-ready-overlay');
  if (readyOverlay) readyOverlay.style.display = 'none';
  const pauseOverlay = document.getElementById('arcade-pause-overlay');
  if (pauseOverlay) pauseOverlay.style.display = 'none';
  const goOverlay = document.getElementById('arcade-gameover-overlay');
  if (goOverlay) goOverlay.style.display = 'none';

  updatePlayPauseButtons();
  updateArcadeHeaderAndStats();
  playArcadeSound('levelup');

  if (!arcadeRafId) {
    g.lastFrameTime = performance.now();
    arcadeRafId = requestAnimationFrame(arcadeGameLoop);
  }
}

function pauseArcadeGame() {
  const g = arcadeState.game;
  if (g.status !== 'playing') return;
  g.status = 'paused';
  const pauseOverlay = document.getElementById('arcade-pause-overlay');
  if (pauseOverlay) pauseOverlay.style.display = 'flex';
  updatePlayPauseButtons();
}

function resumeArcadeGame() {
  const g = arcadeState.game;
  if (g.status !== 'paused') return;
  g.status = 'playing';
  g.lastFrameTime = performance.now();
  const pauseOverlay = document.getElementById('arcade-pause-overlay');
  if (pauseOverlay) pauseOverlay.style.display = 'none';
  updatePlayPauseButtons();
}

function toggleArcadePlayPause() {
  const g = arcadeState.game;
  if (g.status === 'ready' || g.status === 'gameover') {
    startArcadeGame();
  } else if (g.status === 'playing') {
    pauseArcadeGame();
  } else if (g.status === 'paused') {
    resumeArcadeGame();
  }
}

function restartArcadeGame() {
  startArcadeGame();
}

function updatePlayPauseButtons() {
  const g = arcadeState.game;
  const label = document.getElementById('label-arcade-play-pause');
  const icon = document.getElementById('icon-arcade-play-pause');
  if (label && icon) {
    if (g.status === 'playing') {
      label.textContent = 'Pause';
      icon.innerHTML = `<svg xmlns="http://www.w3.org/2000/svg" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect width="4" height="16" x="6" y="4"/><rect width="4" height="16" x="14" y="4"/></svg>`;
    } else {
      label.textContent = 'Play';
      icon.innerHTML = `<svg xmlns="http://www.w3.org/2000/svg" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="6 3 20 12 6 21 6 3"/></svg>`;
    }
  }
}

function spawnArcadePiece() {
  const g = arcadeState.game;
  if (g.nextQueue.length < 5) {
    g.nextQueue.push(...generateArcadeBag());
  }

  const type = g.nextQueue.shift();
  const def = ARCADE_PIECES[type];
  const matrix = JSON.parse(JSON.stringify(def.matrix));

  const startX = Math.floor((g.boardWidth - matrix[0].length) / 2);
  const startY = (type === 'I') ? -1 : 0;

  g.currentPiece = {
    type,
    matrix,
    x: startX,
    y: startY,
    rotation: 0
  };

  g.canHold = true;
  g.lockTimer = 0;
  g.lockResets = 0;
  g.isLocking = false;
  g.lastActionWasRotate = false;

  if (checkArcadeCollision(g.grid, g.currentPiece)) {
    endArcadeGame('topout');
    return;
  }

  calculateGhostY();
  renderHoldCanvas();
  renderNextCanvas();
  renderArcadeCanvas();
}

function holdArcadePiece() {
  const g = arcadeState.game;
  if (g.status !== 'playing' || !g.canHold || !g.currentPiece) return;

  playArcadeSound('hold');
  const curType = g.currentPiece.type;

  if (g.holdPiece) {
    const swapType = g.holdPiece;
    g.holdPiece = curType;
    const def = ARCADE_PIECES[swapType];
    g.currentPiece = {
      type: swapType,
      matrix: JSON.parse(JSON.stringify(def.matrix)),
      x: Math.floor((g.boardWidth - def.matrix[0].length) / 2),
      y: (swapType === 'I') ? -1 : 0,
      rotation: 0
    };
  } else {
    g.holdPiece = curType;
    spawnArcadePiece();
  }

  g.canHold = false;
  calculateGhostY();
  renderHoldCanvas();
  renderArcadeCanvas();
}

function moveArcadePiece(dx, dy) {
  const g = arcadeState.game;
  if (g.status !== 'playing' || !g.currentPiece) return false;

  const testPiece = {
    ...g.currentPiece,
    x: g.currentPiece.x + dx,
    y: g.currentPiece.y + dy
  };

  if (!checkArcadeCollision(g.grid, testPiece)) {
    g.currentPiece.x += dx;
    g.currentPiece.y += dy;
    g.lastActionWasRotate = false;

    if (dx !== 0) {
      playArcadeSound('move');
      if (g.isLocking && g.lockResets < g.maxLockResets) {
        g.lockTimer = 0;
        g.lockResets++;
      }
    }
    if (dy > 0) {
      g.score += 1;
      updateArcadeHeaderAndStats();
    }

    calculateGhostY();
    return true;
  }
  return false;
}

function rotateArcadePiece(dir = 1) {
  const g = arcadeState.game;
  if (g.status !== 'playing' || !g.currentPiece) return;

  const p = g.currentPiece;
  const oldRot = p.rotation;
  const newRot = (oldRot + dir + 4) % 4;
  const rotatedMatrix = rotateMatrix(p.matrix, dir);

  if (arcadeState.config.rotationSystem === 'srs') {
    const kickKey = `${oldRot}->${newRot}`;
    const kicks = (p.type === 'I')
      ? (ARCADE_SRS_KICKS_I[kickKey] || [[0, 0]])
      : (ARCADE_SRS_KICKS_JLSTZ[kickKey] || [[0, 0]]);

    for (let i = 0; i < kicks.length; i++) {
      const [kx, ky] = kicks[i];
      const testPiece = {
        ...p,
        matrix: rotatedMatrix,
        x: p.x + kx,
        y: p.y - ky,
        rotation: newRot
      };

      if (!checkArcadeCollision(g.grid, testPiece)) {
        p.matrix = rotatedMatrix;
        p.x += kx;
        p.y -= ky;
        p.rotation = newRot;
        p.lastActionWasRotate = true;
        playArcadeSound('rotate');

        if (g.isLocking && g.lockResets < g.maxLockResets) {
          g.lockTimer = 0;
          g.lockResets++;
        }

        calculateGhostY();
        return;
      }
    }
  } else {
    if (!checkArcadeCollision(g.grid, { ...p, matrix: rotatedMatrix, rotation: newRot })) {
      p.matrix = rotatedMatrix;
      p.rotation = newRot;
      p.lastActionWasRotate = true;
      playArcadeSound('rotate');
      calculateGhostY();
    }
  }
}

function rotateMatrix(matrix, dir = 1) {
  const N = matrix.length;
  const res = [];
  for (let r = 0; r < N; r++) {
    res.push(new Array(N).fill(0));
  }
  if (dir === 1) {
    for (let r = 0; r < N; r++) {
      for (let c = 0; c < N; c++) {
        res[c][N - 1 - r] = matrix[r][c];
      }
    }
  } else {
    for (let r = 0; r < N; r++) {
      for (let c = 0; c < N; c++) {
        res[N - 1 - c][r] = matrix[r][c];
      }
    }
  }
  return res;
}

function hardDropArcadePiece() {
  const g = arcadeState.game;
  if (g.status !== 'playing' || !g.currentPiece) return;

  let droppedRows = 0;
  while (!checkArcadeCollision(g.grid, { ...g.currentPiece, y: g.currentPiece.y + 1 })) {
    g.currentPiece.y++;
    droppedRows++;
  }

  g.score += droppedRows * 2;
  playArcadeSound('harddrop');
  lockArcadePiece();
}

function calculateGhostY() {
  const g = arcadeState.game;
  if (!g.currentPiece) return;

  let gy = g.currentPiece.y;
  while (!checkArcadeCollision(g.grid, { ...g.currentPiece, y: gy + 1 })) {
    gy++;
  }
  g.ghostY = gy;
}

function checkArcadeCollision(grid, piece) {
  const { matrix, x, y } = piece;
  for (let r = 0; r < matrix.length; r++) {
    for (let c = 0; c < matrix[r].length; c++) {
      if (matrix[r][c]) {
        const boardX = x + c;
        const boardY = y + r;

        if (boardX < 0 || boardX >= 10 || boardY >= 20) {
          return true;
        }
        if (boardY >= 0 && grid[boardY] && grid[boardY][boardX]) {
          return true;
        }
      }
    }
  }
  return false;
}

function lockArcadePiece() {
  const g = arcadeState.game;
  if (!g.currentPiece) return;

  const { matrix, x, y, type } = g.currentPiece;
  let topOut = false;
  for (let r = 0; r < matrix.length; r++) {
    for (let c = 0; c < matrix[r].length; c++) {
      if (matrix[r][c]) {
        const by = y + r;
        const bx = x + c;
        if (by < 0) {
          topOut = true;
        } else if (by < g.boardHeight && bx >= 0 && bx < g.boardWidth) {
          g.grid[by][bx] = type;
        }
      }
    }
  }

  if (topOut) {
    endArcadeGame('topout');
    return;
  }

  playArcadeSound('lock');
  g.currentPiece = null;
  g.isLocking = false;

  checkLineClears();
}

function checkLineClears() {
  const g = arcadeState.game;
  const fullRows = [];

  for (let r = 0; r < g.boardHeight; r++) {
    if (g.grid[r].every(cell => cell !== 0)) {
      fullRows.push(r);
    }
  }

  if (fullRows.length > 0) {
    g.animatingRows = fullRows;
    g.animationTimer = 120;

    const isTetris = fullRows.length === 4;
    if (isTetris) {
      if (g.b2b) {
        g.score += Math.floor(1200 * g.level * 1.5);
      } else {
        g.score += 800 * g.level;
      }
      g.b2b = true;
      playArcadeSound('tetris');
    } else {
      const scoresMap = { 1: 100, 2: 300, 3: 500 };
      g.score += (scoresMap[fullRows.length] || 100) * g.level;
      g.b2b = false;
      playArcadeSound('clear');
    }

    g.lines += fullRows.length;
    g.combo++;

    const newLevel = arcadeState.config.startLevel + Math.floor(g.lines / 10);
    if (newLevel > g.level) {
      g.level = newLevel;
      g.dropInterval = getArcadeGravityMs(g.level);
      playArcadeSound('levelup');
    }

    if (g.mode === 'sprint' && g.lines >= 40) {
      endArcadeGame('sprint_complete');
      return;
    }
  } else {
    g.combo = 0;
    spawnArcadePiece();
  }

  updateArcadeHeaderAndStats();
}

function executeLineCollapse() {
  const g = arcadeState.game;
  if (g.animatingRows.length === 0) return;

  g.animatingRows.sort((a, b) => a - b);
  g.animatingRows.forEach(rowIdx => {
    g.grid.splice(rowIdx, 1);
    g.grid.unshift(new Array(g.boardWidth).fill(0));
  });

  g.animatingRows = [];
  spawnArcadePiece();
}

function endArcadeGame(reason = 'topout') {
  const g = arcadeState.game;
  g.status = 'gameover';
  playArcadeSound('gameover');

  const durationSec = Math.floor(g.elapsedMs / 1000);

  const titleEl = document.getElementById('arcade-gameover-title');
  if (titleEl) {
    if (reason === 'sprint_complete') {
      titleEl.textContent = 'SPRINT FINISHED!';
      titleEl.className = 'arcade-overlay-title arcade-accent-glow';
    } else if (reason === 'time_up') {
      titleEl.textContent = 'TIME IS UP!';
      titleEl.className = 'arcade-overlay-title arcade-accent-glow';
    } else {
      titleEl.textContent = 'GAME OVER';
      titleEl.className = 'arcade-overlay-title arcade-danger-glow';
    }
  }

  const scoreEl = document.getElementById('arcade-go-score');
  const lvlEl = document.getElementById('arcade-go-level');
  const linesEl = document.getElementById('arcade-go-lines');
  const timeEl = document.getElementById('arcade-go-time');

  if (scoreEl) scoreEl.textContent = g.score.toLocaleString();
  if (lvlEl) lvlEl.textContent = g.level;
  if (linesEl) linesEl.textContent = g.lines;
  if (timeEl) timeEl.textContent = formatArcadeTimer(g.elapsedMs);

  const goOverlay = document.getElementById('arcade-gameover-overlay');
  if (goOverlay) goOverlay.style.display = 'flex';
  updatePlayPauseButtons();

  submitArcadeScore(g.score, g.lines, g.level, durationSec, g.mode);
}

function changeArcadeMode(mode) {
  arcadeState.game.mode = mode;
  updateArcadeHeaderAndStats();

  const timerRow = document.getElementById('arcade-timer-row');
  if (timerRow) {
    timerRow.style.display = (mode === 'sprint' || mode === 'ultra') ? 'flex' : 'none';
  }

  if (arcadeState.game.status === 'ready') {
    initArcadeGame();
  }
}

// ---------------- GAME PHYSICS LOOP (60 FPS FIXED TIMESTEP) ----------------
function arcadeGameLoop(timestamp) {
  const g = arcadeState.game;
  const delta = timestamp - (g.lastFrameTime || timestamp);
  g.lastFrameTime = timestamp;

  if (g.status === 'playing') {
    g.elapsedMs += delta;

    if (g.mode === 'ultra' && g.elapsedMs >= 180000) {
      endArcadeGame('time_up');
      return;
    }

    const timerEl = document.getElementById('arcade-stat-timer');
    if (timerEl && (g.mode === 'sprint' || g.mode === 'ultra')) {
      if (g.mode === 'ultra') {
        const remain = Math.max(0, 180000 - g.elapsedMs);
        timerEl.textContent = formatArcadeTimer(remain);
      } else {
        timerEl.textContent = formatArcadeTimer(g.elapsedMs);
      }
    }

    if (g.animatingRows.length > 0) {
      g.animationTimer -= delta;
      if (g.animationTimer <= 0) {
        executeLineCollapse();
      }
    } else if (g.currentPiece) {
      handleArcadeContinuousInput(delta);

      g.dropTimer += delta;
      const currentInterval = g.softDropActive ? Math.min(40, g.dropInterval / 12) : g.dropInterval;

      if (g.dropTimer >= currentInterval) {
        g.dropTimer = 0;
        const moved = moveArcadePiece(0, 1);
        if (!moved) {
          g.isLocking = true;
        }
      }

      if (g.isLocking) {
        g.lockTimer += delta;
        if (g.lockTimer >= g.lockDelay) {
          lockArcadePiece();
        }
      }
    }
  }

  renderArcadeCanvas();
  arcadeRafId = requestAnimationFrame(arcadeGameLoop);
}

function handleArcadeContinuousInput(delta) {
  const g = arcadeState.game;
  if (!g.currentPiece || g.dasDir === 0) return;

  g.dasTimer += delta;
  if (g.dasTimer >= arcadeState.config.das) {
    g.arrTimer += delta;
    const arrSpeed = Math.max(0, arcadeState.config.arr);

    if (arrSpeed === 0) {
      while (moveArcadePiece(g.dasDir, 0)) {}
    } else if (g.arrTimer >= arrSpeed) {
      g.arrTimer = 0;
      moveArcadePiece(g.dasDir, 0);
    }
  }
}

// ---------------- CANVAS RENDERING ----------------
function renderArcadeCanvas() {
  const canvas = document.getElementById('arcade-board-canvas');
  if (!canvas) return;
  const ctx = canvas.getContext('2d');
  const g = arcadeState.game;

  const w = canvas.width;
  const h = canvas.height;
  const cellSize = w / g.boardWidth;

  ctx.fillStyle = (arcadeState.theme === 'gameboy') ? '#0f380f' : '#040507';
  ctx.fillRect(0, 0, w, h);

  ctx.strokeStyle = (arcadeState.theme === 'gameboy') ? 'rgba(48, 98, 48, 0.3)' : 'rgba(255, 255, 255, 0.04)';
  ctx.lineWidth = 1;
  for (let c = 1; c < g.boardWidth; c++) {
    ctx.beginPath();
    ctx.moveTo(c * cellSize, 0);
    ctx.lineTo(c * cellSize, h);
    ctx.stroke();
  }
  for (let r = 1; r < g.boardHeight; r++) {
    ctx.beginPath();
    ctx.moveTo(0, r * cellSize);
    ctx.lineTo(w, r * cellSize);
    ctx.stroke();
  }

  for (let r = 0; r < g.boardHeight; r++) {
    const isFlashing = g.animatingRows.includes(r);
    for (let c = 0; c < g.boardWidth; c++) {
      const type = g.grid[r] ? g.grid[r][c] : 0;
      if (type && type !== 0) {
        if (isFlashing) {
          drawArcadeBlock(ctx, c * cellSize, r * cellSize, cellSize, '#ffffff', 'rgba(255, 255, 255, 0.8)');
        } else {
          const pieceDef = ARCADE_PIECES[type];
          if (pieceDef) {
            drawArcadeBlock(ctx, c * cellSize, r * cellSize, cellSize, pieceDef.color, pieceDef.glow);
          }
        }
      }
    }
  }

  if (arcadeState.config.ghostPiece && g.currentPiece && (g.status === 'playing' || g.status === 'paused')) {
    const { matrix, x } = g.currentPiece;
    const gy = g.ghostY;
    const pieceDef = ARCADE_PIECES[g.currentPiece.type];
    for (let r = 0; r < matrix.length; r++) {
      for (let c = 0; c < matrix[r].length; c++) {
        if (matrix[r][c]) {
          const drawY = gy + r;
          if (drawY >= 0 && drawY < g.boardHeight) {
            drawGhostBlock(ctx, (x + c) * cellSize, drawY * cellSize, cellSize, pieceDef ? pieceDef.color : '#f59e0b');
          }
        }
      }
    }
  }

  if (g.currentPiece && (g.status === 'playing' || g.status === 'paused')) {
    const { matrix, x, y, type } = g.currentPiece;
    const pieceDef = ARCADE_PIECES[type];
    if (pieceDef) {
      for (let r = 0; r < matrix.length; r++) {
        for (let c = 0; c < matrix[r].length; c++) {
          if (matrix[r][c]) {
            const drawY = y + r;
            if (drawY >= 0 && drawY < g.boardHeight) {
              drawArcadeBlock(ctx, (x + c) * cellSize, drawY * cellSize, cellSize, pieceDef.color, pieceDef.glow);
            }
          }
        }
      }
    }
  }
}

function drawArcadeBlock(ctx, x, y, size, color, glow) {
  ctx.save();
  const theme = arcadeState.theme;

  if (theme === 'gameboy') {
    ctx.fillStyle = '#8bac0f';
    ctx.fillRect(x + 1, y + 1, size - 2, size - 2);
    ctx.fillStyle = '#306230';
    ctx.fillRect(x + 3, y + 3, size - 6, size - 6);
    ctx.fillStyle = '#9bbc0f';
    ctx.fillRect(x + 4, y + 4, size - 8, size - 8);
  } else {
    ctx.fillStyle = color;
    ctx.fillRect(x + 1, y + 1, size - 2, size - 2);

    ctx.fillStyle = 'rgba(255, 255, 255, 0.4)';
    ctx.beginPath();
    ctx.moveTo(x + 1, y + 1);
    ctx.lineTo(x + size - 1, y + 1);
    ctx.lineTo(x + size - 4, y + 4);
    ctx.lineTo(x + 4, y + 4);
    ctx.lineTo(x + 4, y + size - 4);
    ctx.lineTo(x + 1, y + size - 1);
    ctx.closePath();
    ctx.fill();

    ctx.fillStyle = 'rgba(0, 0, 0, 0.4)';
    ctx.beginPath();
    ctx.moveTo(x + size - 1, y + 1);
    ctx.lineTo(x + size - 1, y + size - 1);
    ctx.lineTo(x + 1, y + size - 1);
    ctx.lineTo(x + 4, y + size - 4);
    ctx.lineTo(x + size - 4, y + size - 4);
    ctx.lineTo(x + size - 4, y + 4);
    ctx.closePath();
    ctx.fill();

    ctx.fillStyle = 'rgba(255, 255, 255, 0.15)';
    ctx.fillRect(x + 6, y + 6, size - 12, size - 12);
  }
  ctx.restore();
}

function drawGhostBlock(ctx, x, y, size, color) {
  ctx.save();
  ctx.strokeStyle = color;
  ctx.lineWidth = 1.5;
  ctx.strokeRect(x + 2, y + 2, size - 4, size - 4);
  ctx.fillStyle = 'rgba(255, 255, 255, 0.05)';
  ctx.fillRect(x + 2, y + 2, size - 4, size - 4);
  ctx.restore();
}

function renderHoldCanvas() {
  const canvas = document.getElementById('arcade-hold-canvas');
  if (!canvas) return;
  const ctx = canvas.getContext('2d');
  ctx.clearRect(0, 0, canvas.width, canvas.height);

  const holdType = arcadeState.game.holdPiece;
  if (!holdType) return;

  const def = ARCADE_PIECES[holdType];
  const matrix = def.matrix;
  const cellSize = 18;
  const offsetX = (canvas.width - matrix[0].length * cellSize) / 2;
  const offsetY = (canvas.height - matrix.length * cellSize) / 2;

  for (let r = 0; r < matrix.length; r++) {
    for (let c = 0; c < matrix[r].length; c++) {
      if (matrix[r][c]) {
        drawArcadeBlock(ctx, offsetX + c * cellSize, offsetY + r * cellSize, cellSize, def.color, def.glow);
      }
    }
  }
}

function renderNextCanvas() {
  const canvas = document.getElementById('arcade-next-canvas');
  if (!canvas) return;
  const ctx = canvas.getContext('2d');
  ctx.clearRect(0, 0, canvas.width, canvas.height);

  const queue = arcadeState.game.nextQueue.slice(0, 3);
  const cellSize = 16;

  queue.forEach((type, idx) => {
    const def = ARCADE_PIECES[type];
    const matrix = def.matrix;
    const offsetX = (canvas.width - matrix[0].length * cellSize) / 2;
    const offsetY = 12 + idx * 64 + (48 - matrix.length * cellSize) / 2;

    for (let r = 0; r < matrix.length; r++) {
      for (let c = 0; c < matrix[r].length; c++) {
        if (matrix[r][c]) {
          drawArcadeBlock(ctx, offsetX + c * cellSize, offsetY + r * cellSize, cellSize, def.color, def.glow);
        }
      }
    }
  });
}

function updateArcadeHeaderAndStats() {
  const g = arcadeState.game;

  const sbScore = document.getElementById('arcade-sb-score-val');
  const sbLvl = document.getElementById('arcade-sb-lvl-val');
  const sbLines = document.getElementById('arcade-sb-lines-val');
  if (sbScore) sbScore.textContent = g.score.toLocaleString();
  if (sbLvl) sbLvl.textContent = g.level;
  if (sbLines) sbLines.textContent = g.lines;

  const hiScoreEl = document.getElementById('arcade-stat-hiscore');
  if (hiScoreEl) hiScoreEl.textContent = g.highScore.toLocaleString();
  const mobHiScore = document.getElementById('arcade-mob-hiscore');
  if (mobHiScore) mobHiScore.textContent = g.highScore.toLocaleString();

  const arenaBadge = document.getElementById('arcade-arena-mode-badge');
  if (arenaBadge) arenaBadge.textContent = g.mode.toUpperCase();
  const mobModeBadge = document.getElementById('arcade-mobile-mode-badge');
  if (mobModeBadge) mobModeBadge.textContent = g.mode.toUpperCase();
  const modeSel = document.getElementById('arcade-mode-select');
  if (modeSel && modeSel.value !== g.mode) modeSel.value = g.mode;

  const soundBtn = document.getElementById('btn-arcade-audio');
  if (soundBtn) {
    soundBtn.classList.toggle('muted', !arcadeState.soundEnabled);
  }

  const b2bBadge = document.getElementById('arcade-b2b-badge');
  if (b2bBadge) b2bBadge.style.display = g.b2b ? 'block' : 'none';

  const comboBadge = document.getElementById('arcade-combo-badge');
  const comboCnt = document.getElementById('arcade-combo-count');
  if (comboBadge && comboCnt) {
    if (g.combo > 1) {
      comboCnt.textContent = g.combo;
      comboBadge.style.display = 'block';
    } else {
      comboBadge.style.display = 'none';
    }
  }
}

function formatArcadeTimer(ms) {
  const totalSec = Math.floor(ms / 1000);
  const minutes = Math.floor(totalSec / 60);
  const seconds = totalSec % 60;
  const tenths = Math.floor((ms % 1000) / 100);
  return `${String(minutes).padStart(2, '0')}:${String(seconds).padStart(2, '0')}.${tenths}`;
}

// ---------------- INPUT EVENT LISTENERS (KEYBOARD & TOUCH) ----------------
function handleArcadeKeyDown(key) {
  if (arcadeState.activeView !== 'game') return;
  const g = arcadeState.game;

  if (key === 'p' || key === 'P' || key === 'Escape') {
    toggleArcadePlayPause();
    return;
  }

  if (g.status === 'ready' || g.status === 'gameover') {
    if (['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', ' ', 'Space', 'Enter', 'w', 'a', 's', 'd', 'W', 'A', 'S', 'D', 'z', 'x', 'c', 'Z', 'X', 'C'].includes(key)) {
      startArcadeGame();
      return;
    }
  }

  if (g.status !== 'playing') return;

  if (key === 'ArrowLeft' || key === 'a' || key === 'A') {
    if (g.dasDir !== -1) {
      g.dasDir = -1;
      g.dasTimer = 0;
      g.arrTimer = 0;
      moveArcadePiece(-1, 0);
    }
  } else if (key === 'ArrowRight' || key === 'd' || key === 'D') {
    if (g.dasDir !== 1) {
      g.dasDir = 1;
      g.dasTimer = 0;
      g.arrTimer = 0;
      moveArcadePiece(1, 0);
    }
  } else if (key === 'ArrowDown' || key === 's' || key === 'S') {
    g.softDropActive = true;
  } else if (key === 'ArrowUp' || key === 'x' || key === 'X' || key === 'e' || key === 'E') {
    rotateArcadePiece(1);
  } else if (key === 'z' || key === 'Z' || key === 'Control' || key === 'q' || key === 'Q') {
    rotateArcadePiece(-1);
  } else if (key === ' ' || key === 'Space' || key === 'w' || key === 'W') {
    hardDropArcadePiece();
  } else if (key === 'c' || key === 'C' || key === 'Shift') {
    holdArcadePiece();
  }
}

function handleArcadeKeyUp(key) {
  const g = arcadeState.game;
  if (key === 'ArrowLeft' || key === 'a' || key === 'A') {
    if (g.dasDir === -1) g.dasDir = 0;
  } else if (key === 'ArrowRight' || key === 'd' || key === 'D') {
    if (g.dasDir === 1) g.dasDir = 0;
  } else if (key === 'ArrowDown' || key === 's' || key === 'S') {
    g.softDropActive = false;
  }
}

function initArcadeInput() {
  window.addEventListener('keydown', (e) => {
    if (['INPUT', 'TEXTAREA', 'SELECT'].includes(document.activeElement?.tagName) || document.activeElement?.isContentEditable) return;

    if (['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', ' ', 'Space'].includes(e.key)) {
      e.preventDefault();
    }
    handleArcadeKeyDown(e.key);
  });

  window.addEventListener('keyup', (e) => {
    if (['INPUT', 'TEXTAREA', 'SELECT'].includes(document.activeElement?.tagName) || document.activeElement?.isContentEditable) return;
    handleArcadeKeyUp(e.key);
  });

  const touchMap = {
    'tbtn-left': { down: () => { if (arcadeState.game.status !== 'playing') startArcadeGame(); arcadeState.game.dasDir = -1; arcadeState.game.dasTimer = 0; moveArcadePiece(-1, 0); }, up: () => { if (arcadeState.game.dasDir === -1) arcadeState.game.dasDir = 0; } },
    'tbtn-right': { down: () => { if (arcadeState.game.status !== 'playing') startArcadeGame(); arcadeState.game.dasDir = 1; arcadeState.game.dasTimer = 0; moveArcadePiece(1, 0); }, up: () => { if (arcadeState.game.dasDir === 1) arcadeState.game.dasDir = 0; } },
    'tbtn-down': { down: () => { if (arcadeState.game.status !== 'playing') startArcadeGame(); arcadeState.game.softDropActive = true; }, up: () => { arcadeState.game.softDropActive = false; } },
    'tbtn-up': { down: () => { if (arcadeState.game.status !== 'playing') startArcadeGame(); else hardDropArcadePiece(); } },
    'tbtn-rot-cw': { down: () => { if (arcadeState.game.status !== 'playing') startArcadeGame(); else rotateArcadePiece(1); } },
    'tbtn-rot-ccw': { down: () => { if (arcadeState.game.status !== 'playing') startArcadeGame(); else rotateArcadePiece(-1); } },
    'tbtn-hold': { down: () => { if (arcadeState.game.status !== 'playing') startArcadeGame(); else holdArcadePiece(); } },
    'tbtn-pause': { down: () => { toggleArcadePlayPause(); } }
  };

  Object.entries(touchMap).forEach(([btnId, handlers]) => {
    const el = document.getElementById(btnId);
    if (!el) return;
    el.addEventListener('touchstart', (e) => {
      e.preventDefault();
      if (handlers.down) handlers.down();
    }, { passive: false });
    if (handlers.up) {
      el.addEventListener('touchend', (e) => {
        e.preventDefault();
        handlers.up();
      }, { passive: false });
      el.addEventListener('touchcancel', (e) => {
        e.preventDefault();
        handlers.up();
      }, { passive: false });
    }
    el.addEventListener('mousedown', (e) => {
      e.preventDefault();
      if (handlers.down) handlers.down();
    });
    if (handlers.up) {
      el.addEventListener('mouseup', (e) => {
        e.preventDefault();
        handlers.up();
      });
    }
  });
}

// ---------------- LEADERBOARD & DB SYNCHRONIZATION ----------------
async function loadArcadeLeaderboard(mode = 'marathon', showLoading = true) {
  arcadeState.leaderboard.mode = mode;
  const tbody = document.getElementById('arcade-leaderboard-tbody');

  document.querySelectorAll('.arcade-lb-tab').forEach(b => {
    b.classList.toggle('active', b.getAttribute('data-mode') === mode);
  });

  if (showLoading && tbody) {
    tbody.innerHTML = `<tr><td colspan="7" style="text-align: center; color: var(--text-dim); padding: 24px;">Fetching top scores from server...</td></tr>`;
  }

  const userOnly = arcadeState.leaderboard.scope === 'mine';
  const token = getAuthToken();
  const url = `/api/chewtoys/tetradog/scores?mode=${encodeURIComponent(mode)}&user_only=${userOnly}&limit=50`;

  try {
    const headers = {};
    if (token) headers['Authorization'] = `Bearer ${token}`;

    const resp = await fetch(url, { headers });

    if (resp.ok) {
      const data = await resp.json();
      arcadeState.leaderboard.list = data.leaderboard || [];
      arcadeState.leaderboard.userStats = data.user_stats || null;

      if (data.user_best) {
        arcadeState.game.highScore = data.user_best.score;
        updateArcadeHeaderAndStats();
      }

      renderArcadeLeaderboard(data);
    } else {
      if (tbody) tbody.innerHTML = `<tr><td colspan="7" style="text-align: center; color: var(--danger); padding: 20px;">Failed to load scores (${resp.status})</td></tr>`;
    }
  } catch (e) {
    if (tbody) tbody.innerHTML = `<tr><td colspan="7" style="text-align: center; color: var(--danger); padding: 20px;">Error connecting to leaderboard API: ${e}</td></tr>`;
  }
}

function renderArcadeLeaderboard(data) {
  const tbody = document.getElementById('arcade-leaderboard-tbody');
  if (!tbody) return;

  if (data.user_stats) {
    const bestEl = document.getElementById('arcade-sum-best');
    const rankEl = document.getElementById('arcade-sum-rank');
    const lvlEl = document.getElementById('arcade-sum-level');
    const linesEl = document.getElementById('arcade-sum-lines');
    const gamesEl = document.getElementById('arcade-sum-games');

    if (bestEl) bestEl.textContent = data.user_stats.high_score.toLocaleString();
    if (rankEl) rankEl.textContent = data.user_stats.best_rank ? `#${data.user_stats.best_rank}` : '-';
    if (lvlEl) lvlEl.textContent = data.user_stats.max_level;
    if (linesEl) linesEl.textContent = data.user_stats.total_lines.toLocaleString();
    if (gamesEl) gamesEl.textContent = data.user_stats.games_played.toLocaleString();
  }

  const scores = data.leaderboard || [];
  if (scores.length === 0) {
    tbody.innerHTML = `<tr><td colspan="7" style="text-align: center; color: var(--text-dim); padding: 24px;">No high scores recorded yet in ${data.user_stats ? arcadeState.leaderboard.mode : ''} mode. Be the first!</td></tr>`;
    return;
  }

  let html = '';
  scores.forEach((row, idx) => {
    let rankBadge = `${idx + 1}`;
    if (idx === 0) rankBadge = `<span class="arcade-rank-medal rank-gold">🥇</span>`;
    else if (idx === 1) rankBadge = `<span class="arcade-rank-medal rank-silver">🥈</span>`;
    else if (idx === 2) rankBadge = `<span class="arcade-rank-medal rank-bronze">🥉</span>`;

    const rowClass = row.is_current_user ? 'current-user-row' : '';
    const dateStr = row.created_at ? new Date(row.created_at).toLocaleDateString() : '-';
    const durationStr = row.duration_seconds > 0 ? `${Math.floor(row.duration_seconds / 60)}m ${row.duration_seconds % 60}s` : '-';

    html += `
      <tr class="${rowClass}">
        <td>${rankBadge}</td>
        <td><strong>${escapeHtml(row.player_name || row.username)}</strong> ${row.is_current_user ? '<span class="badge" style="font-size: 8px; margin-left: 4px; background: rgba(245, 158, 11, 0.2); color: var(--accent); padding: 1px 4px; border-radius: 3px;">YOU</span>' : ''}</td>
        <td style="text-align: right; font-weight: 800; color: var(--accent);">${row.score.toLocaleString()}</td>
        <td style="text-align: right;">${row.level}</td>
        <td style="text-align: right;">${row.lines_cleared}</td>
        <td style="text-align: right; color: var(--text-dim); font-size: 10px;">${durationStr}</td>
        <td style="text-align: right; color: var(--text-dim); font-size: 10px;">${dateStr}</td>
      </tr>
    `;
  });

  tbody.innerHTML = html;
}

function handleArcadeScopeChange(scope) {
  arcadeState.leaderboard.scope = scope;
  loadArcadeLeaderboard(arcadeState.leaderboard.mode);
}

function refreshArcadeLeaderboard() {
  loadArcadeLeaderboard(arcadeState.leaderboard.mode);
}

function saveArcadePlayerAlias(alias) {
  const clean = alias.trim();
  arcadeState.playerAlias = clean;
  localStorage.setItem('cd_arcade_alias', clean);
  localStorage.setItem('cd_tetradog_alias', clean);
}

async function submitArcadeScore(score, lines, level, duration, mode) {
  if (score <= 0) return;

  const playerAlias = arcadeState.playerAlias || '';
  const token = getAuthToken();

  try {
    const headers = { 'Content-Type': 'application/json' };
    if (token) headers['Authorization'] = `Bearer ${token}`;

    const resp = await fetch('/api/chewtoys/tetradog/scores', {
      method: 'POST',
      headers,
      body: JSON.stringify({
        score,
        lines_cleared: lines,
        level,
        duration_seconds: duration,
        mode,
        player_name: playerAlias
      })
    });

    if (resp.ok) {
      const result = await resp.json();
      const rankBanner = document.getElementById('arcade-rank-banner');
      const rankText = document.getElementById('arcade-rank-text');

      if (rankBanner && rankText) {
        if (result.is_global_high_score) {
          rankText.textContent = `👑 NEW #1 ALL-TIME HIGH SCORE! (${score.toLocaleString()})`;
          rankBanner.style.display = 'block';
        } else if (result.is_personal_best) {
          rankText.textContent = `⭐ Personal Best! Achieved Rank #${result.rank}`;
          rankBanner.style.display = 'block';
        } else {
          rankText.textContent = `Achieved Rank #${result.rank} on Leaderboard`;
          rankBanner.style.display = 'block';
        }
      }

      if (score > arcadeState.game.highScore) {
        arcadeState.game.highScore = score;
        updateArcadeHeaderAndStats();
      }
    }
  } catch (e) {
    console.debug('Failed to submit Arcade score:', e);
  }
}

async function promptClearArcadeScores() {
  if (!confirm('Are you sure you want to reset your high scores?')) return;

  const token = getAuthToken();
  try {
    const headers = {};
    if (token) headers['Authorization'] = `Bearer ${token}`;

    const resp = await fetch('/api/tools/tetradog/scores', {
      method: 'DELETE',
      headers
    });

    if (resp.ok) {
      arcadeState.game.highScore = 0;
      updateArcadeHeaderAndStats();
      loadArcadeLeaderboard(arcadeState.leaderboard.mode);
    }
  } catch (e) {
    console.debug('Error clearing scores:', e);
  }
}

// ---------------- INITIALIZATION & HOST LIFECYCLE ----------------
function bootArcade() {
  if (arcadeState.initialized) return;
  arcadeState.initialized = true;
  applyArcadeTheme();
  applyViewportAndControls();
  initArcadeInput();
  initArcadeGame();

  // Auto-start immediately on load just like original Tetra!
  startArcadeGame();

  // Load scores in background
  loadArcadeLeaderboard(arcadeState.game.mode, false);

  // Focus arcade app container for immediate keyboard input
  try {
    window.focus();
    const appWin = document.getElementById('arcade-app-window');
    if (appWin) appWin.focus();
  } catch (_) {}
}

// Support both instant execution (if DOM ready) and DOMContentLoaded event
if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', bootArcade);
} else {
  bootArcade();
}

// Host context bridge (Brum iframe postMessage listener)
window.addEventListener('message', (event) => {
  if (!event.data || typeof event.data !== 'object') return;

  // 1. Forwarded keyboard input from Brum parent window
  if (event.data.type === 'BRUM_KEY') {
    if (event.data.keyType === 'keydown') {
      handleArcadeKeyDown(event.data.key);
    } else if (event.data.keyType === 'keyup') {
      handleArcadeKeyUp(event.data.key);
    }
    return;
  }

  // 2. Brum initialization & viewport context
  if (event.data.type === 'BRUM_READY' || event.data.type === 'BRUM_CONTEXT') {
    const ctx = event.data.context || {};
    if (ctx.viewport) {
      arcadeState.viewport = ctx.viewport;
    }
    if (ctx.isTouch !== undefined) {
      arcadeState.isTouch = ctx.isTouch;
    }
    if (ctx.isDocked !== undefined) {
      arcadeState.isDocked = ctx.isDocked;
      const win = document.getElementById('arcade-app-window');
      if (win) win.classList.toggle('docked-mode', !!ctx.isDocked);
    }
    if (ctx.theme && !localStorage.getItem('cd_arcade_theme')) {
      const themeMap = {
        'amber-charcoal': 'amber',
        'charcoal': 'amber',
        'zink': 'amber',
        'amber-zink': 'amber',
        'gameboy': 'gameboy',
        'nes': 'nes',
        'cyberpunk': 'cyberpunk'
      };
      const matched = themeMap[ctx.theme] || 'amber';
      if (arcadeState.theme !== matched) {
        setArcadeTheme(matched);
      }
    }
    applyViewportAndControls();
    try {
      window.focus();
      const appWin = document.getElementById('arcade-app-window');
      if (appWin) appWin.focus();
    } catch (_) {}
  }
});
