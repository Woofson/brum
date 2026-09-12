(function() {
  'use strict';

  let currentContext = null;
  let sourceSizeBytes = 0;
  let detectedParts = [];

  function formatBytes(bytes) {
    if (!bytes || bytes <= 0) return '0 B';
    const k = 1024;
    const sizes = ['B', 'KB', 'MB', 'GB', 'TB', 'PB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    if (i < 0) return '0 B';
    const val = (bytes / Math.pow(k, i)).toFixed(i === 0 ? 0 : 2);
    return `${val} ${sizes[i] || 'B'}`;
  }

  function getBasename(path) {
    if (!path) return '';
    const clean = String(path).replace(/[\\/]+$/, '');
    return clean.split(/[\\/]/).filter(Boolean).pop() || clean;
  }

  function getParentDir(path) {
    if (!path) return '/';
    const idx = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'));
    return idx > 0 ? path.substring(0, idx) : '/';
  }

  // 1. Tab Switching
  window.switchTab = function(tabName) {
    const splitBtn = document.getElementById('tab-split-btn');
    const combineBtn = document.getElementById('tab-combine-btn');
    const splitView = document.getElementById('view-split');
    const combineView = document.getElementById('view-combine');

    if (tabName === 'split') {
      splitBtn?.classList.add('active');
      combineBtn?.classList.remove('active');
      splitView?.classList.add('active');
      combineView?.classList.remove('active');
    } else {
      splitBtn?.classList.remove('active');
      combineBtn?.classList.add('active');
      splitView?.classList.remove('active');
      combineView?.classList.add('active');
    }
  };

  // 2. Preset and Calculation Handlers
  window.handlePresetChange = function(val) {
    const customInput = document.getElementById('split-custom-size');
    if (customInput) {
      customInput.style.display = val === 'custom' ? 'block' : 'none';
      if (val === 'custom') customInput.focus();
    }
    updateCalculations();
  };

  window.updateCalculations = function() {
    const preset = document.getElementById('split-preset-select')?.value;
    const custom = document.getElementById('split-custom-size')?.value;
    const srcPath = document.getElementById('split-source-path')?.value;
    const partCountEl = document.getElementById('calc-part-count');
    const patternEl = document.getElementById('calc-pattern-preview');

    const chunkMb = preset === 'custom' ? parseInt(custom, 10) : parseInt(preset, 10);
    if (!chunkMb || chunkMb < 1) {
      if (partCountEl) partCountEl.textContent = 'Invalid chunk size';
      return;
    }

    const chunkBytes = chunkMb * 1024 * 1024;
    const estimatedParts = Math.max(1, Math.ceil((sourceSizeBytes || 1) / chunkBytes));

    if (partCountEl) {
      partCountEl.textContent = `${estimatedParts} part${estimatedParts > 1 ? 's' : ''} (~${chunkMb} MB each)`;
    }

    if (patternEl) {
      const fname = getBasename(srcPath) || 'file.ext';
      const padNum = String(estimatedParts).padStart(3, '0');
      patternEl.textContent = `${fname}.001 ... ${fname}.${padNum}`;
    }
  };

  // 3. Selection & Brum Host Bridge Integration
  window.useActiveSelectionForSplit = async function() {
    if (window.Brum && window.Brum.fs) {
      try {
        const files = await window.Brum.fs.getSelectedFiles();
        if (files && files.length > 0) {
          loadSourceFileForSplit(files[0]);
        }
      } catch (_) {}
    }
  };

  async function loadSourceFileForSplit(filePath) {
    if (!filePath) return;
    const srcInput = document.getElementById('split-source-path');
    const destInput = document.getElementById('split-dest-dir');
    const sizeBadge = document.getElementById('split-source-size-badge');

    if (srcInput) srcInput.value = filePath;
    if (destInput) destInput.value = getParentDir(filePath);

    // Query file stats
    try {
      if (window.Brum && window.Brum.fs) {
        const parent = getParentDir(filePath);
        const listData = await window.Brum.fs.listDir(parent);
        const fname = getBasename(filePath);
        const item = (listData.items || []).find(it => it.name === fname);
        if (item) {
          sourceSizeBytes = item.size || 0;
          if (sizeBadge) sizeBadge.textContent = formatBytes(sourceSizeBytes);
          updateCalculations();
          return;
        }
      }
    } catch (_) {}

    sourceSizeBytes = 0;
    if (sizeBadge) sizeBadge.textContent = 'Ready';
    updateCalculations();
  }

  // 4. Combine Mode Setup & Scanning
  window.scanCurrentPaneForParts = async function() {
    if (!window.Brum || !window.Brum.fs) return;
    try {
      const activePath = await window.Brum.fs.getActivePath();
      const listData = await window.Brum.fs.listDir(activePath);
      const items = listData.items || [];

      // Filter items matching .001, .002 or .part1, .part2
      const partFiles = items
        .filter(it => !it.is_dir && (/\.\d{3}$/.test(it.name) || /\.part\d+/i.test(it.name)))
        .map(it => `${activePath.replace(/\/+$/, '')}/${it.name}`)
        .sort();

      if (partFiles.length > 0) {
        setCombineParts(partFiles);
        if (window.Brum.ui) window.Brum.ui.notify(`Found ${partFiles.length} multi-part chunks`, { type: 'info' });
      } else {
        if (window.Brum.ui) window.Brum.ui.notify('No multi-part files (.001, .part1) found in active directory', { type: 'warning' });
      }
    } catch (e) {
      console.error('Scan error:', e);
    }
  };

  function setCombineParts(parts) {
    detectedParts = [...parts].sort();
    const partsListEl = document.getElementById('combine-parts-list');
    const destInput = document.getElementById('combine-dest-path');
    const shaInput = document.getElementById('combine-expected-sha');

    if (partsListEl) {
      if (detectedParts.length === 0) {
        partsListEl.innerHTML = '<div style="color: var(--text-dim); text-align: center; padding: 12px;">No parts loaded. Select files in Brum or click Scan.</div>';
      } else {
        partsListEl.innerHTML = detectedParts.map((p, idx) => `
          <div class="part-item">
            <span style="color: var(--accent); font-weight: 700;">#${idx + 1}</span>
            <span style="overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">${escapeHtml(getBasename(p))}</span>
          </div>
        `).join('');
      }
    }

    if (detectedParts.length > 0 && destInput && !destInput.value) {
      const first = detectedParts[0];
      const cleaned = first.replace(/\.\d{3}$/, '').replace(/\.part\d+.*$/i, '');
      destInput.value = cleaned;
    }

    // Try auto-loading .sha256 if present
    if (detectedParts.length > 0 && shaInput && !shaInput.value) {
      checkAndLoadShaManifest(detectedParts[0]);
    }
  }

  async function checkAndLoadShaManifest(partPath) {
    try {
      const parent = getParentDir(partPath);
      const firstPartName = getBasename(partPath);
      const baseName = firstPartName.replace(/\.\d{3}$/, '').replace(/\.part\d+.*$/i, '');
      const manifestPath = `${parent}/${baseName}.sha256`;

      if (window.Brum && window.Brum.fs) {
        const content = await window.Brum.fs.readFile(manifestPath);
        if (content && typeof content === 'string') {
          const match = content.match(/[a-fA-F0-9]{64}/);
          if (match) {
            const shaInput = document.getElementById('combine-expected-sha');
            if (shaInput) shaInput.value = match[0].toLowerCase();
          }
        }
      }
    } catch (_) {}
  }

  function escapeHtml(str) {
    return String(str || '')
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;');
  }

  // 5. Execution: Run File Split
  window.runFileSplit = async function() {
    const srcPath = document.getElementById('split-source-path')?.value?.trim();
    const preset = document.getElementById('split-preset-select')?.value;
    const custom = document.getElementById('split-custom-size')?.value;
    const destDir = document.getElementById('split-dest-dir')?.value?.trim();
    const genChk = document.getElementById('split-generate-checksum')?.checked ?? true;
    const progBox = document.getElementById('split-progress-box');
    const statusTxt = document.getElementById('split-status-text');
    const resultBanner = document.getElementById('split-result-banner');
    const btn = document.getElementById('btn-split-action');

    if (!srcPath) {
      if (window.Brum?.ui) window.Brum.ui.notify('Please select a source file to split', { type: 'warning' });
      return;
    }

    const chunkMb = preset === 'custom' ? parseInt(custom, 10) : parseInt(preset, 10);
    if (!chunkMb || chunkMb < 1) {
      if (window.Brum?.ui) window.Brum.ui.notify('Please enter a valid chunk size', { type: 'warning' });
      return;
    }

    if (progBox) progBox.style.display = 'flex';
    if (resultBanner) resultBanner.style.display = 'none';
    if (statusTxt) statusTxt.innerHTML = `<svg class="spinner" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M21 12a9 9 0 1 1-6.219-8.56"/></svg> Splitting into ${chunkMb} MB chunks...`;
    if (btn) btn.disabled = true;

    try {
      const resp = await fetch('/api/tools/split', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          source_path: srcPath,
          chunk_size_mb: chunkMb,
          dest_dir: destDir || null,
          generate_checksum: genChk
        })
      });

      if (resp.ok) {
        const res = await resp.json();
        if (resultBanner) {
          resultBanner.className = 'result-banner success';
          resultBanner.innerHTML = `
            <strong>✔ Split Successful!</strong><br>
            Created <strong>${res.chunk_count} parts</strong> (${formatBytes(res.total_size_bytes)} total).<br>
            <span style="font-family: var(--font-mono); font-size: 10.5px;">SHA-256: ${escapeHtml(res.sha256)}</span>
          `;
        }
        if (window.Brum?.ui) {
          window.Brum.ui.notify(`Split completed: ${res.chunk_count} parts created`, { type: 'success' });
        }
      } else {
        const err = await resp.text();
        if (resultBanner) {
          resultBanner.className = 'result-banner error';
          resultBanner.textContent = `Split failed: ${err}`;
        }
      }
    } catch (err) {
      if (resultBanner) {
        resultBanner.className = 'result-banner error';
        resultBanner.textContent = `Network error: ${err.message || err}`;
      }
    } finally {
      if (progBox) progBox.style.display = 'none';
      if (btn) btn.disabled = false;
    }
  };

  // 6. Execution: Run File Combine
  window.runFileCombine = async function() {
    const destPath = document.getElementById('combine-dest-path')?.value?.trim();
    const expectedSha = document.getElementById('combine-expected-sha')?.value?.trim();
    const progBox = document.getElementById('combine-progress-box');
    const resultBanner = document.getElementById('combine-result-banner');
    const btn = document.getElementById('btn-combine-action');

    if (!destPath) {
      if (window.Brum?.ui) window.Brum.ui.notify('Please specify output destination path', { type: 'warning' });
      return;
    }

    if (detectedParts.length === 0) {
      if (window.Brum?.ui) window.Brum.ui.notify('No part files selected to combine', { type: 'warning' });
      return;
    }

    if (progBox) progBox.style.display = 'flex';
    if (resultBanner) resultBanner.style.display = 'none';
    if (btn) btn.disabled = true;

    try {
      const resp = await fetch('/api/tools/combine', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          parts: detectedParts,
          dest_path: destPath,
          expected_sha256: expectedSha || null
        })
      });

      if (resp.ok) {
        const res = await resp.json();
        const verifiedTag = res.is_verified
          ? '<span style="color: var(--success); font-weight: 700;">✔ SHA-256 Checksum Verified Matching!</span>'
          : (expectedSha ? '<span style="color: var(--danger); font-weight: 700;">⚠️ Checksum Mismatch</span>' : '');

        if (resultBanner) {
          resultBanner.className = 'result-banner success';
          resultBanner.innerHTML = `
            <strong>✔ Files Combined Successfully!</strong><br>
            Joined <strong>${res.parts_joined} parts</strong> into <code>${escapeHtml(getBasename(res.dest_path))}</code> (${formatBytes(res.total_bytes_written)}).<br>
            ${verifiedTag ? verifiedTag + '<br>' : ''}
            <span style="font-family: var(--font-mono); font-size: 10.5px;">SHA-256: ${escapeHtml(res.sha256)}</span>
          `;
        }
        if (window.Brum?.ui) {
          window.Brum.ui.notify(`Joined ${res.parts_joined} parts (${formatBytes(res.total_bytes_written)})`, { type: 'success' });
        }
      } else {
        const err = await resp.text();
        if (resultBanner) {
          resultBanner.className = 'result-banner error';
          resultBanner.textContent = `Combine failed: ${err}`;
        }
      }
    } catch (err) {
      if (resultBanner) {
        resultBanner.className = 'result-banner error';
        resultBanner.textContent = `Network error: ${err.message || err}`;
      }
    } finally {
      if (progBox) progBox.style.display = 'none';
      if (btn) btn.disabled = false;
    }
  };

  // 7. Initialization & Context Dispatch
  function init(context) {
    currentContext = context || {};
    const selected = currentContext.selectedFiles || [];

    if (selected.length > 0) {
      const isMultiPart = selected.some(f => /\.\d{3}$/.test(f) || /\.part\d+/i.test(f));
      if (isMultiPart) {
        window.switchTab('combine');
        setCombineParts(selected);
      } else {
        window.switchTab('split');
        loadSourceFileForSplit(selected[0]);
      }
    } else {
      const activePath = currentContext.activePath || '/';
      const destInput = document.getElementById('split-dest-dir');
      if (destInput && !destInput.value) destInput.value = activePath;
    }
  }

  // Hook into Brum SDK or window message
  if (window.Brum && typeof window.Brum.onReady === 'function') {
    window.Brum.onReady(init);
  }

  window.addEventListener('message', (e) => {
    if (e.data && e.data.type === 'BRUM_READY') {
      init(e.data.context);
    }
  });

  document.addEventListener('DOMContentLoaded', () => {
    updateCalculations();
  });
})();
