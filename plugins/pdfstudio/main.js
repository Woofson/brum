(function() {
  'use strict';

  let currentContext = null;
  let mergeFiles = [];
  let reorderPages = [];
  let currentReorderSource = '';

  // -------------------------------------------------------------------------
  // Helper Utilities
  // -------------------------------------------------------------------------
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

  function isPdfFile(path) {
    return Boolean(path && /\.pdf$/i.test(path));
  }

  function escapeHtml(str) {
    return String(str || '')
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;');
  }

  // -------------------------------------------------------------------------
  // 1. Tab Switching
  // -------------------------------------------------------------------------
  window.switchTab = function(tabName) {
    ['merge', 'split', 'reorder'].forEach(t => {
      const btn = document.getElementById(`tab-${t}-btn`);
      const view = document.getElementById(`view-${t}`);
      if (btn) btn.classList.toggle('active', t === tabName);
      if (view) view.classList.toggle('active', t === tabName);
    });
  };

  // -------------------------------------------------------------------------
  // 2. Tab 1: Merge PDFs Logic
  // -------------------------------------------------------------------------
  window.addActivePanePdfs = async function() {
    let files = [];
    if (window.Brum && window.Brum.fs) {
      try {
        files = await window.Brum.fs.getSelectedFiles();
      } catch (_) {}
    }

    if (!files || files.length === 0) {
      if (currentContext && currentContext.selectedFiles) {
        files = currentContext.selectedFiles;
      }
    }

    const pdfs = (files || []).filter(isPdfFile);
    if (pdfs.length === 0) {
      if (window.Brum?.ui) {
        window.Brum.ui.notify('No PDF files selected in active pane', { type: 'warning' });
      }
      return;
    }

    pdfs.forEach(p => {
      if (!mergeFiles.includes(p)) {
        mergeFiles.push(p);
      }
    });

    renderMergeList();

    const destInput = document.getElementById('merge-dest-path');
    if (destInput && !destInput.value && mergeFiles.length > 0) {
      const parent = getParentDir(mergeFiles[0]);
      destInput.value = `${parent.replace(/\/+$/, '')}/merged_document.pdf`;
    }
  };

  window.clearMergeList = function() {
    mergeFiles = [];
    renderMergeList();
  };

  window.moveMergeItem = function(idx, delta) {
    const target = idx + delta;
    if (target < 0 || target >= mergeFiles.length) return;
    const item = mergeFiles.splice(idx, 1)[0];
    mergeFiles.splice(target, 0, item);
    renderMergeList();
  };

  window.removeMergeItem = function(idx) {
    mergeFiles.splice(idx, 1);
    renderMergeList();
  };

  function renderMergeList() {
    const listEl = document.getElementById('merge-file-list');
    if (!listEl) return;

    if (mergeFiles.length === 0) {
      listEl.innerHTML = '<div class="empty-state">No PDF files added. Select PDFs in Brum or click Add Selected.</div>';
      return;
    }

    listEl.innerHTML = mergeFiles.map((p, idx) => `
      <div class="merge-item" data-idx="${idx}">
        <div class="merge-item-info">
          <span class="merge-item-num">${idx + 1}.</span>
          <svg class="merge-item-icon" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/><line x1="16" y1="13" x2="8" y2="13"/><line x1="16" y1="17" x2="8" y2="17"/><polyline points="10 9 9 9 8 9"/></svg>
          <span class="merge-item-name" title="${escapeHtml(p)}">${escapeHtml(getBasename(p))}</span>
        </div>
        <div class="merge-item-actions">
          <button type="button" class="btn-icon-sm" onclick="moveMergeItem(${idx}, -1)" ${idx === 0 ? 'disabled' : ''} title="Move Up">
            <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><polyline points="18 15 12 9 6 15"/></svg>
          </button>
          <button type="button" class="btn-icon-sm" onclick="moveMergeItem(${idx}, 1)" ${idx === mergeFiles.length - 1 ? 'disabled' : ''} title="Move Down">
            <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><polyline points="6 9 12 15 18 9"/></svg>
          </button>
          <button type="button" class="btn-icon-sm danger" onclick="removeMergeItem(${idx})" title="Remove">
            <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
          </button>
        </div>
      </div>
    `).join('');
  }

  window.runPdfMerge = async function() {
    const dest = document.getElementById('merge-dest-path')?.value?.trim();
    const addBookmarks = document.getElementById('merge-add-bookmarks')?.checked ?? true;
    const progBox = document.getElementById('merge-progress-box');
    const resultBanner = document.getElementById('merge-result-banner');
    const btn = document.getElementById('btn-merge-action');

    if (mergeFiles.length < 2) {
      if (window.Brum?.ui) window.Brum.ui.notify('Please add at least 2 PDF documents to merge', { type: 'warning' });
      return;
    }
    if (!dest) {
      if (window.Brum?.ui) window.Brum.ui.notify('Please enter an output destination PDF path', { type: 'warning' });
      return;
    }

    if (progBox) progBox.style.display = 'flex';
    if (resultBanner) resultBanner.style.display = 'none';
    if (btn) btn.disabled = true;

    try {
      const resp = await fetch('/api/tools/pdf/merge', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          sources: mergeFiles,
          destination: dest,
          add_bookmarks: addBookmarks
        })
      });

      if (resp.ok) {
        if (resultBanner) {
          resultBanner.className = 'result-banner success';
          resultBanner.innerHTML = `
            <strong>✔ Merge Completed!</strong><br>
            Combined <strong>${mergeFiles.length} PDF documents</strong> into <code>${escapeHtml(getBasename(dest))}</code>.<br>
            <span style="font-family: var(--font-mono); font-size: 11px;">Path: ${escapeHtml(dest)}</span>
          `;
        }
        if (window.Brum?.ui) {
          window.Brum.ui.notify(`Successfully merged ${mergeFiles.length} PDFs into ${getBasename(dest)}`, { type: 'success' });
        }
      } else {
        const err = await resp.text();
        if (resultBanner) {
          resultBanner.className = 'result-banner error';
          resultBanner.textContent = `Merge failed: ${err}`;
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

  // -------------------------------------------------------------------------
  // 3. Tab 2: Split PDF Logic
  // -------------------------------------------------------------------------
  let splitInfoTimeout = null;

  window.handleSplitSourceChange = function(val) {
    if (splitInfoTimeout) clearTimeout(splitInfoTimeout);
    splitInfoTimeout = setTimeout(() => {
      fetchPdfSplitInfo(val);
    }, 300);
  };

  window.handleSplitModeChange = function(mode) {
    const rangesGroup = document.getElementById('split-ranges-group');
    const chunkGroup = document.getElementById('split-chunk-group');
    if (rangesGroup) rangesGroup.style.display = mode === 'ranges' ? 'flex' : 'none';
    if (chunkGroup) chunkGroup.style.display = mode === 'page_count' ? 'flex' : 'none';
  };

  async function fetchPdfSplitInfo(pdfPath) {
    const badge = document.getElementById('split-info-badge');
    const rangesVal = document.getElementById('split-ranges-val');
    const destDir = document.getElementById('split-dest-dir');
    if (!badge) return;

    if (!pdfPath || !isPdfFile(pdfPath)) {
      badge.style.display = 'none';
      return;
    }

    try {
      const resp = await fetch(`/api/tools/pdf/info?path=${encodeURIComponent(pdfPath)}`);
      if (resp.ok) {
        const info = await resp.json();
        badge.style.display = 'block';
        badge.textContent = `📄 Total Pages: ${info.page_count} | Size: ${formatBytes(info.file_size_bytes)} | PDF v${info.version || '1.5'}`;
        if (rangesVal && info.page_count > 1 && (!rangesVal.value || rangesVal.value === '1-2')) {
          rangesVal.value = `1-${Math.min(2, info.page_count)}, ${Math.min(3, info.page_count)}-${info.page_count}`;
        }
        if (destDir && !destDir.value) {
          destDir.value = getParentDir(pdfPath);
        }
      } else {
        badge.style.display = 'none';
      }
    } catch (_) {
      badge.style.display = 'none';
    }
  }

  window.runPdfSplit = async function() {
    const source = document.getElementById('split-source-path')?.value?.trim();
    const destDir = document.getElementById('split-dest-dir')?.value?.trim();
    const mode = document.getElementById('split-mode-select')?.value || 'ranges';
    const ranges = document.getElementById('split-ranges-val')?.value?.trim();
    const chunkSize = parseInt(document.getElementById('split-chunk-val')?.value || '2', 10);
    const prefix = document.getElementById('split-prefix')?.value?.trim() || null;
    const progBox = document.getElementById('split-progress-box');
    const resultBanner = document.getElementById('split-result-banner');
    const btn = document.getElementById('btn-split-action');

    if (!source || !destDir) {
      if (window.Brum?.ui) window.Brum.ui.notify('Please specify source PDF and output directory', { type: 'warning' });
      return;
    }

    if (progBox) progBox.style.display = 'flex';
    if (resultBanner) resultBanner.style.display = 'none';
    if (btn) btn.disabled = true;

    try {
      const resp = await fetch('/api/tools/pdf/split', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          source: source,
          destination_dir: destDir,
          split_mode: mode,
          page_ranges: mode === 'ranges' ? ranges : null,
          chunk_size: mode === 'page_count' ? chunkSize : null,
          output_prefix: prefix
        })
      });

      if (resp.ok) {
        const res = await resp.json();
        const count = res.files ? res.files.length : 0;
        if (resultBanner) {
          resultBanner.className = 'result-banner success';
          resultBanner.innerHTML = `
            <strong>✔ PDF Split Successful!</strong><br>
            Generated <strong>${count} split files</strong> in <code>${escapeHtml(destDir)}</code>.
          `;
        }
        if (window.Brum?.ui) {
          window.Brum.ui.notify(`PDF split into ${count} parts in ${getBasename(destDir)}`, { type: 'success' });
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

  // -------------------------------------------------------------------------
  // 4. Tab 3: Reorder & Rotate Organizer Logic
  // -------------------------------------------------------------------------
  let reorderSourceTimeout = null;

  window.handleReorderSourceChange = function(val) {
    if (reorderSourceTimeout) clearTimeout(reorderSourceTimeout);
    reorderSourceTimeout = setTimeout(() => {
      loadPdfPagesOrganizer(val);
    }, 300);
  };

  window.reloadCurrentReorderDoc = function() {
    if (currentReorderSource) {
      loadPdfPagesOrganizer(currentReorderSource);
    }
  };

  async function loadPdfPagesOrganizer(pdfPath) {
    const grid = document.getElementById('reorder-pages-grid');
    const countEl = document.getElementById('reorder-page-count');
    const destEl = document.getElementById('reorder-dest-path');
    if (!grid) return;

    if (!pdfPath || !isPdfFile(pdfPath)) {
      grid.innerHTML = '<div class="empty-state" style="grid-column: 1 / -1;">Enter a PDF path or click "Pick" to load and organize pages.</div>';
      if (countEl) countEl.textContent = '';
      return;
    }

    currentReorderSource = pdfPath;
    grid.innerHTML = '<div class="empty-state" style="grid-column: 1 / -1;">Loading document pages in pure Rust...</div>';

    try {
      const resp = await fetch(`/api/tools/pdf/info?path=${encodeURIComponent(pdfPath)}`);
      if (resp.ok) {
        const info = await resp.json();
        reorderPages = [];
        for (let i = 1; i <= info.page_count; i++) {
          reorderPages.push({ page_num: i, rotation: 0 });
        }
        if (countEl) countEl.textContent = `${info.page_count} pages`;
        if (destEl && !destEl.value) {
          destEl.value = pdfPath.replace(/\.pdf$/i, '_reordered.pdf');
        }
        renderOrganizerGrid();
      } else {
        const err = await resp.text();
        grid.innerHTML = `<div class="empty-state" style="grid-column: 1 / -1; color: var(--danger);">Failed to read PDF document: ${escapeHtml(err)}</div>`;
        if (countEl) countEl.textContent = '';
      }
    } catch (e) {
      grid.innerHTML = `<div class="empty-state" style="grid-column: 1 / -1; color: var(--danger);">Error reading PDF: ${escapeHtml(e.message || e)}</div>`;
      if (countEl) countEl.textContent = '';
    }
  }

  function renderOrganizerGrid() {
    const grid = document.getElementById('reorder-pages-grid');
    const countEl = document.getElementById('reorder-page-count');
    if (!grid) return;

    if (countEl) countEl.textContent = `${reorderPages.length} pages`;

    if (reorderPages.length === 0) {
      grid.innerHTML = '<div class="empty-state" style="grid-column: 1 / -1;">All pages removed. Click Reset to restore original document pages.</div>';
      return;
    }

    grid.innerHTML = reorderPages.map((p, idx) => `
      <div class="page-card" data-idx="${idx}">
        <div class="page-card-title">Page ${p.page_num}</div>
        <div class="page-preview-box" style="transform: rotate(${p.rotation}deg);">
          <span class="page-preview-num">#${p.page_num}</span>
        </div>
        <div class="page-card-controls">
          <button type="button" class="btn-icon-sm" onclick="rotateOrganizerPage(${idx}, 90)" title="Rotate 90° Clockwise">↻</button>
          <button type="button" class="btn-icon-sm" onclick="moveOrganizerPage(${idx}, -1)" ${idx === 0 ? 'disabled' : ''} title="Move Left">◀</button>
          <button type="button" class="btn-icon-sm" onclick="moveOrganizerPage(${idx}, 1)" ${idx === reorderPages.length - 1 ? 'disabled' : ''} title="Move Right">▶</button>
          <button type="button" class="btn-icon-sm danger" onclick="removeOrganizerPage(${idx})" title="Delete Page">✕</button>
        </div>
      </div>
    `).join('');
  }

  window.rotateOrganizerPage = function(idx, deg) {
    if (reorderPages[idx]) {
      reorderPages[idx].rotation = (reorderPages[idx].rotation + deg) % 360;
      renderOrganizerGrid();
    }
  };

  window.moveOrganizerPage = function(idx, delta) {
    const target = idx + delta;
    if (target < 0 || target >= reorderPages.length) return;
    const item = reorderPages.splice(idx, 1)[0];
    reorderPages.splice(target, 0, item);
    renderOrganizerGrid();
  };

  window.removeOrganizerPage = function(idx) {
    reorderPages.splice(idx, 1);
    renderOrganizerGrid();
  };

  window.runPdfReorder = async function() {
    const source = document.getElementById('reorder-source-path')?.value?.trim();
    const dest = document.getElementById('reorder-dest-path')?.value?.trim();
    const progBox = document.getElementById('reorder-progress-box');
    const resultBanner = document.getElementById('reorder-result-banner');
    const btn = document.getElementById('btn-reorder-action');

    if (!source || !dest) {
      if (window.Brum?.ui) window.Brum.ui.notify('Please specify source PDF and destination output path', { type: 'warning' });
      return;
    }
    if (reorderPages.length === 0) {
      if (window.Brum?.ui) window.Brum.ui.notify('No pages to export in organizer', { type: 'warning' });
      return;
    }

    if (progBox) progBox.style.display = 'flex';
    if (resultBanner) resultBanner.style.display = 'none';
    if (btn) btn.disabled = true;

    try {
      const resp = await fetch('/api/tools/pdf/reorder', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          source: source,
          destination: dest,
          pages: reorderPages
        })
      });

      if (resp.ok) {
        if (resultBanner) {
          resultBanner.className = 'result-banner success';
          resultBanner.innerHTML = `
            <strong>✔ PDF Reordered & Saved!</strong><br>
            Exported <strong>${reorderPages.length} organized pages</strong> to <code>${escapeHtml(getBasename(dest))}</code>.<br>
            <span style="font-family: var(--font-mono); font-size: 11px;">Path: ${escapeHtml(dest)}</span>
          `;
        }
        if (window.Brum?.ui) {
          window.Brum.ui.notify(`Reordered & exported PDF to ${getBasename(dest)}`, { type: 'success' });
        }
      } else {
        const err = await resp.text();
        if (resultBanner) {
          resultBanner.className = 'result-banner error';
          resultBanner.textContent = `Reorder failed: ${err}`;
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

  // -------------------------------------------------------------------------
  // 5. Host Bridge Selection Helpers
  // -------------------------------------------------------------------------
  window.pickCurrentBrumPdf = async function(target) {
    let files = [];
    if (window.Brum && window.Brum.fs) {
      try {
        files = await window.Brum.fs.getSelectedFiles();
      } catch (_) {}
    }

    if (!files || files.length === 0) {
      if (currentContext && currentContext.selectedFiles) {
        files = currentContext.selectedFiles;
      }
    }

    const firstPdf = (files || []).find(isPdfFile);
    if (!firstPdf) {
      if (window.Brum?.ui) {
        window.Brum.ui.notify('Please select a PDF file in the active pane', { type: 'warning' });
      }
      return;
    }

    if (target === 'split') {
      const srcInput = document.getElementById('split-source-path');
      const destDir = document.getElementById('split-dest-dir');
      if (srcInput) srcInput.value = firstPdf;
      if (destDir) destDir.value = getParentDir(firstPdf);
      fetchPdfSplitInfo(firstPdf);
    } else if (target === 'reorder') {
      const reSrc = document.getElementById('reorder-source-path');
      if (reSrc) reSrc.value = firstPdf;
      loadPdfPagesOrganizer(firstPdf);
    }
  };

  // -------------------------------------------------------------------------
  // 6. Initialization & Context Dispatch
  // -------------------------------------------------------------------------
  function init(context) {
    currentContext = context || {};
    const selected = (currentContext.selectedFiles || []).filter(isPdfFile);
    const initialPdf = currentContext.initialPdf && isPdfFile(currentContext.initialPdf) ? currentContext.initialPdf : null;
    const initialTab = currentContext.tab || 'merge';

    if (initialTab && ['merge', 'split', 'reorder'].includes(initialTab)) {
      window.switchTab(initialTab);
    }

    if (initialPdf) {
      if (initialTab === 'split') {
        const srcEl = document.getElementById('split-source-path');
        const destDir = document.getElementById('split-dest-dir');
        if (srcEl) srcEl.value = initialPdf;
        if (destDir) destDir.value = getParentDir(initialPdf);
        fetchPdfSplitInfo(initialPdf);
      } else if (initialTab === 'reorder') {
        const reSrc = document.getElementById('reorder-source-path');
        if (reSrc) reSrc.value = initialPdf;
        loadPdfPagesOrganizer(initialPdf);
      } else {
        mergeFiles = [initialPdf];
        renderMergeList();
        const destInput = document.getElementById('merge-dest-path');
        if (destInput && !destInput.value) {
          destInput.value = `${getParentDir(initialPdf).replace(/\/+$/, '')}/merged_document.pdf`;
        }
      }
    } else if (selected.length > 0) {
      if (selected.length === 1 && initialTab === 'split') {
        const srcEl = document.getElementById('split-source-path');
        const destDir = document.getElementById('split-dest-dir');
        if (srcEl) srcEl.value = selected[0];
        if (destDir) destDir.value = getParentDir(selected[0]);
        fetchPdfSplitInfo(selected[0]);
      } else if (selected.length === 1 && initialTab === 'reorder') {
        const reSrc = document.getElementById('reorder-source-path');
        if (reSrc) reSrc.value = selected[0];
        loadPdfPagesOrganizer(selected[0]);
      } else {
        mergeFiles = [...selected];
        renderMergeList();
        const destInput = document.getElementById('merge-dest-path');
        if (destInput && !destInput.value && mergeFiles.length > 0) {
          destInput.value = `${getParentDir(mergeFiles[0]).replace(/\/+$/, '')}/merged_document.pdf`;
        }
      }
    } else {
      const activePath = currentContext.activePath || '/';
      const destDir = document.getElementById('split-dest-dir');
      if (destDir && !destDir.value) destDir.value = activePath;
      const mergeDest = document.getElementById('merge-dest-path');
      if (mergeDest && !mergeDest.value) mergeDest.value = `${activePath.replace(/\/+$/, '')}/merged_document.pdf`;
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
    renderMergeList();
  });
})();
