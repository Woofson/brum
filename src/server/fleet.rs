use axum::{
    body::Body,
    extract::{
        ws::{Message as AxumMessage, WebSocket},
        Path as AxumPath, Query, State, WebSocketUpgrade,
    },
    http::{header, HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri},
    response::Response,
    Json,
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio_tungstenite::tungstenite::Message as TungsteniteMessage;
use tracing::{info, warn};

use crate::auth::Claims;
use crate::config::FleetNodeConfig;
use crate::server::AppState;

static HTTP_CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();

fn get_http_client() -> &'static reqwest::Client {
    HTTP_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .pool_idle_timeout(Duration::from_secs(90))
            .tcp_nodelay(true)
            .build()
            .expect("Failed to initialize fleet proxy HTTP client")
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedFleetTarget {
    pub id: String,
    pub name: String,
    pub endpoint_url: String,
    pub auth_token: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FleetPingResult {
    pub status: String,
    pub latency_ms: Option<u64>,
    pub endpoint_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Helper function to URL-encode query string values
pub fn urlencoding_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b'~' {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

/// Identifies hop-by-hop HTTP headers that must not be blindly forwarded by reverse proxies
pub fn is_hop_by_hop(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailers"
            | "transfer-encoding"
            | "upgrade"
            | "sec-websocket-key"
            | "sec-websocket-version"
            | "sec-websocket-extensions"
            | "sec-websocket-protocol"
            | "sec-websocket-accept"
            | "x-fleet-target-url"
            | "x-fleet-target-token"
            | "host"
    )
}

/// Extracts authenticated claims for incoming fleet gateway requests
pub fn extract_fleet_claims(
    state: &AppState,
    headers: &HeaderMap,
    query: &HashMap<String, String>,
) -> Result<Claims, (StatusCode, String)> {
    if !state.config.server.enable_auth || state.config.server.standalone {
        let current_user = std::env::var("USERNAME")
            .or_else(|_| std::env::var("USER"))
            .unwrap_or_else(|_| "user".to_string());
        let home_dir = dirs::home_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "/".to_string());
        return Ok(Claims {
            sub: current_user,
            role: "admin".to_string(),
            home_dir,
            is_pam: false,
            allowed_roots: Some("[\"*\"]".to_string()),
            token_id: None,
            exp: 9999999999,
        });
    }

    // 1. Check Authorization: Bearer <token>
    if let Some(auth_header) = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) {
        if let Some(token_str) = auth_header.strip_prefix("Bearer ") {
            if let Ok(claims) = state.auth.verify_token(token_str.trim()) {
                return Ok(claims);
            }
        }
    }

    // 2. Check query parameter ?token=... or ?cd_token=... or ?auth=...
    if let Some(tok) = query.get("token").or_else(|| query.get("cd_token")).or_else(|| query.get("auth")) {
        if let Ok(claims) = state.auth.verify_token(tok.trim()) {
            return Ok(claims);
        }
    }

    // 3. Check Cookie header for cd_token=... or token=...
    if let Some(cookie_hdr) = headers.get(header::COOKIE).and_then(|v| v.to_str().ok()) {
        for pair in cookie_hdr.split(';') {
            let pair = pair.trim();
            if let Some(tok) = pair.strip_prefix("cd_token=").or_else(|| pair.strip_prefix("token=")) {
                if let Ok(claims) = state.auth.verify_token(tok.trim()) {
                    return Ok(claims);
                }
            }
        }
    }

    Err((
        StatusCode::UNAUTHORIZED,
        "Unauthorized: Active session or bearer token required to access fleet gateway proxy".to_string(),
    ))
}

/// Resolves a fleet node by id from static configuration
pub fn resolve_target_node(
    state: &AppState,
    node_id: &str,
    _headers: &HeaderMap,
) -> Option<ResolvedFleetTarget> {
    // Strictly check configured fleet nodes by ID to prevent arbitrary SSRF
    state.config.fleet.nodes.iter().find(|n| n.id == node_id).map(|n| {
        ResolvedFleetTarget {
            id: n.id.clone(),
            name: n.name.clone(),
            endpoint_url: n.url.trim_end_matches('/').to_string(),
            auth_token: n.token.clone(),
        }
    })
}

/// Parses URI query into a key-value hash map
fn parse_query_map(uri: &Uri) -> HashMap<String, String> {
    uri.query()
        .map(|q| {
            let mut map = HashMap::new();
            for pair in q.split('&') {
                let mut parts = pair.splitn(2, '=');
                if let Some(k) = parts.next() {
                    let v = parts.next().unwrap_or("");
                    map.insert(k.to_string(), v.to_string());
                }
            }
            map
        })
        .unwrap_or_default()
}

/// Handles wildcard subpath HTTP proxy requests: `/api/fleet/proxy/:node_id/*path`
pub async fn handle_fleet_http_proxy(
    State(state): State<AppState>,
    headers: HeaderMap,
    method: Method,
    uri: Uri,
    AxumPath((node_id, path)): AxumPath<(String, String)>,
    body: Body,
) -> Result<Response, (StatusCode, String)> {
    forward_fleet_http_request(&state, &node_id, &path, headers, method, uri, body).await
}

/// Handles root HTTP proxy requests: `/api/fleet/proxy/:node_id`
pub async fn handle_fleet_http_proxy_root(
    State(state): State<AppState>,
    headers: HeaderMap,
    method: Method,
    uri: Uri,
    AxumPath(node_id): AxumPath<String>,
    body: Body,
) -> Result<Response, (StatusCode, String)> {
    forward_fleet_http_request(&state, &node_id, "", headers, method, uri, body).await
}

/// Forwards arbitrary HTTP request to remote fleet node with zero memory buffering
async fn forward_fleet_http_request(
    state: &AppState,
    node_id: &str,
    subpath: &str,
    headers: HeaderMap,
    method: Method,
    uri: Uri,
    body: Body,
) -> Result<Response, (StatusCode, String)> {
    let query_map = parse_query_map(&uri);

    // 1. Authenticate caller on gateway
    let _claims = extract_fleet_claims(state, &headers, &query_map)?;

    // 2. Resolve destination node
    let target = resolve_target_node(state, node_id, &headers).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            format!("Fleet node '{}' not found or target URL missing", node_id),
        )
    })?;

    // 3. Build destination URL
    let clean_subpath = subpath.trim_start_matches('/');
    let query_str = uri.query().map(|q| format!("?{}", q)).unwrap_or_default();
    let target_url = if clean_subpath.is_empty() {
        format!("{}{}", target.endpoint_url, query_str)
    } else {
        format!("{}/{}{}", target.endpoint_url, clean_subpath, query_str)
    };

    // 4. Translate Method
    let reqwest_method = reqwest::Method::from_bytes(method.as_str().as_bytes())
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid HTTP method: {}", e)))?;

    // 5. Build outgoing request
    let client = get_http_client();
    let mut req_builder = client.request(reqwest_method, &target_url);

    // Copy incoming headers (filtering hop-by-hop headers)
    for (key, val) in &headers {
        let key_str = key.as_str();
        if !is_hop_by_hop(key_str) && !key_str.eq_ignore_ascii_case("authorization") {
            if let Ok(reqwest_name) = reqwest::header::HeaderName::from_bytes(key.as_str().as_bytes()) {
                if let Ok(reqwest_val) = reqwest::header::HeaderValue::from_bytes(val.as_bytes()) {
                    req_builder = req_builder.header(reqwest_name, reqwest_val);
                }
            }
        }
    }

    // Set Authorization header for remote node
    if let Some(ref tok) = target.auth_token {
        if !tok.trim().is_empty() {
            req_builder = req_builder.header(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", tok.trim()),
            );
        }
    } else if let Some(auth_hdr) = headers.get(header::AUTHORIZATION) {
        if let Ok(val_str) = auth_hdr.to_str() {
            req_builder = req_builder.header(reqwest::header::AUTHORIZATION, val_str);
        }
    }

    // Stream body
    let body_stream = body.into_data_stream();
    req_builder = req_builder.body(reqwest::Body::wrap_stream(body_stream));

    // 6. Execute request
    let resp = req_builder.send().await.map_err(|e| {
        warn!("Fleet proxy request to '{}' failed: {}", target_url, e);
        (
            StatusCode::BAD_GATEWAY,
            format!("Failed to reach remote fleet node '{}' ({})", target.name, e),
        )
    })?;

    // 7. Convert reqwest response to Axum response
    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::OK);
    let mut forward_headers = HeaderMap::new();

    for (key, val) in resp.headers() {
        let key_str = key.as_str();
        if !is_hop_by_hop(key_str) {
            if let (Ok(hname), Ok(hval)) = (
                HeaderName::from_bytes(key_str.as_bytes()),
                HeaderValue::from_bytes(val.as_bytes()),
            ) {
                forward_headers.insert(hname, hval);
            }
        }
    }

    // Stream response body back with zero buffering
    let stream = resp.bytes_stream();
    let axum_body = Body::from_stream(stream);
    let mut response = Response::new(axum_body);
    *response.status_mut() = status;
    *response.headers_mut() = forward_headers;

    Ok(response)
}

/// Handles WebSocket proxy connections for remote Bite! Terminal and live streams
pub async fn handle_fleet_ws_proxy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    AxumPath(node_id): AxumPath<String>,
    ws: WebSocketUpgrade,
) -> Result<Response, (StatusCode, String)> {
    let claims = extract_fleet_claims(&state, &headers, &query)?;

    if !crate::server::terminal::is_role_permitted(&claims.role, &state.config.terminal.allow_roles) {
        return Err((
            StatusCode::FORBIDDEN,
            "Terminal access is restricted for your user role".to_string(),
        ));
    }

    let target = resolve_target_node(&state, &node_id, &headers).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            format!("Fleet node '{}' not found", node_id),
        )
    })?;

    // Convert http:// to ws://, https:// to wss://
    let ws_base = if target.endpoint_url.starts_with("https://") {
        target.endpoint_url.replacen("https://", "wss://", 1)
    } else if target.endpoint_url.starts_with("http://") {
        target.endpoint_url.replacen("http://", "ws://", 1)
    } else if target.endpoint_url.starts_with("wss://") || target.endpoint_url.starts_with("ws://") {
        target.endpoint_url.clone()
    } else {
        format!("ws://{}", target.endpoint_url)
    };

    // Reconstruct query parameters for terminal
    let mut query_pairs: Vec<String> = query
        .iter()
        .filter(|(k, _)| *k != "token" && *k != "cd_token" && *k != "auth")
        .map(|(k, v)| format!("{}={}", urlencoding_encode(k), urlencoding_encode(v)))
        .collect();

    if let Some(ref tok) = target.auth_token {
        if !tok.trim().is_empty() {
            query_pairs.push(format!("token={}", urlencoding_encode(tok.trim())));
        }
    } else if let Some(tok) = query.get("token").or_else(|| query.get("cd_token")) {
        query_pairs.push(format!("token={}", urlencoding_encode(tok.trim())));
    }

    let q_str = if query_pairs.is_empty() {
        String::new()
    } else {
        format!("?{}", query_pairs.join("&"))
    };

    let remote_ws_url = format!("{}/api/terminal/ws{}", ws_base, q_str);

    Ok(ws.on_upgrade(move |client_socket| async move {
        tunnel_websocket(client_socket, remote_ws_url).await;
    }))
}

/// Asynchronously tunnels full duplex WebSocket frames between client and remote node
async fn tunnel_websocket(client_socket: WebSocket, remote_ws_url: String) {
    info!("Establishing WebSocket fleet tunnel to {}", remote_ws_url);

    let (remote_socket, _) = match tokio_tungstenite::connect_async(&remote_ws_url).await {
        Ok(res) => res,
        Err(e) => {
            warn!(
                "Failed to establish remote fleet WebSocket connection to {}: {}",
                remote_ws_url, e
            );
            return;
        }
    };

    let (mut client_tx, mut client_rx) = client_socket.split();
    let (mut remote_tx, mut remote_rx) = remote_socket.split();

    // Client -> Remote
    let client_to_remote = async {
        while let Some(msg_res) = client_rx.next().await {
            match msg_res {
                Ok(AxumMessage::Text(t)) => {
                    if remote_tx.send(TungsteniteMessage::Text(t.into())).await.is_err() {
                        break;
                    }
                }
                Ok(AxumMessage::Binary(b)) => {
                    if remote_tx.send(TungsteniteMessage::Binary(b.into())).await.is_err() {
                        break;
                    }
                }
                Ok(AxumMessage::Ping(p)) => {
                    if remote_tx.send(TungsteniteMessage::Ping(p.into())).await.is_err() {
                        break;
                    }
                }
                Ok(AxumMessage::Pong(p)) => {
                    if remote_tx.send(TungsteniteMessage::Pong(p.into())).await.is_err() {
                        break;
                    }
                }
                Ok(AxumMessage::Close(c)) => {
                    let _ = remote_tx
                        .send(TungsteniteMessage::Close(c.map(|frame| {
                            tokio_tungstenite::tungstenite::protocol::CloseFrame {
                                code: tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::from(
                                    frame.code,
                                ),
                                reason: frame.reason.into(),
                            }
                        })))
                        .await;
                    break;
                }
                Err(_) => break,
            }
        }
    };

    // Remote -> Client
    let remote_to_client = async {
        while let Some(msg_res) = remote_rx.next().await {
            match msg_res {
                Ok(TungsteniteMessage::Text(t)) => {
                    if client_tx.send(AxumMessage::Text(t.to_string())).await.is_err() {
                        break;
                    }
                }
                Ok(TungsteniteMessage::Binary(b)) => {
                    if client_tx.send(AxumMessage::Binary(b.into())).await.is_err() {
                        break;
                    }
                }
                Ok(TungsteniteMessage::Ping(p)) => {
                    if client_tx.send(AxumMessage::Ping(p.into())).await.is_err() {
                        break;
                    }
                }
                Ok(TungsteniteMessage::Pong(p)) => {
                    if client_tx.send(AxumMessage::Pong(p.into())).await.is_err() {
                        break;
                    }
                }
                Ok(TungsteniteMessage::Close(c)) => {
                    let _ = client_tx
                        .send(AxumMessage::Close(c.map(|frame| {
                            axum::extract::ws::CloseFrame {
                                code: frame.code.into(),
                                reason: frame.reason.to_string().into(),
                            }
                        })))
                        .await;
                    break;
                }
                Ok(TungsteniteMessage::Frame(_)) => {}
                Err(_) => break,
            }
        }
    };

    tokio::select! {
        _ = client_to_remote => {},
        _ = remote_to_client => {},
    }
}

/// Gateway node diagnostic ping to check remote node health from backend
pub async fn handle_fleet_ping(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    AxumPath(node_id): AxumPath<String>,
) -> Result<Json<FleetPingResult>, (StatusCode, String)> {
    let _ = extract_fleet_claims(&state, &headers, &query)?;

    let target = resolve_target_node(&state, &node_id, &headers).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            format!("Fleet node '{}' not found", node_id),
        )
    })?;

    let health_url = format!("{}/api/health", target.endpoint_url);
    let client = get_http_client();
    let mut req_builder = client.get(&health_url).timeout(Duration::from_secs(4));

    if let Some(ref tok) = target.auth_token {
        req_builder = req_builder.header(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {}", tok.trim()),
        );
    }

    let start = Instant::now();
    match req_builder.send().await {
        Ok(resp) => {
            let latency_ms = start.elapsed().as_millis() as u64;
            if resp.status().is_success() {
                let json_data: serde_json::Value =
                    resp.json().await.unwrap_or_else(|_| serde_json::json!({}));
                Ok(Json(FleetPingResult {
                    status: "online".to_string(),
                    latency_ms: Some(latency_ms),
                    endpoint_url: target.endpoint_url,
                    data: Some(json_data),
                    error: None,
                }))
            } else if resp.status() == reqwest::StatusCode::UNAUTHORIZED
                || resp.status() == reqwest::StatusCode::FORBIDDEN
            {
                Ok(Json(FleetPingResult {
                    status: "unauthorized".to_string(),
                    latency_ms: Some(latency_ms),
                    endpoint_url: target.endpoint_url,
                    data: None,
                    error: Some(format!("HTTP {}", resp.status())),
                }))
            } else {
                Ok(Json(FleetPingResult {
                    status: "offline".to_string(),
                    latency_ms: Some(latency_ms),
                    endpoint_url: target.endpoint_url,
                    data: None,
                    error: Some(format!("HTTP {}", resp.status())),
                }))
            }
        }
        Err(e) => Ok(Json(FleetPingResult {
            status: "offline".to_string(),
            latency_ms: None,
            endpoint_url: target.endpoint_url,
            data: None,
            error: Some(e.to_string()),
        })),
    }
}

/// Returns list of static and dynamically configured fleet nodes
pub async fn handle_list_fleet_nodes(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<Vec<FleetNodeConfig>>, (StatusCode, String)> {
    let _ = extract_fleet_claims(&state, &headers, &query)?;
    Ok(Json(state.config.fleet.nodes.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_hop_by_hop() {
        assert!(is_hop_by_hop("Connection"));
        assert!(is_hop_by_hop("TRANSFER-ENCODING"));
        assert!(is_hop_by_hop("Upgrade"));
        assert!(is_hop_by_hop("Host"));
        assert!(is_hop_by_hop("X-Fleet-Target-Url"));
        assert!(!is_hop_by_hop("Content-Type"));
        assert!(!is_hop_by_hop("Content-Length"));
        assert!(!is_hop_by_hop("Accept"));
    }

    #[test]
    fn test_urlencoding_encode() {
        assert_eq!(urlencoding_encode("hello world"), "hello%20world");
        assert_eq!(urlencoding_encode("abc-123_.~"), "abc-123_.~");
        assert_eq!(urlencoding_encode("/path/to/file"), "%2Fpath%2Fto%2Ffile");
    }

    #[test]
    fn test_parse_query_map() {
        let uri: Uri = "/api/test?token=abc123xyz&cols=80&rows=24".parse().unwrap();
        let map = parse_query_map(&uri);
        assert_eq!(map.get("token"), Some(&"abc123xyz".to_string()));
        assert_eq!(map.get("cols"), Some(&"80".to_string()));
        assert_eq!(map.get("rows"), Some(&"24".to_string()));
    }

    fn mock_app_state(mut config: crate::config::AppConfig) -> AppState {
        use std::sync::Arc;
        let tmp = tempfile::tempdir().unwrap();
        config.server.database_path = tmp.path().join("fleet_test.db").to_string_lossy().to_string();
        let auth = crate::auth::AuthManager::new(
            &config.server.database_path,
            &config.server.jwt_secret,
            config.server.session_duration_hours,
            &config.auth.mode,
            &config.auth.pam_service,
            &config.auth.default_admin_user,
            &config.auth.default_admin_pass,
        ).unwrap();
        let db = auth.db();
        let auth_arc = Arc::new(auth);
        let task_mgr = Arc::new(crate::tools::tasks::TaskManager::new());
        let tag_mgr = Arc::new(crate::tools::tags::TagManager::new(db.clone()).unwrap());
        let vault_mgr = Arc::new(crate::vfs::vault::VaultManager::new());
        let backup_mgr = Arc::new(crate::tools::sync::BackupManager::new(db).unwrap());
        let plugin_mgr = Arc::new(crate::plugins::PluginManager::new(
            std::path::PathBuf::from("/tmp/system_plugins"),
            std::path::PathBuf::from("/tmp/user_plugins"),
            false,
            "allow_all".to_string(),
            vec!["*".to_string()],
            vec![],
        ));

        let oidc_mgr = Arc::new(crate::auth::oidc::OidcManager::new(config.auth.oidc.clone(), auth_arc.clone()));

        AppState {
            config: Arc::new(config),
            auth: auth_arc,
            oidc: oidc_mgr,
            tasks: task_mgr,
            tags: tag_mgr,
            vaults: vault_mgr,
            backup: backup_mgr,
            plugins: plugin_mgr,
        }
    }

    #[test]
    fn test_resolve_target_node_from_headers() {
        let mut config = crate::config::AppConfig::default();
        config.server.enable_auth = false;
        let state = mock_app_state(config);

        // 1. Without header -> None
        let headers_empty = HeaderMap::new();
        assert!(resolve_target_node(&state, "unknown-node", &headers_empty).is_none());

        // 2. Unconfigured ad-hoc headers are rejected to prevent SSRF
        let mut headers_with_url = HeaderMap::new();
        headers_with_url.insert("X-Fleet-Target-Url", HeaderValue::from_static("http://192.168.1.50:3140/"));
        headers_with_url.insert("X-Fleet-Target-Token", HeaderValue::from_static("jwt-token-1234"));

        assert!(resolve_target_node(&state, "my-nas", &headers_with_url).is_none());
    }

    #[test]
    fn test_resolve_target_node_from_static_config() {
        let mut config = crate::config::AppConfig::default();
        config.fleet.nodes.push(FleetNodeConfig {
            id: "proxmox-104".to_string(),
            name: "Proxmox LXC".to_string(),
            url: "http://192.168.1.104:3140".to_string(),
            token: Some("secret-token".to_string()),
            start_path: Some("/data".to_string()),
            read_only: false,
            color_accent: Some("emerald".to_string()),
            tags: vec!["proxmox".to_string(), "prod".to_string()],
        });
        let state = mock_app_state(config);

        let headers = HeaderMap::new();
        let target = resolve_target_node(&state, "proxmox-104", &headers).unwrap();
        assert_eq!(target.id, "proxmox-104");
        assert_eq!(target.name, "Proxmox LXC");
        assert_eq!(target.endpoint_url, "http://192.168.1.104:3140");
        assert_eq!(target.auth_token, Some("secret-token".to_string()));
    }

    #[test]
    fn test_extract_fleet_claims_standalone() {
        let mut config = crate::config::AppConfig::default();
        config.server.enable_auth = false;
        let state = mock_app_state(config);

        let headers = HeaderMap::new();
        let query = HashMap::new();
        let claims = extract_fleet_claims(&state, &headers, &query).unwrap();
        assert_eq!(claims.role, "admin");
    }
}
