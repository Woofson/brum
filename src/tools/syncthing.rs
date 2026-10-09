use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncthingConfig {
    pub enabled: bool,
    pub url: String,
    #[serde(skip_serializing)]
    pub api_key: String,
}

impl Default for SyncthingConfig {
    fn default() -> Self {
        let (auto_url, auto_key) = Self::auto_discover();
        Self {
            enabled: !auto_key.is_empty(),
            url: if auto_url.is_empty() { "http://127.0.0.1:8384".to_string() } else { auto_url },
            api_key: auto_key,
        }
    }
}

impl SyncthingConfig {
    /// Auto-discovers local Syncthing GUI URL and API Key from config.xml
    pub fn auto_discover() -> (String, String) {
        let mut possible_paths: Vec<PathBuf> = Vec::new();
        if let Some(home) = dirs::home_dir() {
            possible_paths.push(home.join(".config/syncthing/config.xml"));
            possible_paths.push(home.join(".local/state/syncthing/config.xml"));
        }
        possible_paths.push(PathBuf::from("/var/lib/syncthing/config.xml"));
        possible_paths.push(PathBuf::from("/etc/syncthing/config.xml"));

        for p in possible_paths {
            if p.exists() {
                if let Ok(content) = std::fs::read_to_string(&p) {
                    let mut api_key = String::new();
                    let mut url = "http://127.0.0.1:8384".to_string();

                    // Simple XML tag extraction
                    if let Some(start) = content.find("<apikey>") {
                        if let Some(end) = content[start..].find("</apikey>") {
                            api_key = content[start + 8..start + end].trim().to_string();
                        }
                    }

                    if let Some(start) = content.find("<address>") {
                        if let Some(end) = content[start..].find("</address>") {
                            let addr = content[start + 9..start + end].trim();
                            if !addr.is_empty() {
                                url = if addr.starts_with("http") {
                                    addr.to_string()
                                } else {
                                    format!("http://{}", addr)
                                };
                            }
                        }
                    }

                    if !api_key.is_empty() {
                        return (url, api_key);
                    }
                }
            }
        }

        ("http://127.0.0.1:8384".to_string(), String::new())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncthingFolderInfo {
    pub id: String,
    pub label: String,
    pub path: String,
    pub state: String, // "idle", "syncing", "scanning", "error"
    pub need_files: u64,
    pub need_bytes: u64,
    pub global_bytes: u64,
    pub in_sync_percent: f64,
    pub paused: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncthingPeerInfo {
    pub id: String,
    pub name: String,
    pub connected: bool,
    pub in_bps: u64,
    pub out_bps: u64,
    pub address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncthingStatusResponse {
    pub available: bool,
    pub message: String,
    pub version: Option<String>,
    pub my_id: Option<String>,
    pub url: String,
    pub folders: Vec<SyncthingFolderInfo>,
    pub peers: Vec<SyncthingPeerInfo>,
    pub total_download_speed: u64, // bytes/sec
    pub total_upload_speed: u64,   // bytes/sec
}

pub struct SyncthingClient {
    config: SyncthingConfig,
    http: reqwest::Client,
}

impl SyncthingClient {
    pub fn new(config: SyncthingConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(5))
                .build()
                .unwrap_or_default(),
        }
    }

    pub async fn get_status(&self) -> SyncthingStatusResponse {
        if self.config.api_key.is_empty() {
            return SyncthingStatusResponse {
                available: false,
                message: "Syncthing API Key not configured. Configure in Settings or ensure Syncthing is running locally.".to_string(),
                version: None,
                my_id: None,
                url: self.config.url.clone(),
                folders: Vec::new(),
                peers: Vec::new(),
                total_download_speed: 0,
                total_upload_speed: 0,
            };
        }

        let base_url = self.config.url.trim_end_matches('/');

        // 1. Fetch System Status (/rest/system/status)
        let sys_res = self.http.get(format!("{}/rest/system/status", base_url))
            .header("X-API-Key", &self.config.api_key)
            .send()
            .await;

        let sys_json = match sys_res {
            Ok(resp) if resp.status().is_success() => resp.json::<serde_json::Value>().await.ok(),
            _ => None,
        };

        if sys_json.is_none() {
            return SyncthingStatusResponse {
                available: false,
                message: format!("Cannot reach Syncthing daemon at {}", base_url),
                version: None,
                my_id: None,
                url: self.config.url.clone(),
                folders: Vec::new(),
                peers: Vec::new(),
                total_download_speed: 0,
                total_upload_speed: 0,
            };
        }

        let sys_val = sys_json.unwrap();
        let my_id = sys_val.get("myID").and_then(|v| v.as_str()).map(|s| s.to_string());

        // 2. Fetch System Connections (/rest/system/connections)
        let mut peers = Vec::new();
        let mut total_down = 0u64;
        let mut total_up = 0u64;

        if let Ok(conn_res) = self.http.get(format!("{}/rest/system/connections", base_url))
            .header("X-API-Key", &self.config.api_key)
            .send()
            .await
        {
            if let Ok(conn_val) = conn_res.json::<serde_json::Value>().await {
                if let Some(conns) = conn_val.get("connections").and_then(|c| c.as_object()) {
                    for (peer_id, p_info) in conns {
                        let connected = p_info.get("connected").and_then(|c| c.as_bool()).unwrap_or(false);
                        let in_bps = p_info.get("inBits").and_then(|b| b.as_u64()).unwrap_or(0) / 8;
                        let out_bps = p_info.get("outBits").and_then(|b| b.as_u64()).unwrap_or(0) / 8;
                        let address = p_info.get("address").and_then(|a| a.as_str()).unwrap_or("").to_string();
                        let name = p_info.get("clientVersion").and_then(|a| a.as_str()).unwrap_or(peer_id).to_string();

                        if connected {
                            total_down += in_bps;
                            total_up += out_bps;
                        }

                        peers.push(SyncthingPeerInfo {
                            id: peer_id.clone(),
                            name,
                            connected,
                            in_bps,
                            out_bps,
                            address,
                        });
                    }
                }
            }
        }

        // 3. Fetch Folders (/rest/config/folders)
        let mut folders = Vec::new();
        if let Ok(folders_res) = self.http.get(format!("{}/rest/config/folders", base_url))
            .header("X-API-Key", &self.config.api_key)
            .send()
            .await
        {
            if let Ok(folders_val) = folders_res.json::<serde_json::Value>().await {
                if let Some(folder_arr) = folders_val.as_array() {
                    for f in folder_arr {
                        let id = f.get("id").and_then(|s| s.as_str()).unwrap_or("").to_string();
                        let label = f.get("label").and_then(|s| s.as_str()).unwrap_or(&id).to_string();
                        let path = f.get("path").and_then(|s| s.as_str()).unwrap_or("").to_string();
                        let paused = f.get("paused").and_then(|p| p.as_bool()).unwrap_or(false);

                        // Fetch Folder DB status
                        let mut state = if paused { "paused".to_string() } else { "idle".to_string() };
                        let mut need_files = 0u64;
                        let mut need_bytes = 0u64;
                        let mut global_bytes = 0u64;
                        let mut in_sync_percent = 100.0;

                        if let Ok(db_res) = self.http.get(format!("{}/rest/db/status?folder={}", base_url, id))
                            .header("X-API-Key", &self.config.api_key)
                            .send()
                            .await
                        {
                            if let Ok(db_val) = db_res.json::<serde_json::Value>().await {
                                state = db_val.get("state").and_then(|s| s.as_str()).unwrap_or(&state).to_string();
                                need_files = db_val.get("needFiles").and_then(|n| n.as_u64()).unwrap_or(0);
                                need_bytes = db_val.get("needBytes").and_then(|n| n.as_u64()).unwrap_or(0);
                                global_bytes = db_val.get("globalBytes").and_then(|n| n.as_u64()).unwrap_or(0);

                                if global_bytes > 0 && need_bytes > 0 {
                                    let sync_fraction = (global_bytes.saturating_sub(need_bytes)) as f64 / global_bytes as f64;
                                    in_sync_percent = (sync_fraction * 100.0).clamp(0.0, 100.0);
                                }
                            }
                        }

                        folders.push(SyncthingFolderInfo {
                            id,
                            label,
                            path,
                            state,
                            need_files,
                            need_bytes,
                            global_bytes,
                            in_sync_percent,
                            paused,
                        });
                    }
                }
            }
        }

        SyncthingStatusResponse {
            available: true,
            message: "Connected to Syncthing daemon".to_string(),
            version: Some("Syncthing API v1".to_string()),
            my_id,
            url: self.config.url.clone(),
            folders,
            peers,
            total_download_speed: total_down,
            total_upload_speed: total_up,
        }
    }

    /// Triggers a rescan of a specific folder or all folders
    pub async fn trigger_scan(&self, folder_id: Option<&str>, subpath: Option<&str>) -> Result<String, String> {
        let base_url = self.config.url.trim_end_matches('/');
        let url = if let Some(fid) = folder_id {
            if let Some(sub) = subpath {
                format!("{}/rest/db/scan?folder={}&sub={}", base_url, fid, sub)
            } else {
                format!("{}/rest/db/scan?folder={}", base_url, fid)
            }
        } else {
            format!("{}/rest/db/scan", base_url)
        };

        let resp = self.http.post(&url)
            .header("X-API-Key", &self.config.api_key)
            .send()
            .await
            .map_err(|e| format!("Failed to send scan request: {}", e))?;

        if resp.status().is_success() {
            Ok("Scan triggered successfully".to_string())
        } else {
            Err(format!("Syncthing scan failed with HTTP {}", resp.status()))
        }
    }
}
