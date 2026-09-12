// 🐶 Starter ChewToy Client Logic

document.addEventListener("DOMContentLoaded", () => {
  const contextDisplay = document.getElementById("context-display");
  const fileContent = document.getElementById("file-content");
  const btnPing = document.getElementById("btn-ping");
  const btnRead = document.getElementById("btn-read");

  let currentContext = null;

  // Initialize with window.Brum host SDK
  const Brum = window.Brum || window.parent?.Brum;
  if (Brum) {
    Brum.onReady((context) => {
      currentContext = context;
      contextDisplay.textContent = JSON.stringify(context, null, 2);
    });

    btnPing.addEventListener("click", () => {
      Brum.ui.notify("Hello from Starter ChewToy!", { type: "info" });
    });

    btnRead.addEventListener("click", async () => {
      if (!currentContext || !currentContext.selectedFiles || currentContext.selectedFiles.length === 0) {
        Brum.ui.notify("No file selected in active panel.", { type: "warning" });
        return;
      }
      const target = currentContext.selectedFiles[0];
      try {
        const text = await Brum.fs.readFile(target);
        fileContent.textContent = text.slice(0, 1000) + (text.length > 1000 ? "\n... [truncated]" : "");
        Brum.ui.notify(`Loaded ${target}`, { type: "success" });
      } catch (err) {
        fileContent.textContent = "Error reading file: " + err;
        Brum.ui.notify("Failed to read file: " + err, { type: "error" });
      }
    });
  } else {
    contextDisplay.textContent = "Running standalone preview (window.Brum SDK not detected).";
  }
});
