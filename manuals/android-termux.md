# <img src="../assets/brum_commanderdog_legacy.webp" alt="Brum Logo" height="40" style="vertical-align: -6px; margin-right: 8px;" /> Brum on Android & Termux

> **High-Performance Multi-Pane File Commander & Web Environment for Android**  
> *Run Brum directly on unrooted Android devices via Termux, PRoot, UserLAnd, or as a standalone Progressive Web App (PWA).*

---

## 1. Overview & Architecture

Brum is built in pure asynchronous Rust (`axum`, `tokio`, `rust-embed`) and compiles into a single, zero-dependency native binary embedding the entire web interface. 

On Android devices, Brum can run as a local userspace daemon in **Termux** or a **PRoot Linux container**, providing:
- **Direct Android Storage Access**: Manage internal storage (`/sdcard`), DCIM, Downloads, Documents, and external USB OTG drives without root.
- **Progressive Web App (PWA)**: Access `http://127.0.0.1:3000` via Chrome, Firefox, or Brave with full-screen, native-feeling mobile touch UI.
- **Embedded Bite! Terminal**: Spawns an interactive Android shell (`bash`, `zsh`, `sh`) directly inside your browser.
- **Wireless Commander Fleet Node**: Wirelessly browse, edit, and sync phone files from a PC or tablet over local Wi-Fi or Tailscale without MTP cables.

---

## 2. Installation via Termux (Recommended)

[Termux](https://termux.dev) provides a robust Linux environment for Android. Install the official build from [F-Droid](https://f-droid.org/packages/com.termux/) or [GitHub Releases](https://github.com/termux/termux-app/releases) (avoid Google Play Store builds which are deprecated).

### Step 1: Grant Android Storage Permission
Run the following inside Termux and tap **Allow** on the Android system popup:
```bash
termux-setup-storage
```
This maps standard Android storage folders to `~/storage/` inside Termux:
- `~/storage/shared` $\to$ Internal Storage root (`/sdcard`)
- `~/storage/downloads` $\to$ Android Downloads
- `~/storage/dcim` $\to$ Camera & Photos
- `~/storage/documents` $\to$ Android Documents
- `~/storage/external-1` $\to$ External MicroSD / USB OTG drive

---

### Step 2: Install Build Toolchain & Dependencies
```bash
pkg update -y
pkg install -y rust clang openssl make git
```

---

### Step 3: Install Brum

#### Option A: Direct Install via Cargo (Crates.io)
```bash
cargo install brum
```

#### Option B: Compile from Source Repository
```bash
git clone https://github.com/Woofson/brum.git
cd brum
cargo build --release
cp target/release/brum $PREFIX/bin/
```

---

### Step 4: Launch Brum
Start the server listening on localhost:
```bash
brum
```

To expose Brum across your local Wi-Fi network (to browse phone files from a PC):
```bash
brum -b 0.0.0.0:3000
```

---

## 3. Alternative Android Environments

If you prefer using full Linux distributions on Android without Termux's custom package prefix, use one of the following methods:

### A. PRoot Distro inside Termux (Alpine / Debian / Ubuntu)
1. Install `proot-distro` in Termux:
   ```bash
   pkg install proot-distro
   proot-distro install alpine
   proot-distro login alpine
   ```
2. Inside Alpine Linux, install the official Brum `.apk` package:
   ```bash
   apk add --allow-untrusted https://github.com/Woofson/brum/releases/download/v0.8.9/brum-0.8.9-r0.x86_64.apk
   brum -b 0.0.0.0:3000
   ```

### B. UserLAnd / AndroNix
1. Launch an Alpine or Debian session in UserLAnd.
2. Download the generic Linux binary tarball:
   ```bash
   curl -LO https://github.com/Woofson/brum/releases/latest/download/brum-v0.8.9-linux-x86_64.tar.gz
   tar -xzf brum-v0.8.9-linux-x86_64.tar.gz
   cd brum-v0.8.9-linux-x86_64
   ./brum
   ```

---

## 4. Setting Up the Android PWA (Standalone App)

Brum automatically detects mobile screen viewports (`<600px` Phone mode) with single-pane navigation and quick bottom actions.

1. Open **Chrome**, **Firefox**, or **Brave** on your Android device.
2. Navigate to `http://127.0.0.1:3000`.
3. Open the browser menu (three dots `⋮`) and select **"Add to Home screen"** or **"Install App"**.
4. Brum will install as a native-feeling standalone application with no address bars or browser frames.

---

## 5. Background Daemon & Autostart (Termux:Boot)

Android's battery management will eventually kill background processes. Follow these steps to ensure continuous background operation:

### 1. Acquire Wake Lock
Prevent Android from suspending the Termux CPU:
```bash
termux-wake-lock
```

### 2. Run as a Background Service
```bash
nohup brum -b 0.0.0.0:3000 > /dev/null 2>&1 &
```

### 3. Autostart on Device Boot via Termux:Boot
1. Install the **Termux:Boot** add-on from F-Droid.
2. Launch the Termux:Boot app once to register Android broadcast permissions.
3. Create the boot script in Termux:
   ```bash
   mkdir -p ~/.termux/boot
   cat << 'EOF' > ~/.termux/boot/start-brum.sh
   #!/data/data/com.termux/files/usr/bin/bash
   termux-wake-lock
   brum -b 0.0.0.0:3000 > ~/.brum.log 2>&1 &
   EOF
   chmod +x ~/.termux/boot/start-brum.sh
   ```

---

## 6. Configuring Android Storage Roots in `config.toml`

Customize the pre-configured bookmarks and storage roots in `~/.config/brum/config.toml` for Android:

```toml
[server]
bind_address = "0.0.0.0:3000"
default_path = "/data/data/com.termux/files/home/storage/shared"

[storage]
# Pre-configure Android directories in Places & Favorites menu
roots = [
    { name = "Internal Storage", path = "/sdcard" },
    { name = "Downloads", path = "/data/data/com.termux/files/home/storage/downloads" },
    { name = "Photos (DCIM)", path = "/data/data/com.termux/files/home/storage/dcim" },
    { name = "Documents", path = "/data/data/com.termux/files/home/storage/documents" },
    { name = "Termux Home", path = "/data/data/com.termux/files/home" }
]

[terminal]
default_shell = "/data/data/com.termux/files/usr/bin/bash"
```

---

## 7. Accessing from PC over Wi-Fi (Wireless File Commander)

1. Check your phone's local IP in Termux:
   ```bash
   ifconfig wlan0 | grep inet
   ```
   *(Example: `192.168.1.145`)*
2. On your desktop PC or laptop browser, open:
   ```
   http://192.168.1.145:3000
   ```
3. You now have full Total Commander / Orthodox dual-pane control over your Android phone's storage from your desktop PC, including drag-and-drop file transfers, audio streaming, PDF viewing, and hex editing.

---

## 8. Cross-Compiling Android ARM64 Binaries from PC

If you are developing or compiling release builds for Android ARM64 devices from a Linux or macOS host machine, use `cross`:

```bash
# Install cross
cargo install cross --git https://github.com/cross-rs/cross

# Build Android aarch64 binary
cross build --target aarch64-linux-android --release
```

The compiled binary will be located at `target/aarch64-linux-android/release/brum` and can be pushed directly to Android via ADB:
```bash
adb push target/aarch64-linux-android/release/brum /data/local/tmp/
```
