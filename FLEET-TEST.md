# Commander Fleet: Morning Coffee Testing Guide ☕⚡

Welcome to the **Commander Fleet** verification walkthrough. This step-by-step guide will walk you through testing multi-node orchestration, cross-pane remote file management, and streamed cross-node transfers in Brum / CommanderDog.

---

## 🛠️ Step 0: Quick Architecture Overview

Commander Fleet allows each browser panel to independently bind to different Brum nodes across your network, VPN, or local ports:
* **Node A (Host 1)**: e.g. `http://localhost:3140`
* **Node B (Host 2 / Test Node)**: e.g. `http://localhost:3141` or a remote machine (`http://nas.local:3140`, `http://100.x.y.z:3140`)

---

## 🚀 Step 1: Spin Up a Secondary Test Node

If you already have a second Brum server running on your network/VPN, you can skip to **Step 2**.

To spin up a secondary test node locally on port `3141`:

```bash
# In a new terminal window:
PORT=3141 STORAGE_BASE_DIR=/tmp/fleet-test-node cargo run
```

---

## 🔑 Step 2: Generate an API Token on Node 2

1. Open Node 2 in your browser: `http://localhost:3141` (or your remote host).
2. Open **Settings** (Gear icon in top header or `F9` / `Ctrl+,`).
3. Click the **Security & Access** tab, then click **API Tokens & Service Credentials** (or open via ChewToy menu).
4. Click **+ New API Token**:
   - **Token Name**: `Fleet Primary Console`
   - **Role**: `Admin` (or Read/Write)
   - **Expiration**: `Never (Persistent)`
5. Copy the generated token string (`arf_...`).

---

## 🛰️ Step 3: Register Node 2 in Fleet Manager on Node 1

1. Return to your primary CommanderDog session: `http://localhost:3140`.
2. Open the **ChewToy Menu** (Wrench icon) $\rightarrow$ select **Fleet Manager** (or click the Server badge in the top header $\rightarrow$ **Fleet Manager**).
3. Under **Add Remote Node Profile**, fill in:
   - **Node Name**: `Secondary Test Node` (or `Goshawk NAS`)
   - **Endpoint URL**: `http://localhost:3141` (or `http://remote-ip:3140`)
   - **Accent Color**: Pick an accent color (e.g. Amber `#f59e0b` or Cyan `#06b6d4`)
   - **Auth Token**: Paste the `arf_...` token from Step 2.
4. Click **Add Node**.
5. Verify the node appears in the saved list with a green ping status indicator.

---

## 🔀 Step 4: Cross-Pane Multi-Node Binding (The Dual-Panel Power Move)

1. Look at the pane sub-headers:
   - **Panel 1 (Left)**: Shows `⚡ Localhost` with a green indicator.
   - **Panel 2 (Right)**: Shows `⚡ Localhost` with a green indicator.
2. Click the `⚡ Localhost` button on **Panel 2 (Right)** header.
3. Select **`Secondary Test Node`** from the dropdown.
4. **Verification**:
   - Panel 2 now illuminates with the remote node's accent glow.
   - Panel 2 breadcrumb bar shows the `[⚡ Secondary Test Node]` badge chip.
   - Directory contents in Panel 2 now represent Node 2's filesystem!

---

## 📂 Step 5: Test Remote VFS Navigation & Root Switching

1. In Panel 2 (Remote), click the root dropdown (`/` icon) on the breadcrumb bar:
   - Notice that available roots (Storage base, Home, Disks) reflect Node 2's drives and configuration.
2. Double-click folders to navigate into remote directories.
3. Press `F7` (or click `+ New Folder`) in Panel 2:
   - Create folder `fleet-transfer-test`.
   - Verify it appears instantly on the remote node.

---

## ⚡ Step 6: Test Cross-Node Streamed Transfers (F5 Copy & F6 Move)

1. In **Panel 1 (Local)**, select one or more files (e.g. text files, images, or audio tracks).
2. Press **`F5`** (Copy):
   - Notice the copy dialog indicates source and destination targets across different nodes.
   - Confirm copy.
3. **Verification**:
   - The Task Manager pill in the bottom status bar tracks the streamed pipeline transfer (`Source Node` $\rightarrow$ `Browser Stream` $\rightarrow$ `Destination Node`).
   - The files appear in Panel 2 (Remote) once complete.
4. Try transferring in the opposite direction (Panel 2 Remote $\rightarrow$ Panel 1 Local).
5. Try **`F6`** (Move) to verify source deletion upon successful transfer.

---

## 👁️ Step 7: Test Remote Viewers, Code Editor & Media Streaming

1. **Document Viewer (`F3` / Click)**:
   - Click a `.md`, `.txt`, or `.log` file in Panel 2 (Remote).
   - Verify the document viewer opens, displays the remote content, and dynamic tools (`tail -f`, line numbers, wrap) work seamlessly.
2. **Code Editor (`F4`)**:
   - Select a remote file in Panel 2 and press `F4`.
   - Make an edit and press `Ctrl+S`.
   - Verify the file is saved directly to the remote node.
3. **AMP / SoundDog Audio Streaming**:
   - Double click an `.mp3` or `.flac` file in Panel 2 (Remote).
   - Verify SoundDog streams audio smoothly from the remote host with seeking and ID3 tags.
4. **Video Player**:
   - Click a `.mp4` or `.mkv` file on the remote pane and verify HTTP range-request seekable streaming.

---

## 🔌 Step 8: Test Disconnect & Quick Switching

1. On Panel 2 (Remote), click the small **`✕`** icon on the `[⚡ Secondary Test Node]` breadcrumb badge (or click the node button $\rightarrow$ select `Localhost`).
2. Verify Panel 2 smoothly switches back to Localhost and restores your previous local directory.

---

## ☕ Verification Checklist

| Test Item | Status | Notes |
| :--- | :---: | :--- |
| Node registration in Fleet Manager | [ ] | Ping test green |
| Pane 2 switched to remote node | [ ] | Breadcrumb chip & glow visible |
| Remote directory listing & navigation | [ ] | Subfolders load properly |
| Remote folder creation (`F7`) | [ ] | Created on target node |
| Cross-pane file copy (`F5`) | [ ] | Local $\rightarrow$ Remote streamed |
| Cross-pane file move (`F6`) | [ ] | Remote $\rightarrow$ Local streamed |
| Remote file viewing (`F3`) & editing (`F4`) | [ ] | Direct save confirmed |
| SoundDog streaming from remote node | [ ] | Audio plays with seek bar |
| Pane 1-click disconnect | [ ] | Returns to Localhost |

*Have a great morning coffee!* ☕🐕
