(function () {
  'use strict';

  // Saved data lives in Brum: Chewtoys run sandboxed, without localStorage of their own.
  // Brum sends the saved values with BRUM_READY (context.storage); changes go back over the bridge.
  function createChewtoyStore() {
    let data = {};
    let loaded = false;
    let reqId = 0;
    const onLoad = [];
    const send = (action, payload) => {
      try {
        window.parent.postMessage({ type: 'BRUM_REQ', reqId: 'store-' + (++reqId), action, payload }, '*');
      } catch (_) {}
    };
    window.addEventListener('message', (e) => {
      if (e.source !== window.parent || !e.data || typeof e.data !== 'object') return;
      if ((e.data.type === 'BRUM_READY' || e.data.type === 'BRUM_CONTEXT_UPDATE') && !loaded) {
        const saved = e.data.context && e.data.context.storage;
        if (saved && typeof saved === 'object') data = Object.assign({}, saved);
        loaded = true;
        onLoad.splice(0).forEach((cb) => { try { cb(); } catch (err) { console.error(err); } });
      }
    });
    return {
      get: (key) => (Object.prototype.hasOwnProperty.call(data, key) ? data[key] : null),
      set: (key, value) => { data[key] = String(value); send('storage.set', { key, value: String(value) }); },
      remove: (key) => { delete data[key]; send('storage.remove', { key }); },
      // Runs once the saved values have arrived (right away if they already have)
      onLoad: (cb) => { if (loaded) cb(); else onLoad.push(cb); },
    };
  }

  const store = createChewtoyStore();

  let calcExpr = '';
  let calcCurrentVal = '0';
  let calcHasEvaluated = false;
  let calcHistory = [];
  store.onLoad(() => {
    try { calcHistory = JSON.parse(store.get('history') || '[]'); } catch (_) { calcHistory = []; }
    renderHistory();
  });

  // DOM Elements
  const exprDisplay = document.getElementById('calc-expr-display');
  const resultInput = document.getElementById('calc-display-input');
  const convSize = document.getElementById('calc-conv-size');
  const convHex = document.getElementById('calc-conv-hex');
  const convOct = document.getElementById('calc-conv-oct');
  const convBin = document.getElementById('calc-conv-bin');
  const historyDrawer = document.getElementById('calc-history-drawer');
  const historyList = document.getElementById('calc-history-list');
  const btnHistoryToggle = document.getElementById('btn-history-toggle');
  const btnClearHistory = document.getElementById('btn-clear-history');

  function formatBytes(bytes) {
    if (bytes === 0) return '0 B';
    const k = 1024;
    const sizes = ['B', 'KB', 'MB', 'GB', 'TB', 'PB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    if (i < 0) return '0 B';
    const val = (bytes / Math.pow(k, i)).toFixed(i === 0 ? 0 : 2);
    return `${val} ${sizes[i] || 'B'}`;
  }

  function updateDisplay() {
    if (exprDisplay) exprDisplay.textContent = calcExpr || '\u00A0';
    if (resultInput) resultInput.value = calcCurrentVal;

    const num = parseFloat(calcCurrentVal) || 0;

    // 1. File Size
    if (convSize) convSize.textContent = formatBytes(Math.abs(num));

    // 2. Hexadecimal
    if (convHex) {
      if (!isNaN(num) && Math.abs(num) < 0x1000000000000) {
        const intVal = Math.floor(Math.abs(num));
        convHex.textContent = (num < 0 ? '-' : '') + '0x' + intVal.toString(16).toUpperCase();
      } else {
        convHex.textContent = '-';
      }
    }

    // 3. Octal
    if (convOct) {
      if (!isNaN(num) && Math.abs(num) < 0x1000000) {
        const intVal = Math.floor(Math.abs(num));
        convOct.textContent = (num < 0 ? '-' : '') + '0' + intVal.toString(8);
      } else {
        convOct.textContent = '-';
      }
    }

    // 4. Binary
    if (convBin) {
      if (!isNaN(num) && Math.abs(num) < 0x10000) {
        const intVal = Math.floor(Math.abs(num));
        convBin.textContent = (num < 0 ? '-' : '') + '0b' + intVal.toString(2);
      } else {
        convBin.textContent = '-';
      }
    }
  }

  function appendNum(num) {
    if (calcHasEvaluated) {
      calcCurrentVal = num;
      calcExpr = '';
      calcHasEvaluated = false;
    } else if (calcCurrentVal === '0') {
      calcCurrentVal = num;
    } else {
      calcCurrentVal += num;
    }
    updateDisplay();
  }

  function appendDot() {
    if (calcHasEvaluated) {
      calcCurrentVal = '0.';
      calcExpr = '';
      calcHasEvaluated = false;
    } else if (!calcCurrentVal.includes('.')) {
      calcCurrentVal += '.';
    }
    updateDisplay();
  }

  function appendOp(op) {
    if (calcHasEvaluated) {
      calcExpr = calcCurrentVal + ' ' + op + ' ';
      calcCurrentVal = '0';
      calcHasEvaluated = false;
    } else if (calcExpr && calcCurrentVal === '0') {
      calcExpr = calcExpr.trim().replace(/[\+\-\*\/\%\^]$/, op) + ' ';
    } else {
      calcExpr += calcCurrentVal + ' ' + op + ' ';
      calcCurrentVal = '0';
    }
    updateDisplay();
  }

  function appendChar(char) {
    if (calcHasEvaluated) {
      calcExpr = '';
      calcHasEvaluated = false;
    }
    if (calcCurrentVal === '0') calcCurrentVal = char;
    else calcCurrentVal += char;
    updateDisplay();
  }

  function appendUnit(unit) {
    let multiplier = 1;
    if (unit === 'KB') multiplier = 1024;
    else if (unit === 'MB') multiplier = 1024 * 1024;
    else if (unit === 'GB') multiplier = 1024 * 1024 * 1024;
    else if (unit === 'TB') multiplier = 1024 * 1024 * 1024 * 1024;

    const currentNum = parseFloat(calcCurrentVal) || 0;
    calcCurrentVal = (currentNum * multiplier).toString();
    calcHasEvaluated = true;
    updateDisplay();
  }

  function appendFunc(func) {
    if (func === 'sqrt(') {
      const num = parseFloat(calcCurrentVal) || 0;
      if (num >= 0) {
        calcCurrentVal = Math.sqrt(num).toString();
        calcHasEvaluated = true;
        updateDisplay();
      }
    }
  }

  function clearAll() {
    calcExpr = '';
    calcCurrentVal = '0';
    calcHasEvaluated = false;
    updateDisplay();
  }

  function clearEntry() {
    calcCurrentVal = '0';
    updateDisplay();
  }

  function backspace() {
    if (calcHasEvaluated) {
      clearAll();
      return;
    }
    if (calcCurrentVal.length > 1) {
      calcCurrentVal = calcCurrentVal.slice(0, -1);
    } else {
      calcCurrentVal = '0';
    }
    updateDisplay();
  }

  function toggleSign() {
    if (calcCurrentVal === '0') return;
    if (calcCurrentVal.startsWith('-')) {
      calcCurrentVal = calcCurrentVal.substring(1);
    } else {
      calcCurrentVal = '-' + calcCurrentVal;
    }
    updateDisplay();
  }

  function appendPi() {
    calcCurrentVal = Math.PI.toString();
    calcHasEvaluated = true;
    updateDisplay();
  }

  function evaluate() {
    const fullExpr = (calcExpr + calcCurrentVal).trim();
    if (!fullExpr) return;

    try {
      let sanitized = fullExpr
        .replace(/×/g, '*')
        .replace(/÷/g, '/')
        .replace(/−/g, '-')
        .replace(/\^/g, '**');

      if (!/^[0-9+\-*/().%*^ eE_]+$/.test(sanitized)) {
        throw new Error('Invalid characters');
      }

      // Safe evaluation using Function
      const result = Function('"use strict"; return (' + sanitized + ')')();
      const formattedResult = Number.isFinite(result) ? (Math.round(result * 1e12) / 1e12).toString() : 'Error';

      // Save to History Tape
      if (formattedResult !== 'Error') {
        calcHistory.unshift({ expr: fullExpr, val: formattedResult, time: Date.now() });
        if (calcHistory.length > 30) calcHistory.pop();
        store.set('history', JSON.stringify(calcHistory));
        renderHistory();
      }

      calcExpr = fullExpr + ' =';
      calcCurrentVal = formattedResult;
      calcHasEvaluated = true;
      updateDisplay();
    } catch (e) {
      calcCurrentVal = 'Error';
      calcHasEvaluated = true;
      updateDisplay();
    }
  }

  function renderHistory() {
    if (!historyList) return;
    if (calcHistory.length === 0) {
      historyList.innerHTML = '<div class="calc-history-empty">No calculations yet</div>';
      return;
    }

    historyList.innerHTML = calcHistory.map(item => `
      <div class="calc-history-item" data-val="${item.val}">
        <span class="calc-history-expr">${item.expr}</span>
        <span class="calc-history-val">= ${item.val}</span>
      </div>
    `).join('');

    historyList.querySelectorAll('.calc-history-item').forEach(el => {
      el.addEventListener('click', () => {
        calcCurrentVal = el.getAttribute('data-val');
        calcHasEvaluated = true;
        updateDisplay();
      });
    });
  }

  // Requests to Brum over the bridge (the sandbox blocks direct access to Brum's page)
  let hostReqId = 0;
  const pendingHost = new Map();
  window.addEventListener('message', (e) => {
    if (e.source !== window.parent || !e.data || e.data.type !== 'BRUM_RES') return;
    const pending = pendingHost.get(e.data.reqId);
    if (!pending) return;
    pendingHost.delete(e.data.reqId);
    if (e.data.error) pending.reject(new Error(e.data.error));
    else pending.resolve(e.data.result);
  });
  function hostRequest(action, payload) {
    return new Promise((resolve, reject) => {
      const reqId = 'calc-' + (++hostReqId);
      pendingHost.set(reqId, { resolve, reject });
      window.parent.postMessage({ type: 'BRUM_REQ', reqId, action, payload }, '*');
    });
  }

  function copyResult() {
    const text = resultInput ? resultInput.value : calcCurrentVal;
    hostRequest('clipboard.writeText', { text })
      .then(() => notify('Copied ' + text + ' to clipboard'))
      .catch((err) => notify('Could not copy: ' + err.message));
  }

  function notify(msg) {
    hostRequest('ui.notify', { message: msg, opts: { type: 'info' } }).catch(() => {});
  }

  // Event Listeners
  document.querySelectorAll('[data-num]').forEach(btn => {
    btn.addEventListener('click', () => appendNum(btn.getAttribute('data-num')));
  });

  document.querySelectorAll('[data-op]').forEach(btn => {
    btn.addEventListener('click', () => appendOp(btn.getAttribute('data-op')));
  });

  document.querySelectorAll('[data-char]').forEach(btn => {
    btn.addEventListener('click', () => appendChar(btn.getAttribute('data-char')));
  });

  document.querySelectorAll('[data-unit]').forEach(btn => {
    btn.addEventListener('click', () => appendUnit(btn.getAttribute('data-unit')));
  });

  document.querySelectorAll('[data-func]').forEach(btn => {
    btn.addEventListener('click', () => appendFunc(btn.getAttribute('data-func')));
  });

  document.getElementById('btn-dot')?.addEventListener('click', appendDot);
  document.getElementById('btn-clear-all')?.addEventListener('click', clearAll);
  document.getElementById('btn-clear-entry')?.addEventListener('click', clearEntry);
  document.getElementById('btn-backspace')?.addEventListener('click', backspace);
  document.getElementById('btn-toggle-sign')?.addEventListener('click', toggleSign);
  document.getElementById('btn-pi')?.addEventListener('click', appendPi);
  document.getElementById('btn-equals')?.addEventListener('click', evaluate);
  document.getElementById('btn-copy')?.addEventListener('click', copyResult);

  btnHistoryToggle?.addEventListener('click', () => {
    if (!historyDrawer) return;
    const isHidden = historyDrawer.style.display === 'none';
    historyDrawer.style.display = isHidden ? 'flex' : 'none';
    if (isHidden) renderHistory();
  });

  btnClearHistory?.addEventListener('click', () => {
    calcHistory = [];
    store.remove('history');
    renderHistory();
  });

  // Global Keyboard listener inside iframe
  window.addEventListener('keydown', (e) => {
    if (e.key >= '0' && e.key <= '9') {
      appendNum(e.key);
    } else if (['+', '-', '*', '/'].includes(e.key)) {
      appendOp(e.key);
    } else if (e.key === '.') {
      appendDot();
    } else if (e.key === 'Enter' || e.key === '=') {
      e.preventDefault();
      evaluate();
    } else if (e.key === 'Backspace') {
      backspace();
    } else if (e.key === 'Escape') {
      clearAll();
    } else if (e.key === '(' || e.key === ')') {
      appendChar(e.key);
    } else if (e.key === '%') {
      appendOp('%');
    }
  });

  // Brum SDK Integration
  if (window.Brum && typeof window.Brum.onReady === 'function') {
    window.Brum.onReady((context) => {
      if (context.theme) {
        document.documentElement.setAttribute('data-theme', context.theme);
      }
    });
  }

  // Initial render
  updateDisplay();
  renderHistory();
})();
