// Runs before the page is drawn: picks the login, lock or loading state from what the
// browser remembers, so the right screen shows without a flash. Kept as a file (not inline)
// so the page policy can allow only Brum's own scripts.
(function() {
  try {
    if (localStorage.getItem('cd_show_fkeys') === 'false') {
      document.documentElement.classList.add('fkeys-disabled');
    }
    var token = localStorage.getItem('cd_token');
    var isLocked = localStorage.getItem('cd_is_locked') === 'true';
    var isTauri = window.__TAURI__ !== undefined || window.__TAURI_INTERNALS__ !== undefined || window.__WRY__ !== undefined;
    if (isLocked) {
      document.documentElement.classList.add('auth-pending-lock');
    } else if (!token && !isTauri) {
      document.documentElement.classList.add('auth-pending-login');
    } else {
      document.documentElement.classList.add('auth-verifying');
    }
  } catch (_) {
    document.documentElement.classList.add('auth-pending-login');
  }

  // Immediate frame-0 version badge update from cache & early API sync
  function updateVersionElements(v) {
    if (!v) return;
    document.querySelectorAll('.login-version-badge').forEach(function(el) { el.textContent = 'v' + v; });
    document.querySelectorAll('.lock-version-badge, #lock-corner-version-badge').forEach(function(el) { el.textContent = 'Brum v' + v; });
    document.querySelectorAll('.login-app-version, #login-app-version').forEach(function(el) { el.textContent = v; });
    document.querySelectorAll('#about-version-badge, .about-version-badge').forEach(function(el) { el.textContent = 'v' + v + ' (Desktop & Web)'; });
  }
  document.addEventListener('DOMContentLoaded', function() {
    try {
      var v = localStorage.getItem('cd_cached_version');
      if (v) updateVersionElements(v);

      // Early Lock Screen Avatar & Name Hydration
      if (localStorage.getItem('cd_is_locked') === 'true') {
        var localAvatar = localStorage.getItem('cd_local_avatar');
        var localNick = localStorage.getItem('cd_local_nickname');
        var cachedInfo = null;
        try { cachedInfo = JSON.parse(localStorage.getItem('cd_user_info') || 'null'); } catch (_) {}
        var avatar = (cachedInfo && cachedInfo.avatar_url) || localAvatar || '👤';
        var uname = (cachedInfo && (cachedInfo.nickname || cachedInfo.username)) || localNick || localStorage.getItem('cd_last_username') || 'Brum User';

        var userTextEl = document.getElementById('lock-username-text');
        if (userTextEl) userTextEl.textContent = uname;

        var avatarEl = document.getElementById('lock-avatar-thumb');
        if (avatarEl && avatar) {
          var isImg = avatar.startsWith('data:image') || avatar.startsWith('http://') || avatar.startsWith('https://') || avatar.startsWith('/') || avatar.startsWith('blob:') || avatar.startsWith('assets/') || /\.(png|jpe?g|webp|gif|svg|ico)(\?.*)?$/i.test(avatar);
          if (isImg) {
            // Built as an element: the cached avatar address is never parsed as HTML
            var img = document.createElement('img');
            img.src = avatar;
            img.alt = 'Avatar';
            img.style.cssText = 'width:100%;height:100%;object-fit:cover;border-radius:inherit;display:block;';
            avatarEl.textContent = '';
            avatarEl.appendChild(img);
          } else {
            avatarEl.textContent = avatar;
          }
        }
      }

      // Early Mascot Mood Hydration (prevents jitter / flickering)
      try {
        var storedMascot = sessionStorage.getItem('cd_session_mascot');
        if (!storedMascot) {
          storedMascot = Math.random() < 0.5 ? 'happy' : 'fierce';
          sessionStorage.setItem('cd_session_mascot', storedMascot);
        }
        var mascotSrc = storedMascot === 'fierce' ? 'assets/brum_fierce.webp' : 'assets/brum_happy.webp';
        document.querySelectorAll('#header-brand-logo, #login-brand-logo, #lock-mascot-logo, #about-modal-brand-logo, #about-settings-brand-logo').forEach(function(img) {
          img.src = mascotSrc;
          img.dataset.mascotMood = storedMascot;
        });
      } catch (_) {}

      fetch('/api/system/status').then(function(r) { return r.json(); }).then(function(d) {
        if (d && d.version) {
          localStorage.setItem('cd_cached_version', d.version);
          updateVersionElements(d.version);
        }
      }).catch(function() {});
    } catch (_) {}
  });

  // Safety watchdog: Only fallback if authentication verification takes > 10s
  setTimeout(function() {
    if (document.documentElement.classList.contains('auth-verifying')) {
      document.documentElement.classList.remove('auth-verifying');
      var isLocked = localStorage.getItem('cd_is_locked') === 'true';
      if (isLocked) {
        document.documentElement.classList.add('auth-pending-lock');
      } else if (!localStorage.getItem('cd_token')) {
        document.documentElement.classList.add('auth-pending-login');
      }
    }
  }, 10000);
})();
