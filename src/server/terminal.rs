use axum::{
    extract::{
        ws::{Message, WebSocket},
        Query, WebSocketUpgrade,
    },
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use serde::Deserialize;
use std::io::{Read, Write};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::error;

#[derive(Deserialize)]
pub struct TerminalQuery {
    pub cwd: Option<String>,
    pub cols: Option<u16>,
    pub rows: Option<u16>,
}

pub async fn handle_terminal_ws(
    ws: WebSocketUpgrade,
    Query(query): Query<TerminalQuery>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_terminal_socket(socket, query))
}

async fn handle_terminal_socket(socket: WebSocket, query: TerminalQuery) {
    let pty_system = native_pty_system();
    let cols = query.cols.unwrap_or(100).max(10);
    let rows = query.rows.unwrap_or(24).max(2);

    let pair = match pty_system.openpty(PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }) {
        Ok(p) => p,
        Err(e) => {
            error!("Failed to open PTY: {}", e);
            return;
        }
    };

    #[cfg(windows)]
    let shell = if let Ok(comspec) = std::env::var("COMSPEC") {
        comspec
    } else {
        "powershell.exe".to_string()
    };
    #[cfg(not(windows))]
    let shell = if std::path::Path::new("/bin/bash").exists() {
        "/bin/bash".to_string()
    } else if std::path::Path::new("/usr/bin/bash").exists() {
        "/usr/bin/bash".to_string()
    } else if std::path::Path::new("/bin/sh").exists() {
        "/bin/sh".to_string()
    } else {
        "/bin/sh".to_string()
    };

    let mut cmd = CommandBuilder::new(&shell);
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    #[cfg(not(windows))]
    {
        cmd.env("LANG", "C.UTF-8");
        cmd.env("LC_ALL", "C.UTF-8");
        cmd.env("SHELL", &shell);
    }

    if let Some(ref cwd) = query.cwd {
        let clean = cwd.trim();
        let expanded = if clean == "~" || clean.starts_with("~/") {
            if let Some(home) = dirs::home_dir() {
                if clean == "~" {
                    home
                } else {
                    home.join(&clean[2..])
                }
            } else {
                std::path::PathBuf::from(clean)
            }
        } else {
            std::path::PathBuf::from(clean)
        };
        if expanded.exists() && expanded.is_dir() {
            cmd.cwd(expanded);
        } else if let Some(home) = dirs::home_dir() {
            if home.exists() && home.is_dir() {
                cmd.cwd(home);
            }
        }
    } else if let Some(home) = dirs::home_dir() {
        if home.exists() && home.is_dir() {
            cmd.cwd(home);
        }
    }

    let mut child = match pair.slave.spawn_command(cmd) {
        Ok(c) => c,
        Err(e) => {
            error!("Failed to spawn shell process in PTY: {}", e);
            return;
        }
    };

    drop(pair.slave);

    let mut reader = match pair.master.try_clone_reader() {
        Ok(r) => r,
        Err(e) => {
            error!("Failed to clone PTY reader: {}", e);
            return;
        }
    };

    let mut writer = match pair.master.take_writer() {
        Ok(w) => w,
        Err(e) => {
            error!("Failed to take PTY writer: {}", e);
            return;
        }
    };

    let (ws_sender, mut ws_receiver) = socket.split();
    let ws_sender = Arc::new(tokio::sync::Mutex::new(ws_sender));

    let (pty_out_tx, mut pty_out_rx) = mpsc::channel::<Vec<u8>>(100);

    // Blocking thread for reading from PTY master
    std::thread::spawn(move || {
        let mut buffer = [0u8; 4096];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    if pty_out_tx.blocking_send(buffer[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    // Task forwarding PTY output -> WebSocket
    let ws_sender_clone = ws_sender.clone();
    let pty_to_ws = tokio::spawn(async move {
        while let Some(data) = pty_out_rx.recv().await {
            let mut sender = ws_sender_clone.lock().await;
            if sender.send(Message::Binary(data)).await.is_err() {
                break;
            }
        }
    });

#[derive(Deserialize, Debug)]
struct TerminalResizePayload {
    cols: u16,
    rows: u16,
    #[serde(default)]
    #[allow(dead_code)]
    resize: Option<bool>,
}

    let master_pty = Arc::new(parking_lot::Mutex::new(pair.master));
    let master_pty_clone = master_pty.clone();

    let mut cur_cols = cols;
    let mut cur_rows = rows;

    // Task forwarding WebSocket input -> PTY master writer
    let ws_to_pty = tokio::spawn(async move {
        while let Some(Ok(msg)) = ws_receiver.next().await {
            match msg {
                Message::Text(text) => {
                    // Check for JSON control message (e.g. resize)
                    if text.starts_with('{') {
                        if let Ok(resize_cmd) = serde_json::from_str::<TerminalResizePayload>(&text) {
                            let target_cols = resize_cmd.cols.max(10);
                            let target_rows = resize_cmd.rows.max(2);
                            if target_cols != cur_cols || target_rows != cur_rows {
                                cur_cols = target_cols;
                                cur_rows = target_rows;
                                let _ = master_pty_clone.lock().resize(PtySize {
                                    rows: target_rows,
                                    cols: target_cols,
                                    pixel_width: 0,
                                    pixel_height: 0,
                                });
                            }
                            continue;
                        }
                    }
                    if writer.write_all(text.as_bytes()).is_err() {
                        break;
                    }
                    let _ = writer.flush();
                }
                Message::Binary(bytes) => {
                    if bytes.starts_with(b"{") {
                        if let Ok(text) = std::str::from_utf8(&bytes) {
                            if let Ok(resize_cmd) = serde_json::from_str::<TerminalResizePayload>(text) {
                                let target_cols = resize_cmd.cols.max(10);
                                let target_rows = resize_cmd.rows.max(2);
                                if target_cols != cur_cols || target_rows != cur_rows {
                                    cur_cols = target_cols;
                                    cur_rows = target_rows;
                                    let _ = master_pty_clone.lock().resize(PtySize {
                                        rows: target_rows,
                                        cols: target_cols,
                                        pixel_width: 0,
                                        pixel_height: 0,
                                    });
                                }
                                continue;
                            }
                        }
                    }
                    if writer.write_all(&bytes).is_err() {
                        break;
                    }
                    let _ = writer.flush();
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
    });

    tokio::select! {
        _ = pty_to_ws => {},
        _ = ws_to_pty => {},
    }

    let _ = child.kill();
}
