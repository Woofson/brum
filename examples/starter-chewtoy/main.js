// 🐶 Starter ChewToy Client Logic & Host Bridge Integration

/**
 * Universal window.Brum Host Bridge Client
 * Works directly or seamlessly across cross-window / postMessage boundaries
 */
function createBrumClient() {
  if (window.Brum) return window.Brum;
  if (window.parent && window.parent.Brum) return window.parent.Brum;

  let reqIdCounter = 0;
  const pendingRequests = new Map();
  const readyListeners = [];

  window.addEventListener("message", (e) => {
    if (!e.data) return;
    if (e.data.type === "BRUM_RES") {
      const { reqId, result, error } = e.data;
      if (pendingRequests.has(reqId)) {
        const { resolve, reject } = pendingRequests.get(reqId);
        pendingRequests.delete(reqId);
        if (error) reject(new Error(error));
        else resolve(result);
      }
    } else if (e.data.type === "BRUM_READY") {
      readyListeners.forEach(cb => {
        try { cb(e.data.context); } catch (err) { console.error(err); }
      });
    }
  });

  function callHost(action, payload = {}) {
    return new Promise((resolve, reject) => {
      const reqId = ++reqIdCounter;
      pendingRequests.set(reqId, { resolve, reject });
      window.parent.postMessage({ type: "BRUM_REQ", reqId, action, payload }, "*");
    });
  }

  return {
    onReady: function(cb) {
      if (typeof cb === "function") readyListeners.push(cb);
    },
    fs: {
      readFile: (path, opts) => callHost("fs.readFile", { path, opts }),
      writeFile: (path, content) => callHost("fs.writeFile", { path, content }),
      listDir: (path) => callHost("fs.listDir", { path }),
      getActivePath: () => callHost("fs.getActivePath"),
      getSelectedFiles: () => callHost("fs.getSelectedFiles"),
    },
    ui: {
      notify: (message, opts) => callHost("ui.notify", { message, opts }),
      getTheme: () => callHost("ui.getTheme"),
      showConfirm: (title, message) => callHost("ui.showConfirm", { title, message }),
    },
    window: {
      dockTo: (pane) => callHost("window.dockTo", { pane }),
      float: () => callHost("window.float"),
      close: () => callHost("window.close"),
      setTitle: (title) => callHost("window.setTitle", { title }),
    },
    auth: {
      getUser: () => callHost("auth.getUser"),
    }
  };
}

document.addEventListener("DOMContentLoaded", () => {
  const contextDisplay = document.getElementById("context-display");
  const outputDisplay = document.getElementById("output-display");
  const outputTitle = document.getElementById("output-title");
  const sdkStatus = document.getElementById("sdk-status");

  const btnPing = document.getElementById("btn-ping");
  const btnDock1 = document.getElementById("btn-dock1");
  const btnDock2 = document.getElementById("btn-dock2");
  const btnListDir = document.getElementById("btn-list-dir");
  const btnReadFile = document.getElementById("btn-read-file");
  const btnWriteFile = document.getElementById("btn-write-file");
  const btnConfirm = document.getElementById("btn-confirm-dialog");
  const btnClear = document.getElementById("btn-clear-output");

  let currentContext = {
    activePath: "/",
    selectedFiles: [],
    paneIndex: 0,
    theme: "amber-charcoal"
  };

  const Brum = createBrumClient();

  if (Brum) {
    if (sdkStatus) sdkStatus.textContent = "● Connected to window.Brum";

    Brum.onReady((context) => {
      currentContext = Object.assign(currentContext, context);
      if (contextDisplay) {
        contextDisplay.textContent = JSON.stringify(currentContext, null, 2);
      }
    });

    // 1. Toast Ping
    btnPing?.addEventListener("click", () => {
      Brum.ui.notify("Hello from Starter ChewToy!", { type: "info" });
    });

    // 2. Dock Actions
    btnDock1?.addEventListener("click", () => {
      Brum.window.dockTo(1);
    });
    btnDock2?.addEventListener("click", () => {
      Brum.window.dockTo(2);
    });

    // 3. List Active Directory
    btnListDir?.addEventListener("click", async () => {
      const targetPath = currentContext.activePath || "/";
      if (outputTitle) outputTitle.textContent = `📂 Directory Listing: ${targetPath}`;
      if (outputDisplay) outputDisplay.textContent = "Querying directory listing from host...";
      try {
        const listing = await Brum.fs.listDir(targetPath);
        const entries = (listing.entries || listing || []).map(e => {
          const type = e.is_dir ? "📁 DIR " : "📄 FILE";
          const size = e.size !== undefined ? ` (${e.size} bytes)` : "";
          return `${type}  ${e.name}${size}`;
        });
        if (outputDisplay) {
          outputDisplay.textContent = `Total Entries: ${entries.length}\nPath: ${targetPath}\n\n` + entries.join("\n");
        }
        Brum.ui.notify(`Listed ${entries.length} items in ${targetPath}`, { type: "success" });
      } catch (err) {
        if (outputDisplay) outputDisplay.textContent = "Error listing directory: " + (err.message || err);
        Brum.ui.notify("Failed to list directory: " + (err.message || err), { type: "error" });
      }
    });

    // 4. Read Selected File
    btnReadFile?.addEventListener("click", async () => {
      let targetFile = currentContext.selectedFiles?.[0];
      if (!targetFile) {
        // Fallback: prompt or try reading a common file
        targetFile = prompt("No file selected in active pane. Enter path to read:", `${currentContext.activePath}/Cargo.toml`);
      }
      if (!targetFile) return;

      if (outputTitle) outputTitle.textContent = `📄 File Inspector: ${targetFile}`;
      if (outputDisplay) outputDisplay.textContent = `Reading ${targetFile}...`;

      try {
        const content = await Brum.fs.readFile(targetFile, { maxBytes: 50000 });
        const text = typeof content === "string" ? content : JSON.stringify(content, null, 2);
        if (outputDisplay) {
          outputDisplay.textContent = text.slice(0, 4000) + (text.length > 4000 ? "\n\n... [Truncated for preview]" : "");
        }
        Brum.ui.notify(`Read file successfully (${text.length} chars)`, { type: "success" });
      } catch (err) {
        if (outputDisplay) outputDisplay.textContent = "Error reading file: " + (err.message || err);
        Brum.ui.notify("Failed to read file: " + (err.message || err), { type: "error" });
      }
    });

    // 5. Write Test Note
    btnWriteFile?.addEventListener("click", async () => {
      const folder = currentContext.activePath || "/tmp";
      const targetPath = `${folder.replace(/\/$/, "")}/chewtoy-note.txt`;
      const noteContent = `🐶 ChewToy Note\nGenerated at: ${new Date().toISOString()}\nHost Theme: ${currentContext.theme}\nActive Path: ${currentContext.activePath}\n`;

      if (outputTitle) outputTitle.textContent = `✍️ Write Output: ${targetPath}`;
      if (outputDisplay) outputDisplay.textContent = `Writing test file to ${targetPath}...`;

      try {
        await Brum.fs.writeFile(targetPath, noteContent);
        if (outputDisplay) {
          outputDisplay.textContent = `Successfully saved:\n${targetPath}\n\nContent:\n${noteContent}`;
        }
        Brum.ui.notify(`Saved note to ${targetPath}`, { type: "success" });
      } catch (err) {
        if (outputDisplay) outputDisplay.textContent = "Error writing file: " + (err.message || err);
        Brum.ui.notify("Failed to write file: " + (err.message || err), { type: "error" });
      }
    });

    // 6. Confirm Dialog
    btnConfirm?.addEventListener("click", async () => {
      try {
        const res = await Brum.ui.showConfirm("Starter ChewToy", "Do you like the new .grr ChewToy extension system?");
        if (outputTitle) outputTitle.textContent = "❓ Confirmation Dialog Result";
        if (outputDisplay) outputDisplay.textContent = `User confirmation response: ${res ? "Confirmed (OK)" : "Cancelled"}`;
        Brum.ui.notify(`Confirmation answer: ${res}`, { type: res ? "success" : "info" });
      } catch (err) {
        console.error(err);
      }
    });

    // 7. Clear Output
    btnClear?.addEventListener("click", () => {
      if (outputDisplay) outputDisplay.textContent = "Inspector output cleared.";
      if (outputTitle) outputTitle.textContent = "📄 Output & Inspector";
    });
  } else {
    if (sdkStatus) {
      sdkStatus.textContent = "⚠ Standalone Mode (No window.Brum)";
      sdkStatus.style.color = "#f59e0b";
    }
    if (contextDisplay) {
      contextDisplay.textContent = "Running outside Brum host file manager.";
    }
  }
});

