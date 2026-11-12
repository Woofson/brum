// ==============================================================================
// Brum - Tray icon (Windows notification area)
// A small part of brum.exe that runs as the signed-in user, shows whether Brum
// is running and opens it quickly. It talks to the running Brum (the Windows
// service or the user's own desktop mode) over HTTP, like the browser does,
// and uses only Windows' own tray functions.
// ==============================================================================

/// Which Brum the tray icon is showing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Target {
    /// The user's own desktop mode (`brum -s`), found through its instance file
    Desktop,
    /// The Windows service of an all-users install
    Service,
    /// Desktop mode started with `--minimized`, which shows its own tray icon
    ThisProcess,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ServiceState {
    NotInstalled,
    Running,
    Stopped,
    /// Starting, stopping or paused
    Busy,
}

#[derive(Clone, Debug)]
pub struct Status {
    pub target: Option<Target>,
    pub reachable: bool,
    pub version: Option<String>,
    /// Plain address, e.g. http://127.0.0.1:3140
    pub address: Option<String>,
    /// Address to open, with the desktop-mode access code when there is one
    pub open_url: Option<String>,
    pub service: ServiceState,
}

impl Status {
    pub fn nothing(service: ServiceState) -> Self {
        Status { target: None, reachable: false, version: None, address: None, open_url: None, service }
    }
}

/// One line of the tray menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuLine {
    /// Command id; 0 for the status line and separators
    pub id: u32,
    pub label: String,
    pub enabled: bool,
    pub separator: bool,
}

pub const ID_OPEN: u32 = 1;
pub const ID_APP_WINDOW: u32 = 2;
pub const ID_COPY: u32 = 3;
pub const ID_LOGS: u32 = 4;
pub const ID_DATA: u32 = 5;
pub const ID_SERVICE_START: u32 = 6;
pub const ID_SERVICE_STOP: u32 = 7;
pub const ID_SERVICE_RESTART: u32 = 8;
pub const ID_QUIT: u32 = 9;

/// The status shown in the tooltip and at the top of the menu.
pub fn status_text(s: &Status) -> String {
    if s.reachable {
        let name = match &s.version {
            Some(v) => format!("Brum {v}"),
            None => "Brum".to_string(),
        };
        return match &s.address {
            Some(a) => format!("{name} - running at {a}"),
            None => format!("{name} - running"),
        };
    }
    match s.service {
        ServiceState::Stopped => "Brum - the service is stopped".to_string(),
        ServiceState::Running => "Brum - the service is not answering".to_string(),
        ServiceState::Busy => "Brum - the service is starting or stopping".to_string(),
        ServiceState::NotInstalled => "Brum - not running".to_string(),
    }
}

/// The tray menu for a status. `in_process` is true when the tray belongs to desktop mode
/// started with `--minimized`, where quitting ends Brum itself.
pub fn menu_lines(s: &Status, in_process: bool) -> Vec<MenuLine> {
    let item = |id: u32, label: &str, enabled: bool| MenuLine { id, label: label.to_string(), enabled, separator: false };
    let sep = || MenuLine { id: 0, label: String::new(), enabled: false, separator: true };
    let can_start_desktop = !s.reachable && s.service == ServiceState::NotInstalled;

    let mut lines = vec![
        MenuLine { id: 0, label: status_text(s), enabled: false, separator: false },
        sep(),
        item(ID_OPEN, if can_start_desktop { "Start Brum" } else { "Open Brum" }, s.reachable || can_start_desktop),
        item(ID_APP_WINDOW, "Open as app window", s.reachable),
        item(ID_COPY, "Copy address", s.reachable),
        sep(),
        item(ID_LOGS, "Open logs folder", true),
        item(ID_DATA, "Open data folder", true),
    ];
    if s.service != ServiceState::NotInstalled {
        lines.push(sep());
        lines.push(item(ID_SERVICE_START, "Start service", s.service == ServiceState::Stopped));
        lines.push(item(ID_SERVICE_STOP, "Stop service", s.service == ServiceState::Running));
        lines.push(item(ID_SERVICE_RESTART, "Restart service", s.service == ServiceState::Running));
    }
    lines.push(sep());
    lines.push(item(ID_QUIT, if in_process { "Quit Brum" } else { "Quit tray icon" }, true));
    lines
}

/// The address without the access code, for showing and copying.
pub fn plain_address(url: &str) -> String {
    url.split('?').next().unwrap_or(url).trim_end_matches('/').to_string()
}

/// Picks the icon entry closest to (and preferably not smaller than) `want` pixels from an
/// .ico file; returns (offset, size) of its image data.
pub fn pick_ico_entry(ico: &[u8], want: u32) -> Option<(usize, usize)> {
    if ico.len() < 6 || u16::from_le_bytes([ico[2], ico[3]]) != 1 {
        return None;
    }
    let count = u16::from_le_bytes([ico[4], ico[5]]) as usize;
    let mut best: Option<(u32, usize, usize)> = None;
    for i in 0..count {
        let e = 6 + i * 16;
        let entry = ico.get(e..e + 16)?;
        let w = if entry[0] == 0 { 256 } else { entry[0] as u32 };
        let size = u32::from_le_bytes([entry[8], entry[9], entry[10], entry[11]]) as usize;
        let offset = u32::from_le_bytes([entry[12], entry[13], entry[14], entry[15]]) as usize;
        if offset.checked_add(size).map_or(true, |end| end > ico.len()) {
            continue;
        }
        // Bigger-or-equal sizes rank by closeness; smaller ones only as a last resort
        let rank = if w >= want { w - want } else { 10_000 + (want - w) };
        if best.map_or(true, |(r, _, _)| rank < r) {
            best = Some((rank, offset, size));
        }
    }
    best.map(|(_, o, s)| (o, s))
}

/// `brum tray [install|uninstall] ...`. Returns when the command is done (or the tray quits).
pub fn run_command(args: &[String]) {
    #[cfg(windows)]
    {
        win::run_command(args);
    }
    #[cfg(not(windows))]
    {
        let _ = args;
        println!("The Brum tray icon is available on Windows only for now.");
    }
}

/// Shows a tray icon for desktop mode started with `--minimized` (Windows only). Runs on its
/// own thread; does nothing when a separate tray icon is already running for this user.
pub fn spawn_in_process(open_url: String) {
    #[cfg(windows)]
    {
        std::thread::spawn(move || win::run_tray(Some(open_url)));
    }
    #[cfg(not(windows))]
    {
        let _ = open_url;
    }
}

#[cfg(windows)]
mod win {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant};
    use tracing::{info, warn};
    use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::System::Threading::CreateMutexW;
    use windows_sys::Win32::UI::Shell::{
        ShellExecuteW, Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
        NIM_SETVERSION, NIN_SELECT, NOTIFYICONDATAW, NOTIFYICON_VERSION_4,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    /// Keyboard selection of the icon (NIN_SELECT | NINF_KEY)
    const NIN_KEYSELECT: u32 = NIN_SELECT | 0x1;
    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const RUN_VALUE: &str = "Brum Tray";
    const WM_TRAY: u32 = WM_APP + 1;
    const WM_STATUS: u32 = WM_APP + 2;
    const POLL_EVERY: Duration = Duration::from_secs(10);

    struct Shared {
        status: Status,
        in_process_url: Option<String>,
    }

    static SHARED: OnceLock<Mutex<Shared>> = OnceLock::new();
    static HWND_RAW: AtomicIsize = AtomicIsize::new(0);
    static HICON_RAW: AtomicIsize = AtomicIsize::new(0);
    static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
    static REFRESH_SOON: AtomicBool = AtomicBool::new(false);

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn exe_path() -> PathBuf {
        std::env::current_exe().unwrap_or_else(|_| PathBuf::from("brum.exe"))
    }

    fn conhost() -> PathBuf {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string());
        Path::new(&root).join("System32").join("conhost.exe")
    }

    /// Starts brum.exe with these arguments in a hidden console, detached from this one.
    fn spawn_hidden(args: &[&str]) -> std::io::Result<()> {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        std::process::Command::new(conhost())
            .arg("--headless")
            .arg(exe_path())
            .args(args)
            .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
            .spawn()
            .map(|_| ())
    }

    fn run_value_command() -> String {
        format!("\"{}\" --headless \"{}\" tray --here", conhost().display(), exe_path().display())
    }

    pub fn run_command(args: &[String]) {
        let all_users = args.iter().any(|a| a == "--all-users");
        let no_start = args.iter().any(|a| a == "--no-start");
        match args.first().map(String::as_str) {
            Some("--here") => run_tray(None),
            None => match spawn_hidden(&["tray", "--here"]) {
                Ok(()) => println!("Brum tray icon started."),
                Err(e) => eprintln!("Could not start the tray icon: {e}"),
            },
            Some("install") => {
                use windows_sys::Win32::System::Registry::{RegSetKeyValueW, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, REG_SZ};
                let root = if all_users { HKEY_LOCAL_MACHINE } else { HKEY_CURRENT_USER };
                let data = wide(&run_value_command());
                let err = unsafe {
                    RegSetKeyValueW(
                        root,
                        wide(RUN_KEY).as_ptr(),
                        wide(RUN_VALUE).as_ptr(),
                        REG_SZ,
                        data.as_ptr() as _,
                        (data.len() * 2) as u32,
                    )
                };
                if err != 0 {
                    eprintln!(
                        "Could not set the tray icon to start at sign-in (Windows error {err}){}",
                        if all_users { "; --all-users needs a terminal opened as administrator" } else { "" }
                    );
                    return;
                }
                println!(
                    "The Brum tray icon now starts at sign-in for {}.",
                    if all_users { "every user of this computer" } else { "this user" }
                );
                if !no_start {
                    let _ = spawn_hidden(&["tray", "--here"]);
                }
            }
            Some("uninstall") => {
                use windows_sys::Win32::System::Registry::{RegDeleteKeyValueW, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
                let mut roots = vec![HKEY_CURRENT_USER];
                if all_users {
                    roots.push(HKEY_LOCAL_MACHINE);
                }
                for root in roots {
                    unsafe { RegDeleteKeyValueW(root, wide(RUN_KEY).as_ptr(), wide(RUN_VALUE).as_ptr()) };
                }
                println!("The Brum tray icon no longer starts at sign-in. A running one can be closed from its menu.");
            }
            _ => {
                println!("Usage: brum tray                         Start the tray icon");
                println!("       brum tray install [--all-users]   Start it at sign-in (all users needs administrator)");
                println!("       brum tray uninstall [--all-users] Stop starting it at sign-in");
            }
        }
    }

    // --------------------------------------------------------------------------
    // What the tray shows
    // --------------------------------------------------------------------------

    fn service_state() -> ServiceState {
        use windows_sys::Win32::System::Services::{
            CloseServiceHandle, OpenSCManagerW, OpenServiceW, QueryServiceStatus, SC_MANAGER_CONNECT, SERVICE_QUERY_STATUS,
            SERVICE_RUNNING, SERVICE_STATUS, SERVICE_STOPPED,
        };
        unsafe {
            let scm = OpenSCManagerW(std::ptr::null(), std::ptr::null(), SC_MANAGER_CONNECT);
            if scm.is_null() {
                return ServiceState::NotInstalled;
            }
            let svc = OpenServiceW(scm, wide("Brum").as_ptr(), SERVICE_QUERY_STATUS);
            if svc.is_null() {
                CloseServiceHandle(scm);
                return ServiceState::NotInstalled;
            }
            let mut st: SERVICE_STATUS = std::mem::zeroed();
            let ok = QueryServiceStatus(svc, &mut st);
            CloseServiceHandle(svc);
            CloseServiceHandle(scm);
            if ok == 0 {
                return ServiceState::Busy;
            }
            match st.dwCurrentState {
                SERVICE_RUNNING => ServiceState::Running,
                SERVICE_STOPPED => ServiceState::Stopped,
                _ => ServiceState::Busy,
            }
        }
    }

    /// The port in the config next to brum.exe, which the service uses.
    fn service_port() -> u16 {
        exe_path()
            .parent()
            .map(|d| d.join("config.toml"))
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| crate::config::ConfigManager::parse_config_str(&t).ok())
            .map(|c| c.server.port)
            .filter(|p| *p != 0)
            .unwrap_or(3140)
    }

    /// Asks Brum for its public information; Some(version) when it answers.
    fn probe(address: &str) -> Option<Option<String>> {
        let client = reqwest::blocking::Client::builder().timeout(Duration::from_secs(3)).build().ok()?;
        let resp = client.get(format!("{address}/api/config")).send().ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let v: serde_json::Value = resp.json().ok()?;
        Some(v.get("version").and_then(|x| x.as_str()).map(str::to_string))
    }

    fn discover(in_process_url: Option<&str>) -> Status {
        let service = service_state();
        let (target, open_url) = if let Some(url) = in_process_url {
            (Target::ThisProcess, url.to_string())
        } else if let Some(inst) = crate::instance::get_active_user_instance() {
            (Target::Desktop, format!("http://127.0.0.1:{}?instance_token={}", inst.port, inst.instance_token))
        } else if service != ServiceState::NotInstalled {
            (Target::Service, format!("http://127.0.0.1:{}", service_port()))
        } else {
            return Status::nothing(service);
        };
        let address = plain_address(&open_url);
        let answer = probe(&address);
        Status {
            target: Some(target),
            reachable: answer.is_some(),
            version: answer.flatten(),
            address: Some(address),
            open_url: Some(open_url),
            service,
        }
    }

    fn folders(target: Option<Target>) -> (PathBuf, PathBuf) {
        if target == Some(Target::Service) {
            if let Some(shared) = crate::win_perms::shared_data_dir() {
                return (shared.join("logs"), shared);
            }
        }
        let cfg = crate::config::ConfigManager::load_all();
        let data = Path::new(&cfg.server.database_path)
            .parent()
            .map(Path::to_path_buf)
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| exe_path().parent().map(Path::to_path_buf).unwrap_or_default());
        (cfg.logging.resolve_log_directory(), data)
    }

    // --------------------------------------------------------------------------
    // The icon and its window
    // --------------------------------------------------------------------------

    fn load_icon() -> HICON {
        static ICO: &[u8] = include_bytes!("../assets/brum.ico");
        unsafe {
            let want = GetSystemMetrics(SM_CXSMICON).max(16) as u32;
            if let Some((off, size)) = pick_ico_entry(ICO, want) {
                let h = CreateIconFromResourceEx(ICO[off..].as_ptr(), size as u32, 1, 0x0003_0000, want as i32, want as i32, LR_DEFAULTCOLOR);
                if !h.is_null() {
                    return h;
                }
            }
            LoadIconW(std::ptr::null_mut(), IDI_APPLICATION)
        }
    }

    fn icon_data(hwnd: HWND) -> NOTIFYICONDATAW {
        let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = 1;
        nid
    }

    fn set_tip(nid: &mut NOTIFYICONDATAW, text: &str) {
        let w: Vec<u16> = text.encode_utf16().take(nid.szTip.len() - 1).collect();
        nid.szTip[..w.len()].copy_from_slice(&w);
        nid.szTip[w.len()] = 0;
    }

    fn current_status() -> Status {
        SHARED
            .get()
            .and_then(|m| m.lock().ok().map(|s| s.status.clone()))
            .unwrap_or_else(|| Status::nothing(ServiceState::NotInstalled))
    }

    fn add_icon(hwnd: HWND) {
        let mut nid = icon_data(hwnd);
        nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
        nid.uCallbackMessage = WM_TRAY;
        nid.hIcon = HICON_RAW.load(Ordering::SeqCst) as HICON;
        set_tip(&mut nid, &status_text(&current_status()));
        unsafe {
            Shell_NotifyIconW(NIM_ADD, &nid);
            nid.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            Shell_NotifyIconW(NIM_SETVERSION, &nid);
        }
    }

    fn update_tip(hwnd: HWND) {
        let mut nid = icon_data(hwnd);
        nid.uFlags = NIF_TIP | NIF_SHOWTIP;
        set_tip(&mut nid, &status_text(&current_status()));
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid) };
    }

    fn remove_icon(hwnd: HWND) {
        let nid = icon_data(hwnd);
        unsafe { Shell_NotifyIconW(NIM_DELETE, &nid) };
    }

    fn shell(verb: &str, file: &str, params: Option<&str>, show: i32) -> bool {
        let verb_w = wide(verb);
        let file_w = wide(file);
        let params_w = params.map(wide);
        let r = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                verb_w.as_ptr(),
                file_w.as_ptr(),
                params_w.as_ref().map_or(std::ptr::null(), |p| p.as_ptr()),
                std::ptr::null(),
                show,
            )
        };
        r as isize > 32
    }

    fn copy_to_clipboard(hwnd: HWND, text: &str) {
        use windows_sys::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
        use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
        const CF_UNICODETEXT: u32 = 13;
        let w = wide(text);
        unsafe {
            if OpenClipboard(hwnd) == 0 {
                return;
            }
            EmptyClipboard();
            let mem = GlobalAlloc(GMEM_MOVEABLE, w.len() * 2);
            if !mem.is_null() {
                let p = GlobalLock(mem) as *mut u16;
                if !p.is_null() {
                    std::ptr::copy_nonoverlapping(w.as_ptr(), p, w.len());
                    GlobalUnlock(mem);
                    SetClipboardData(CF_UNICODETEXT, mem as _);
                }
            }
            CloseClipboard();
        }
    }

    fn run_command_id(hwnd: HWND, id: u32) {
        let s = current_status();
        let in_process = SHARED.get().and_then(|m| m.lock().ok().map(|x| x.in_process_url.is_some())).unwrap_or(false);
        let exe = exe_path().display().to_string();
        match id {
            ID_OPEN => {
                if let (true, Some(url)) = (s.reachable, &s.open_url) {
                    shell("open", url, None, SW_SHOWNORMAL);
                } else if s.service == ServiceState::NotInstalled {
                    // Desktop mode opens the browser itself once it runs
                    if spawn_hidden(&["-s"]).is_ok() {
                        REFRESH_SOON.store(true, Ordering::SeqCst);
                    }
                }
            }
            ID_APP_WINDOW => {
                if let Some(url) = &s.open_url {
                    if !shell("open", "msedge.exe", Some(&format!("--app={url}")), SW_SHOWNORMAL) {
                        shell("open", url, None, SW_SHOWNORMAL);
                    }
                }
            }
            ID_COPY => {
                if let Some(a) = &s.address {
                    copy_to_clipboard(hwnd, a);
                }
            }
            ID_LOGS | ID_DATA => {
                let (logs, data) = folders(s.target);
                let dir = if id == ID_LOGS { logs } else { data };
                let _ = std::fs::create_dir_all(&dir);
                shell("open", &dir.display().to_string(), None, SW_SHOWNORMAL);
            }
            ID_SERVICE_START | ID_SERVICE_STOP | ID_SERVICE_RESTART => {
                let action = match id {
                    ID_SERVICE_START => "start",
                    ID_SERVICE_STOP => "stop",
                    _ => "restart",
                };
                // Windows asks for administrator approval
                if shell("runas", &exe, Some(&format!("service {action}")), SW_HIDE) {
                    REFRESH_SOON.store(true, Ordering::SeqCst);
                }
            }
            ID_QUIT => {
                if in_process {
                    remove_icon(hwnd);
                    crate::instance::unregister_user_instance();
                    std::process::exit(0);
                }
                unsafe { DestroyWindow(hwnd) };
            }
            _ => {}
        }
    }

    fn show_menu(hwnd: HWND) {
        let s = current_status();
        let in_process = SHARED.get().and_then(|m| m.lock().ok().map(|x| x.in_process_url.is_some())).unwrap_or(false);
        unsafe {
            let menu = CreatePopupMenu();
            if menu.is_null() {
                return;
            }
            for line in menu_lines(&s, in_process) {
                if line.separator {
                    AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
                } else {
                    let flags = MF_STRING | if line.enabled { 0 } else { MF_GRAYED };
                    AppendMenuW(menu, flags, line.id as usize, wide(&line.label).as_ptr());
                }
            }
            let mut pt = POINT { x: 0, y: 0 };
            GetCursorPos(&mut pt);
            SetForegroundWindow(hwnd);
            let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY, pt.x, pt.y, 0, hwnd, std::ptr::null());
            DestroyMenu(menu);
            PostMessageW(hwnd, WM_NULL, 0, 0);
            if cmd > 0 {
                run_command_id(hwnd, cmd as u32);
            }
        }
    }

    unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if msg == WM_TRAY {
            match (lparam as u32) & 0xFFFF {
                e if e == NIN_SELECT || e == NIN_KEYSELECT => run_command_id(hwnd, ID_OPEN),
                WM_CONTEXTMENU => show_menu(hwnd),
                _ => {}
            }
            return 0;
        }
        if msg == WM_STATUS {
            update_tip(hwnd);
            return 0;
        }
        let taskbar_created = TASKBAR_CREATED.load(Ordering::SeqCst);
        if taskbar_created != 0 && msg == taskbar_created {
            // Explorer restarted: the icon has to be added again
            add_icon(hwnd);
            return 0;
        }
        if msg == WM_DESTROY {
            remove_icon(hwnd);
            PostQuitMessage(0);
            return 0;
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }

    pub fn run_tray(in_process_url: Option<String>) {
        if !crate::config::ConfigManager::load_all().desktop.enable_tray {
            info!("Tray icon is switched off (enable_tray = false in config.toml)");
            return;
        }
        // One tray icon per signed-in user
        let mutex = unsafe { CreateMutexW(std::ptr::null(), 1, wide(r"Local\BrumTrayIcon").as_ptr()) };
        if mutex.is_null() || unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            if in_process_url.is_none() {
                println!("The Brum tray icon is already running.");
            }
            return;
        }

        let _ = SHARED.set(Mutex::new(Shared {
            status: Status::nothing(ServiceState::NotInstalled),
            in_process_url: in_process_url.clone(),
        }));

        unsafe {
            let hinst = GetModuleHandleW(std::ptr::null());
            let class = wide("BrumTrayIcon");
            let mut wc: WNDCLASSW = std::mem::zeroed();
            wc.lpfnWndProc = Some(wndproc);
            wc.hInstance = hinst;
            wc.lpszClassName = class.as_ptr();
            RegisterClassW(&wc);
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                wide("Brum").as_ptr(),
                WS_OVERLAPPED,
                0,
                0,
                0,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                hinst,
                std::ptr::null(),
            );
            if hwnd.is_null() {
                warn!("Could not create the tray icon window");
                return;
            }
            HWND_RAW.store(hwnd as isize, Ordering::SeqCst);
            HICON_RAW.store(load_icon() as isize, Ordering::SeqCst);
            TASKBAR_CREATED.store(RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()), Ordering::SeqCst);
            add_icon(hwnd);
        }

        // Status check on its own thread, so the menu never waits for the network
        std::thread::spawn(move || {
            let mut last = Instant::now() - POLL_EVERY;
            loop {
                let soon = REFRESH_SOON.swap(false, Ordering::SeqCst);
                if soon {
                    // Give a starting or stopping Brum a moment
                    std::thread::sleep(Duration::from_secs(3));
                }
                if soon || last.elapsed() >= POLL_EVERY {
                    let status = discover(in_process_url.as_deref());
                    if let Some(m) = SHARED.get() {
                        if let Ok(mut s) = m.lock() {
                            s.status = status;
                        }
                    }
                    let hwnd = HWND_RAW.load(Ordering::SeqCst) as HWND;
                    unsafe { PostMessageW(hwnd, WM_STATUS, 0, 0) };
                    last = Instant::now();
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        });

        unsafe {
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(reachable: bool, service: ServiceState) -> Status {
        Status {
            target: Some(Target::Service),
            reachable,
            version: reachable.then(|| "1.22.0".to_string()),
            address: Some("http://127.0.0.1:3140".to_string()),
            open_url: Some("http://127.0.0.1:3140".to_string()),
            service,
        }
    }

    #[test]
    fn test_status_text() {
        assert_eq!(status_text(&status(true, ServiceState::Running)), "Brum 1.22.0 - running at http://127.0.0.1:3140");
        assert_eq!(status_text(&status(false, ServiceState::Stopped)), "Brum - the service is stopped");
        assert_eq!(status_text(&Status::nothing(ServiceState::NotInstalled)), "Brum - not running");
        // Fits the Windows tooltip (127 characters)
        assert!(status_text(&status(true, ServiceState::Running)).chars().count() < 128);
    }

    #[test]
    fn test_menu_for_running_service() {
        let lines = menu_lines(&status(true, ServiceState::Running), false);
        let find = |id: u32| lines.iter().find(|l| l.id == id).cloned().unwrap();
        assert!(find(ID_OPEN).enabled && find(ID_OPEN).label == "Open Brum");
        assert!(find(ID_COPY).enabled);
        assert!(!find(ID_SERVICE_START).enabled);
        assert!(find(ID_SERVICE_STOP).enabled && find(ID_SERVICE_RESTART).enabled);
        assert_eq!(find(ID_QUIT).label, "Quit tray icon");
        assert!(!lines[0].enabled, "the status line is not a command");
    }

    #[test]
    fn test_menu_when_nothing_runs() {
        // No service and no desktop mode: offer to start desktop mode, no service lines
        let lines = menu_lines(&Status::nothing(ServiceState::NotInstalled), true);
        let open = lines.iter().find(|l| l.id == ID_OPEN).unwrap();
        assert!(open.enabled && open.label == "Start Brum");
        assert!(!lines.iter().any(|l| l.id == ID_SERVICE_START));
        assert!(!lines.iter().find(|l| l.id == ID_APP_WINDOW).unwrap().enabled);
        assert_eq!(lines.iter().find(|l| l.id == ID_QUIT).unwrap().label, "Quit Brum");

        // Service stopped: it can be started, Brum cannot be opened yet
        let lines = menu_lines(&status(false, ServiceState::Stopped), false);
        assert!(lines.iter().find(|l| l.id == ID_SERVICE_START).unwrap().enabled);
        assert!(!lines.iter().find(|l| l.id == ID_OPEN).unwrap().enabled);
    }

    #[test]
    fn test_plain_address_drops_access_code() {
        assert_eq!(plain_address("http://127.0.0.1:52011?instance_token=abc"), "http://127.0.0.1:52011");
        assert_eq!(plain_address("http://127.0.0.1:3140/"), "http://127.0.0.1:3140");
    }

    #[test]
    fn test_pick_ico_entry_from_brum_icon() {
        let ico = include_bytes!("../assets/brum.ico");
        let size_of = |want: u32| {
            let (off, size) = pick_ico_entry(ico, want).expect("icon entry");
            assert_eq!(&ico[off..off + 4], b"\x89PNG");
            size
        };
        // 16 px exists exactly; 20 px rounds up to 24; huge falls back to the largest
        assert_eq!(size_of(16), 878);
        assert_eq!(size_of(20), 1662);
        assert_eq!(size_of(512), 80504);
        assert!(pick_ico_entry(b"not an icon", 16).is_none());
    }
}
