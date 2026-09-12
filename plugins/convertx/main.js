/**
 * ConvertX Media & Document Transcoder
 * Modular ChewToy for Brum / CommanderDog
 */

(function () {
  'use strict';

  let currentContext = {
    selectedFiles: [],
    filePath: '',
    defaultFormat: null,
    paneIndex: 0,
    token: '',
    endpoint: ''
  };

  let activeSourceFile = '';
  let activeSourceSize = 0;
  let isCustomOutputName = false;
  let isConverting = false;
  let timerInterval = null;
  let startTime = 0;

  const EXTENSION_CATEGORIES = {
    image: ['png', 'jpg', 'jpeg', 'webp', 'avif', 'gif', 'bmp', 'ico', 'tiff', 'svg', 'heic', 'psd'],
    video: ['mp4', 'webm', 'mkv', 'avi', 'mov', 'flv', 'wmv', 'm4v', '3gp', 'ts'],
    audio: ['mp3', 'wav', 'flac', 'ogg', 'm4a', 'aac', 'wma', 'opus', 'aiff'],
    docs: ['json', 'yaml', 'yml', 'toml', 'csv', 'html', 'htm', 'md', 'txt', 'base64']
  };

  function getExtension(path) {
    if (!path) return '';
    const name = path.split('/').pop() || '';
    const dotIdx = name.lastIndexOf('.');
    return dotIdx !== -1 ? name.substring(dotIdx + 1).toLowerCase() : '';
  }

  function getStem(path) {
    if (!path) return '';
    const name = path.split('/').pop() || '';
    const dotIdx = name.lastIndexOf('.');
    return dotIdx !== -1 ? name.substring(0, dotIdx) : name;
  }

  function getDirectory(path) {
    if (!path) return '';
    const lastSlash = path.lastIndexOf('/');
    return lastSlash !== -1 ? path.substring(0, lastSlash) : '';
  }

  function formatBytes(bytes) {
    if (!bytes || bytes === 0) return '0 B';
    const k = 1024;
    const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + ' ' + sizes[i];
  }

  function detectCategory(ext) {
    if (EXTENSION_CATEGORIES.image.includes(ext)) return 'images';
    if (EXTENSION_CATEGORIES.video.includes(ext)) return 'video';
    if (EXTENSION_CATEGORIES.audio.includes(ext)) return 'audio';
    if (EXTENSION_CATEGORIES.docs.includes(ext)) return 'docs';
    return 'all';
  }

  function getIconForCategory(cat) {
    switch (cat) {
      case 'images': return '🖼️';
      case 'video': return '🎬';
      case 'audio': return '🎵';
      case 'docs': return '📄';
      default: return '📦';
    }
  }

  function initHostBridge() {
    if (window.Brum && typeof window.Brum.onReady === 'function') {
      window.Brum.onReady(function (ctx) {
        if (!ctx) return;
        currentContext = Object.assign({}, currentContext, ctx);
        if (ctx.theme) {
          document.documentElement.setAttribute('data-theme', ctx.theme);
        }

        let initialFile = ctx.filePath || '';
        if (!initialFile && ctx.selectedFiles && ctx.selectedFiles.length > 0) {
          initialFile = typeof ctx.selectedFiles[0] === 'string'
            ? ctx.selectedFiles[0]
            : (ctx.selectedFiles[0].path || '');
        }

        if (initialFile) {
          loadSourceFile(initialFile, ctx.defaultFormat);
        }
      });
    }
  }

  function loadSourceFile(filePath, defaultFormat) {
    activeSourceFile = filePath || '';
    isCustomOutputName = false;

    const fileName = activeSourceFile.split('/').pop() || 'No file selected';
    const ext = getExtension(activeSourceFile);
    const cat = detectCategory(ext);

    const nameEl = document.getElementById('source-filename');
    const pathEl = document.getElementById('source-path-label');
    const inputEl = document.getElementById('input-source-path');
    const iconEl = document.getElementById('source-type-icon');
    const sizeEl = document.getElementById('source-filesize');

    if (nameEl) nameEl.textContent = fileName;
    if (pathEl) pathEl.textContent = activeSourceFile || 'Select a file in Brum';
    if (inputEl) inputEl.value = activeSourceFile;
    if (iconEl) iconEl.textContent = getIconForCategory(cat);
    if (sizeEl) sizeEl.textContent = formatBytes(activeSourceSize || 0);

    // Auto-select category filter and recommended target format
    const targetSelect = document.getElementById('select-target-format');
    if (targetSelect) {
      if (defaultFormat) {
        targetSelect.value = defaultFormat;
      } else if (cat === 'images') {
        targetSelect.value = ext === 'webp' ? 'png' : 'webp';
      } else if (cat === 'video') {
        targetSelect.value = 'mp4';
      } else if (cat === 'audio') {
        targetSelect.value = 'mp3';
      } else if (cat === 'docs') {
        targetSelect.value = ext === 'json' ? 'yaml' : (ext === 'yaml' ? 'json' : 'toml');
      }
      handleFormatChange(targetSelect.value);
    }

    filterCategory(cat !== 'all' ? cat : 'all');
    updateOutputNamePreview(true);
    resetResultViews();
  }

  window.handleManualPathChange = function (val) {
    activeSourceFile = val ? val.trim() : '';
    const fileName = activeSourceFile.split('/').pop() || 'No file selected';
    const ext = getExtension(activeSourceFile);
    const cat = detectCategory(ext);

    const nameEl = document.getElementById('source-filename');
    const pathEl = document.getElementById('source-path-label');
    const iconEl = document.getElementById('source-type-icon');

    if (nameEl) nameEl.textContent = fileName;
    if (pathEl) pathEl.textContent = activeSourceFile || 'No path specified';
    if (iconEl) iconEl.textContent = getIconForCategory(cat);

    updateOutputNamePreview(false);
  };

  window.pickCurrentBrumSelection = function () {
    if (window.Brum && typeof window.Brum.getSelectedFiles === 'function') {
      const sel = window.Brum.getSelectedFiles();
      if (sel && sel.length > 0) {
        const item = sel[0];
        const path = typeof item === 'string' ? item : item.path;
        if (path) {
          loadSourceFile(path, null);
          if (window.Brum.showToast) {
            window.Brum.showToast('Loaded active file from Brum pane', 'info');
          }
          return;
        }
      }
    }
    if (window.Brum && window.Brum.showToast) {
      window.Brum.showToast('No file selected in active pane', 'warning');
    }
  };

  window.filterCategory = function (category) {
    const buttons = document.querySelectorAll('.category-btn');
    buttons.forEach(function (btn) {
      btn.classList.remove('active');
    });

    const activeBtn = document.getElementById('cat-' + category);
    if (activeBtn) activeBtn.classList.add('active');

    const optImages = document.getElementById('optgroup-images');
    const optVideo = document.getElementById('optgroup-video');
    const optAudio = document.getElementById('optgroup-audio');
    const optDocs = document.getElementById('optgroup-docs');

    if (category === 'all') {
      if (optImages) optImages.style.display = '';
      if (optVideo) optVideo.style.display = '';
      if (optAudio) optAudio.style.display = '';
      if (optDocs) optDocs.style.display = '';
    } else {
      if (optImages) optImages.style.display = category === 'images' ? '' : 'none';
      if (optVideo) optVideo.style.display = category === 'video' ? '' : 'none';
      if (optAudio) optAudio.style.display = category === 'audio' ? '' : 'none';
      if (optDocs) optDocs.style.display = category === 'docs' ? '' : 'none';
    }
  };

  window.handleFormatChange = function (fmt) {
    const isImage = ['png', 'jpg', 'jpeg', 'webp', 'avif', 'gif', 'bmp', 'ico', 'tiff'].includes(fmt.toLowerCase());
    const imgPanel = document.getElementById('panel-image-options');
    if (imgPanel) {
      imgPanel.style.display = isImage ? 'flex' : 'none';
    }
    updateOutputNamePreview(false);
  };

  window.updateQualityDisplay = function (val) {
    const badge = document.getElementById('quality-val-badge');
    if (badge) badge.textContent = val + '%';
  };

  window.applyDimensionPreset = function (w, h) {
    const wInput = document.getElementById('input-resize-w');
    const hInput = document.getElementById('input-resize-h');
    if (wInput) wInput.value = w !== null ? w : '';
    if (hInput) hInput.value = h !== null ? h : '';
    updateOutputNamePreview(false);
  };

  window.handleCustomNameInput = function () {
    isCustomOutputName = true;
    updateOutputNamePreview(false);
  };

  window.updateOutputNamePreview = function (forceReset) {
    if (!activeSourceFile) {
      const previewEl = document.getElementById('preview-dest-path');
      if (previewEl) previewEl.textContent = 'Select a file to preview destination';
      return;
    }

    const stem = getStem(activeSourceFile);
    const srcExt = getExtension(activeSourceFile);
    const targetFmt = (document.getElementById('select-target-format')?.value || 'webp').toLowerCase().trim();
    const nameInput = document.getElementById('input-output-name');
    const previewEl = document.getElementById('preview-dest-path');

    if (forceReset || !isCustomOutputName) {
      const isSameExt = srcExt === targetFmt;
      const defaultName = isSameExt ? (stem + '_converted.' + targetFmt) : (stem + '.' + targetFmt);
      if (nameInput) nameInput.value = defaultName;
    }

    const currentName = nameInput ? nameInput.value.trim() : (stem + '.' + targetFmt);
    const parentDir = getDirectory(activeSourceFile);
    const fullDest = parentDir ? (parentDir + '/' + currentName) : currentName;

    if (previewEl) {
      previewEl.textContent = fullDest;
    }
  };

  function resetResultViews() {
    const progressCard = document.getElementById('card-progress');
    const resultCard = document.getElementById('card-result');
    const errorCard = document.getElementById('card-error');
    const btnCancel = document.getElementById('btn-cancel');
    const btnBackground = document.getElementById('btn-background');
    const btnRun = document.getElementById('btn-run');
    const btnDone = document.getElementById('btn-done');

    if (progressCard) progressCard.style.display = 'none';
    if (resultCard) resultCard.style.display = 'none';
    if (errorCard) errorCard.style.display = 'none';

    if (btnCancel) btnCancel.style.display = 'inline-flex';
    if (btnBackground) btnBackground.style.display = 'none';
    if (btnRun) {
      btnRun.style.display = 'inline-flex';
      btnRun.disabled = false;
    }
    if (btnDone) btnDone.style.display = 'none';

    if (timerInterval) {
      clearInterval(timerInterval);
      timerInterval = null;
    }
  }

  window.runConversion = async function () {
    if (!activeSourceFile) {
      showError('Please select or specify a valid file to convert.');
      return;
    }

    const targetFormat = document.getElementById('select-target-format')?.value || 'webp';
    const quality = parseInt(document.getElementById('input-quality')?.value || '85', 10);
    const resizeW = parseInt(document.getElementById('input-resize-w')?.value, 10) || null;
    const resizeH = parseInt(document.getElementById('input-resize-h')?.value, 10) || null;
    const customName = document.getElementById('input-output-name')?.value.trim();

    let resolvedOutputPath = null;
    if (customName) {
      const parentDir = getDirectory(activeSourceFile);
      resolvedOutputPath = parentDir ? (parentDir + '/' + customName) : customName;
    }

    resetResultViews();

    const progressCard = document.getElementById('card-progress');
    const progressText = document.getElementById('progress-text');
    const progressElapsed = document.getElementById('progress-elapsed');
    const btnBackground = document.getElementById('btn-background');
    const btnRun = document.getElementById('btn-run');

    if (progressCard) progressCard.style.display = 'flex';
    if (progressText) progressText.textContent = 'Transcoding ' + (activeSourceFile.split('/').pop()) + ' to ' + targetFormat.toUpperCase() + '...';
    if (btnBackground) btnBackground.style.display = 'inline-flex';
    if (btnRun) btnRun.disabled = true;

    isConverting = true;
    startTime = Date.now();
    timerInterval = setInterval(function () {
      const elapsed = ((Date.now() - startTime) / 1000).toFixed(1);
      if (progressElapsed) progressElapsed.textContent = elapsed + 's';
    }, 100);

    try {
      const endpoint = currentContext.endpoint || '';
      const headers = {
        'Content-Type': 'application/json'
      };
      if (currentContext.token) {
        headers['Authorization'] = 'Bearer ' + currentContext.token;
      }

      const resp = await fetch(endpoint + '/api/tools/convert', {
        method: 'POST',
        headers: headers,
        body: JSON.stringify({
          source_path: activeSourceFile,
          target_format: targetFormat,
          output_path: resolvedOutputPath,
          quality: quality,
          resize_width: resizeW,
          resize_height: resizeH
        })
      });

      isConverting = false;
      if (timerInterval) {
        clearInterval(timerInterval);
        timerInterval = null;
      }
      if (progressCard) progressCard.style.display = 'none';

      if (resp.ok) {
        const data = await resp.json();
        showSuccessResult(data, targetFormat);
      } else {
        const errText = await resp.text();
        showError('Conversion failed: ' + errText);
      }
    } catch (e) {
      isConverting = false;
      if (timerInterval) {
        clearInterval(timerInterval);
        timerInterval = null;
      }
      if (progressCard) progressCard.style.display = 'none';
      showError('Network or system error: ' + e);
    }
  };

  function showSuccessResult(data, targetFormat) {
    const resultCard = document.getElementById('card-result');
    const srcSizeEl = document.getElementById('res-source-size');
    const outSizeEl = document.getElementById('res-output-size');
    const outPathEl = document.getElementById('res-output-path');
    const btnCancel = document.getElementById('btn-cancel');
    const btnBackground = document.getElementById('btn-background');
    const btnRun = document.getElementById('btn-run');
    const btnDone = document.getElementById('btn-done');

    if (srcSizeEl) srcSizeEl.textContent = formatBytes(activeSourceSize || 0);
    if (outSizeEl) outSizeEl.textContent = formatBytes(data.output_size || 0);
    if (outPathEl) outPathEl.textContent = 'Destination: ' + data.output_path;

    if (resultCard) resultCard.style.display = 'flex';
    if (btnCancel) btnCancel.style.display = 'none';
    if (btnBackground) btnBackground.style.display = 'none';
    if (btnRun) btnRun.style.display = 'none';
    if (btnDone) btnDone.style.display = 'inline-flex';

    if (window.Brum) {
      if (typeof window.Brum.refreshPanes === 'function') {
        window.Brum.refreshPanes();
      }
      if (typeof window.Brum.showToast === 'function') {
        window.Brum.showToast('✅ Converted to ' + targetFormat.toUpperCase() + ' (' + formatBytes(data.output_size) + ')', 'success');
      }
    }
  }

  function showError(msg) {
    const errorCard = document.getElementById('card-error');
    const btnRun = document.getElementById('btn-run');
    const btnBackground = document.getElementById('btn-background');

    if (errorCard) {
      errorCard.textContent = msg;
      errorCard.style.display = 'block';
    }
    if (btnRun) btnRun.disabled = false;
    if (btnBackground) btnBackground.style.display = 'none';

    if (window.Brum && typeof window.Brum.showToast === 'function') {
      window.Brum.showToast(msg, 'error');
    }
  }

  window.minimizeToBackground = function () {
    if (window.Brum && typeof window.Brum.closeChewToy === 'function') {
      window.Brum.closeChewToy();
    }
    if (window.Brum && typeof window.Brum.showToast === 'function') {
      window.Brum.showToast('ConvertX running in background...', 'info');
    }
  };

  window.handleCancelOrClose = function () {
    if (window.Brum && typeof window.Brum.closeChewToy === 'function') {
      window.Brum.closeChewToy();
    }
  };

  window.handleDone = function () {
    if (window.Brum && typeof window.Brum.closeChewToy === 'function') {
      window.Brum.closeChewToy();
    }
  };

  // Initialize
  document.addEventListener('DOMContentLoaded', function () {
    initHostBridge();
  });
})();
