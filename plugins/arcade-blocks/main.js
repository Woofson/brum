(function () {
  'use strict';

  // 1. Tetromino Shapes & Color Palettes
  const SHAPES = {
    I: [[0,0,0,0], [1,1,1,1], [0,0,0,0], [0,0,0,0]],
    J: [[1,0,0], [1,1,1], [0,0,0]],
    L: [[0,0,1], [1,1,1], [0,0,0]],
    O: [[1,1], [1,1]],
    S: [[0,1,1], [1,1,0], [0,0,0]],
    T: [[0,1,0], [1,1,1], [0,0,0]],
    Z: [[1,1,0], [0,1,1], [0,0,0]]
  };

  const COLORS = {
    I: '#06b6d4', // Cyan
    J: '#3b82f6', // Blue
    L: '#f97316', // Orange
    O: '#eab308', // Yellow
    S: '#22c55e', // Green
    T: '#a855f7', // Purple
    Z: '#ef4444'  // Red
  };

  const COLS = 10;
  const ROWS = 20;
  const BLOCK_SIZE = 20;

  // DOM Canvases
  const mainCanvas = document.getElementById('main-canvas');
  const mainCtx = mainCanvas.getContext('2d');
  const holdCanvas = document.getElementById('hold-canvas');
  const holdCtx = holdCanvas.getContext('2d');
  const nextCanvas = document.getElementById('next-canvas');
  const nextCtx = nextCanvas.getContext('2d');

  // DOM HUD
  const hudScore = document.getElementById('hud-score');
  const hudLevel = document.getElementById('hud-level');
  const hudLines = document.getElementById('hud-lines');
  const statTime = document.getElementById('stat-time');
  const statCombo = document.getElementById('stat-combo');
  const statB2B = document.getElementById('stat-b2b');
  const overlay = document.getElementById('game-overlay');
  const overlayTitle = document.getElementById('overlay-title');
  const overlaySub = document.getElementById('overlay-sub');
  const btnPlayPause = document.getElementById('btn-play-pause');

  // Game State
  let board = Array.from({ length: ROWS }, () => Array(COLS).fill(0));
  let currentPiece = null;
  let holdPiece = null;
  let canHold = true;
  let bag = [];
  let nextPieces = [];
  let score = 0;
  let lines = 0;
  let level = 1;
  let combo = -1;
  let b2b = 0;
  let startTime = 0;
  let elapsedTime = 0;
  let isPlaying = false;
  let isPaused = false;
  let dropCounter = 0;
  let lastTime = 0;
  let animationId = null;
  let audioCtx = null;
  let soundEnabled = true;
  let ghostEnabled = true;

  // High Scores
  let highScores = JSON.parse(localStorage.getItem('brum_arcade_scores') || '[]');

  // Sound Synth via Web Audio API
  function playBeep(freq, duration = 0.08, type = 'sine') {
    if (!soundEnabled) return;
    try {
      if (!audioCtx) audioCtx = new (window.AudioContext || window.webkitAudioContext)();
      if (audioCtx.state === 'suspended') audioCtx.resume();
      const osc = audioCtx.createOscillator();
      const gain = audioCtx.createGain();
      osc.type = type;
      osc.frequency.setValueAtTime(freq, audioCtx.currentTime);
      gain.gain.setValueAtTime(0.15, audioCtx.currentTime);
      gain.gain.exponentialRampToValueAtTime(0.001, audioCtx.currentTime + duration);
      osc.connect(gain);
      gain.connect(audioCtx.destination);
      osc.start();
      osc.stop(audioCtx.currentTime + duration);
    } catch (e) {}
  }

  // 7-Bag Randomizer
  function refillBag() {
    const pieces = ['I', 'J', 'L', 'O', 'S', 'T', 'Z'];
    for (let i = pieces.length - 1; i > 0; i--) {
      const j = Math.floor(Math.random() * (i + 1));
      [pieces[i], pieces[j]] = [pieces[j], pieces[i]];
    }
    return pieces;
  }

  function getNextPieceType() {
    if (bag.length === 0) bag = refillBag();
    return bag.pop();
  }

  function createPiece(type) {
    const shape = SHAPES[type];
    return {
      type,
      matrix: shape.map(row => [...row]),
      x: Math.floor((COLS - shape[0].length) / 2),
      y: 0
    };
  }

  function initGame() {
    board = Array.from({ length: ROWS }, () => Array(COLS).fill(0));
    bag = refillBag();
    nextPieces = [getNextPieceType(), getNextPieceType(), getNextPieceType()];
    currentPiece = createPiece(getNextPieceType());
    holdPiece = null;
    canHold = true;
    score = 0;
    lines = 0;
    level = parseInt(document.getElementById('cfg-start-level')?.value || '1', 10);
    combo = -1;
    b2b = 0;
    startTime = Date.now();
    elapsedTime = 0;
    isPlaying = true;
    isPaused = false;
    dropCounter = 0;
    lastTime = performance.now();

    if (overlay) overlay.style.display = 'none';
    if (btnPlayPause) btnPlayPause.textContent = 'Pause';
    updateHUD();
    render();
  }

  function collide(b, piece, offset = { x: 0, y: 0 }) {
    const m = piece.matrix;
    for (let y = 0; y < m.length; ++y) {
      for (let x = 0; x < m[y].length; ++x) {
        if (m[y][x] !== 0) {
          const newX = piece.x + x + offset.x;
          const newY = piece.y + y + offset.y;
          if (newX < 0 || newX >= COLS || newY >= ROWS) return true;
          if (newY >= 0 && b[newY][newX] !== 0) return true;
        }
      }
    }
    return false;
  }

  function merge(b, piece) {
    piece.matrix.forEach((row, y) => {
      row.forEach((value, x) => {
        if (value !== 0) {
          const targetY = piece.y + y;
          const targetX = piece.x + x;
          if (targetY >= 0 && targetY < ROWS && targetX >= 0 && targetX < COLS) {
            b[targetY][targetX] = piece.type;
          }
        }
      });
    });
  }

  function rotate(matrix, dir) {
    const result = matrix.map((_, i) => matrix.map(row => row[i]));
    return dir > 0 ? result.map(row => row.reverse()) : result.reverse();
  }

  function playerRotate(dir) {
    if (!isPlaying || isPaused || !currentPiece) return;
    const origX = currentPiece.x;
    const rotated = rotate(currentPiece.matrix, dir);
    const prevMatrix = currentPiece.matrix;
    currentPiece.matrix = rotated;

    // Wall kick offsets
    let offset = 1;
    while (collide(board, currentPiece)) {
      currentPiece.x += offset;
      offset = -(offset + (offset > 0 ? 1 : -1));
      if (offset > currentPiece.matrix[0].length) {
        currentPiece.matrix = prevMatrix;
        currentPiece.x = origX;
        return;
      }
    }
    playBeep(440, 0.05, 'triangle');
    render();
  }

  function playerMove(dir) {
    if (!isPlaying || isPaused || !currentPiece) return;
    currentPiece.x += dir;
    if (collide(board, currentPiece)) {
      currentPiece.x -= dir;
    } else {
      playBeep(260, 0.03, 'square');
      render();
    }
  }

  function playerDrop() {
    if (!isPlaying || isPaused || !currentPiece) return;
    currentPiece.y++;
    if (collide(board, currentPiece)) {
      currentPiece.y--;
      lockPiece();
    }
    dropCounter = 0;
    render();
  }

  function playerHardDrop() {
    if (!isPlaying || isPaused || !currentPiece) return;
    let dropDist = 0;
    while (!collide(board, currentPiece, { x: 0, y: 1 })) {
      currentPiece.y++;
      dropDist++;
    }
    score += dropDist * 2;
    playBeep(180, 0.06, 'sawtooth');
    lockPiece();
    render();
  }

  function playerHold() {
    if (!isPlaying || isPaused || !canHold || !currentPiece) return;
    playBeep(350, 0.08, 'sine');
    if (!holdPiece) {
      holdPiece = currentPiece.type;
      currentPiece = createPiece(nextPieces.shift());
      nextPieces.push(getNextPieceType());
    } else {
      const temp = holdPiece;
      holdPiece = currentPiece.type;
      currentPiece = createPiece(temp);
    }
    canHold = false;
    render();
  }

  function lockPiece() {
    merge(board, currentPiece);
    playBeep(200, 0.05, 'triangle');
    canHold = true;
    clearLines();

    currentPiece = createPiece(nextPieces.shift());
    nextPieces.push(getNextPieceType());

    if (collide(board, currentPiece)) {
      gameOver();
    }
  }

  function clearLines() {
    let linesCleared = 0;
    for (let y = ROWS - 1; y >= 0; --y) {
      if (board[y].every(cell => cell !== 0)) {
        board.splice(y, 1);
        board.unshift(Array(COLS).fill(0));
        linesCleared++;
        y++;
      }
    }

    if (linesCleared > 0) {
      combo++;
      lines += linesCleared;
      const basePoints = [0, 100, 300, 500, 800][linesCleared] || 1000;
      const isTetris = linesCleared === 4;

      if (isTetris) {
        b2b++;
        playBeep(880, 0.25, 'sawtooth');
      } else {
        b2b = 0;
        playBeep(520 + linesCleared * 80, 0.12, 'square');
      }

      const multiplier = 1 + (b2b > 1 ? 0.5 : 0) + (combo > 0 ? combo * 0.1 : 0);
      score += Math.round(basePoints * level * multiplier);
      level = Math.floor(lines / 10) + 1;
      updateHUD();
    } else {
      combo = -1;
    }
  }

  function gameOver() {
    isPlaying = false;
    playBeep(130, 0.4, 'sawtooth');
    if (animationId) cancelAnimationFrame(animationId);

    // Save score
    if (score > 0) {
      highScores.unshift({ score, lines, level, time: Math.floor(elapsedTime), date: new Date().toLocaleDateString() });
      highScores.sort((a, b) => b.score - a.score);
      if (highScores.length > 20) highScores.pop();
      localStorage.setItem('brum_arcade_scores', JSON.stringify(highScores));
      renderLeaderboard();
    }

    if (overlay) {
      overlayTitle.textContent = 'GAME OVER';
      overlaySub.textContent = `Score: ${score.toLocaleString()} • Lines: ${lines}`;
      overlay.style.display = 'flex';
    }
    if (btnPlayPause) btnPlayPause.textContent = 'Play';
  }

  function getGhostY() {
    if (!currentPiece) return 0;
    let ghostY = currentPiece.y;
    while (!collide(board, currentPiece, { x: 0, y: ghostY - currentPiece.y + 1 })) {
      ghostY++;
    }
    return ghostY;
  }

  function drawBlock(ctx, x, y, color, isGhost = false) {
    ctx.fillStyle = isGhost ? 'rgba(255, 255, 255, 0.15)' : color;
    ctx.fillRect(x * BLOCK_SIZE, y * BLOCK_SIZE, BLOCK_SIZE, BLOCK_SIZE);
    ctx.strokeStyle = isGhost ? 'rgba(255, 255, 255, 0.3)' : 'rgba(0, 0, 0, 0.4)';
    ctx.strokeRect(x * BLOCK_SIZE, y * BLOCK_SIZE, BLOCK_SIZE, BLOCK_SIZE);
  }

  function render() {
    // 1. Main Matrix
    mainCtx.fillStyle = '#090a0f';
    mainCtx.fillRect(0, 0, mainCanvas.width, mainCanvas.height);

    // Grid lines
    mainCtx.strokeStyle = 'rgba(255, 255, 255, 0.03)';
    for (let x = 0; x < COLS; x++) {
      mainCtx.beginPath();
      mainCtx.moveTo(x * BLOCK_SIZE, 0);
      mainCtx.lineTo(x * BLOCK_SIZE, ROWS * BLOCK_SIZE);
      mainCtx.stroke();
    }
    for (let y = 0; y < ROWS; y++) {
      mainCtx.beginPath();
      mainCtx.moveTo(0, y * BLOCK_SIZE);
      mainCtx.lineTo(COLS * BLOCK_SIZE, y * BLOCK_SIZE);
      mainCtx.stroke();
    }

    // Board locked blocks
    board.forEach((row, y) => {
      row.forEach((type, x) => {
        if (type !== 0) drawBlock(mainCtx, x, y, COLORS[type]);
      });
    });

    // Ghost Piece
    if (isPlaying && ghostEnabled && currentPiece) {
      const ghostY = getGhostY();
      currentPiece.matrix.forEach((row, y) => {
        row.forEach((val, x) => {
          if (val !== 0) drawBlock(mainCtx, currentPiece.x + x, ghostY + y, COLORS[currentPiece.type], true);
        });
      });
    }

    // Active Piece
    if (isPlaying && currentPiece) {
      currentPiece.matrix.forEach((row, y) => {
        row.forEach((val, x) => {
          if (val !== 0) drawBlock(mainCtx, currentPiece.x + x, currentPiece.y + y, COLORS[currentPiece.type]);
        });
      });
    }

    // 2. Hold Canvas
    holdCtx.fillStyle = '#000';
    holdCtx.fillRect(0, 0, holdCanvas.width, holdCanvas.height);
    if (holdPiece) {
      const shape = SHAPES[holdPiece];
      const offsetX = (holdCanvas.width - shape[0].length * 16) / 2;
      const offsetY = (holdCanvas.height - shape.length * 16) / 2;
      shape.forEach((row, y) => {
        row.forEach((val, x) => {
          if (val !== 0) {
            holdCtx.fillStyle = COLORS[holdPiece];
            holdCtx.fillRect(offsetX + x * 16, offsetY + y * 16, 15, 15);
          }
        });
      });
    }

    // 3. Next Canvas (3 pieces)
    nextCtx.fillStyle = '#000';
    nextCtx.fillRect(0, 0, nextCanvas.width, nextCanvas.height);
    nextPieces.slice(0, 3).forEach((type, idx) => {
      const shape = SHAPES[type];
      const offsetX = (nextCanvas.width - shape[0].length * 14) / 2;
      const offsetY = 10 + idx * 55;
      shape.forEach((row, y) => {
        row.forEach((val, x) => {
          if (val !== 0) {
            nextCtx.fillStyle = COLORS[type];
            nextCtx.fillRect(offsetX + x * 14, offsetY + y * 14, 13, 13);
          }
        });
      });
    });
  }

  function update(time = 0) {
    if (!isPlaying || isPaused) return;

    const delta = time - lastTime;
    lastTime = time;
    dropCounter += delta;

    // Gravity speed curve
    const speed = Math.max(80, 800 - (level - 1) * 70);
    if (dropCounter > speed) {
      playerDrop();
    }

    elapsedTime = (Date.now() - startTime) / 1000;
    const mins = Math.floor(elapsedTime / 60);
    const secs = Math.floor(elapsedTime % 60).toString().padStart(2, '0');
    if (statTime) statTime.textContent = `${mins}:${secs}`;

    animationId = requestAnimationFrame(update);
  }

  function updateHUD() {
    if (hudScore) hudScore.textContent = score.toLocaleString();
    if (hudLevel) hudLevel.textContent = level;
    if (hudLines) hudLines.textContent = lines;
    if (statCombo) statCombo.textContent = Math.max(0, combo);
    if (statB2B) statB2B.textContent = b2b;
  }

  function togglePlayPause() {
    if (!isPlaying) {
      initGame();
      lastTime = performance.now();
      animationId = requestAnimationFrame(update);
      return;
    }
    isPaused = !isPaused;
    if (isPaused) {
      if (btnPlayPause) btnPlayPause.textContent = 'Resume';
      if (overlay) {
        overlayTitle.textContent = 'PAUSED';
        overlaySub.textContent = 'Press P or Resume to continue';
        overlay.style.display = 'flex';
      }
    } else {
      if (btnPlayPause) btnPlayPause.textContent = 'Pause';
      if (overlay) overlay.style.display = 'none';
      lastTime = performance.now();
      animationId = requestAnimationFrame(update);
    }
  }

  function renderLeaderboard() {
    const list = document.getElementById('leaderboard-list');
    if (!list) return;
    if (highScores.length === 0) {
      list.innerHTML = '<div class="arcade-empty">No high scores recorded yet</div>';
      return;
    }
    list.innerHTML = highScores.map((s, idx) => `
      <div class="arcade-score-row">
        <span>#${idx + 1} • Level ${s.level} (${s.lines} lines)</span>
        <span style="color: var(--accent); font-weight: 700;">${s.score.toLocaleString()} pts</span>
      </div>
    `).join('');
  }

  // Keyboard Controls
  window.addEventListener('keydown', (e) => {
    if (e.key === 'ArrowLeft') { e.preventDefault(); playerMove(-1); }
    else if (e.key === 'ArrowRight') { e.preventDefault(); playerMove(1); }
    else if (e.key === 'ArrowDown') { e.preventDefault(); playerDrop(); }
    else if (e.key === 'ArrowUp' || e.key === 'x' || e.key === 'X') { e.preventDefault(); playerRotate(1); }
    else if (e.key === 'z' || e.key === 'Z') { e.preventDefault(); playerRotate(-1); }
    else if (e.key === ' ') { e.preventDefault(); playerHardDrop(); }
    else if (e.key === 'c' || e.key === 'C' || e.key === 'Shift') { e.preventDefault(); playerHold(); }
    else if (e.key === 'p' || e.key === 'P' || e.key === 'Escape') { e.preventDefault(); togglePlayPause(); }
  });

  // Touch & UI Listeners
  document.querySelectorAll('[data-act]').forEach(btn => {
    const act = btn.getAttribute('data-act');
    btn.addEventListener('click', () => {
      if (act === 'left') playerMove(-1);
      else if (act === 'right') playerMove(1);
      else if (act === 'down') playerDrop();
      else if (act === 'hardDrop') playerHardDrop();
      else if (act === 'rotateCW') playerRotate(1);
      else if (act === 'rotateCCW') playerRotate(-1);
      else if (act === 'hold') playerHold();
    });
  });

  document.querySelectorAll('[data-view]').forEach(tab => {
    tab.addEventListener('click', () => {
      const v = tab.getAttribute('data-view');
      document.querySelectorAll('.arcade-tab').forEach(t => t.classList.toggle('active', t === tab));
      ['game', 'leaderboard', 'settings'].forEach(viewName => {
        const el = document.getElementById(`view-${viewName}`);
        if (el) el.style.display = (viewName === v) ? 'flex' : 'none';
      });
      if (v === 'leaderboard') renderLeaderboard();
    });
  });

  btnPlayPause?.addEventListener('click', togglePlayPause);
  document.getElementById('tbtn-touch-pause')?.addEventListener('click', togglePlayPause);
  document.getElementById('btn-restart')?.addEventListener('click', initGame);
  document.getElementById('btn-overlay-play')?.addEventListener('click', initGame);
  document.getElementById('btn-overlay-scores')?.addEventListener('click', () => {
    document.querySelector('[data-view="leaderboard"]')?.click();
  });
  document.getElementById('btn-clear-scores')?.addEventListener('click', () => {
    highScores = [];
    localStorage.removeItem('brum_arcade_scores');
    renderLeaderboard();
  });

  document.getElementById('cfg-sound')?.addEventListener('change', (e) => {
    soundEnabled = e.target.checked;
  });
  document.getElementById('cfg-ghost')?.addEventListener('change', (e) => {
    ghostEnabled = e.target.checked;
    render();
  });

  // Initial Start
  initGame();
  renderLeaderboard();
  lastTime = performance.now();
  animationId = requestAnimationFrame(update);
})();
