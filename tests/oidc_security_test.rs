use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use brum::auth::oidc::{OidcMetadata, OidcManager};
use brum::auth::AuthManager;
use brum::config::OidcConfig;
use serde_json::json;
use std::sync::Arc;
use tempfile::tempdir;

fn build_mock_jwt(payload: &serde_json::Value) -> String {
    let header = json!({
        "alg": "RS256",
        "typ": "JWT",
        "kid": "key1"
    });
    let header_b64 = URL_SAFE_NO_PAD.encode(header.to_string().as_bytes());
    let payload_b64 = URL_SAFE_NO_PAD.encode(payload.to_string().as_bytes());
    let sig_b64 = URL_SAFE_NO_PAD.encode(b"mock_signature_bytes");
    format!("{}.{}.{}", header_b64, payload_b64, sig_b64)
}

fn create_test_oidc_manager() -> (OidcManager, tempfile::TempDir) {
    let tmp = tempdir().unwrap();
    let db_path = tmp.path().join("test_oidc.db").to_string_lossy().to_string();
    let auth = Arc::new(
        AuthManager::new(
            &db_path,
            "secret",
            72,
            "builtin",
            "login",
            "admin",
            "AdminSecret123!",
        )
        .unwrap(),
    );

    let mut config = OidcConfig::default();
    config.enabled = true;
    config.client_id = "brum-test-client".to_string();
    config.issuer_url = "https://auth.example.com".to_string();

    let manager = OidcManager::new(config, auth);
    (manager, tmp)
}

#[tokio::test]
async fn test_oidc_fails_closed_when_jwks_validation_fails() {
    let (manager, _tmp) = create_test_oidc_manager();
    let mut meta = OidcMetadata::default();
    meta.issuer = "https://auth.example.com".to_string();
    // JWKS URI is configured with an unreachable/invalid endpoint
    meta.jwks_uri = Some("http://127.0.0.1:1/jwks.json".to_string());

    let payload = json!({
        "iss": "https://auth.example.com",
        "aud": "brum-test-client",
        "sub": "user123",
        "nonce": "test-nonce-123",
        "exp": chrono::Utc::now().timestamp() + 3600
    });
    let token = build_mock_jwt(&payload);

    let res = manager.verify_id_token_claims(&token, &meta, "test-nonce-123").await;
    assert!(res.is_err(), "Must fail closed when JWKS validation cannot verify cryptographic signature");
    let err_msg = res.unwrap_err().to_string();
    assert!(err_msg.contains("signature verification failed"));
}

#[tokio::test]
async fn test_oidc_rejects_missing_nonce() {
    let (manager, _tmp) = create_test_oidc_manager();
    let mut meta = OidcMetadata::default();
    meta.issuer = "https://auth.example.com".to_string();
    meta.jwks_uri = None;

    // Token without a nonce claim
    let payload = json!({
        "iss": "https://auth.example.com",
        "aud": "brum-test-client",
        "sub": "user123",
        "exp": chrono::Utc::now().timestamp() + 3600
    });
    let token = build_mock_jwt(&payload);

    let res = manager.verify_id_token_claims(&token, &meta, "expected-nonce").await;
    assert!(res.is_err(), "Must reject token missing nonce claim");
    assert!(res.unwrap_err().to_string().contains("missing required nonce claim"));
}

#[tokio::test]
async fn test_oidc_rejects_mismatched_nonce() {
    let (manager, _tmp) = create_test_oidc_manager();
    let mut meta = OidcMetadata::default();
    meta.issuer = "https://auth.example.com".to_string();
    meta.jwks_uri = None;

    let payload = json!({
        "iss": "https://auth.example.com",
        "aud": "brum-test-client",
        "sub": "user123",
        "nonce": "attacker-injected-nonce",
        "exp": chrono::Utc::now().timestamp() + 3600
    });
    let token = build_mock_jwt(&payload);

    let res = manager.verify_id_token_claims(&token, &meta, "valid-session-nonce").await;
    assert!(res.is_err(), "Must reject token with mismatched nonce");
    assert!(res.unwrap_err().to_string().contains("nonce mismatch"));
}

#[tokio::test]
async fn test_oidc_rejects_audience_mismatch() {
    let (manager, _tmp) = create_test_oidc_manager();
    let mut meta = OidcMetadata::default();
    meta.issuer = "https://auth.example.com".to_string();
    meta.jwks_uri = None;

    let payload = json!({
        "iss": "https://auth.example.com",
        "aud": "different-foreign-app",
        "sub": "user123",
        "nonce": "valid-session-nonce",
        "exp": chrono::Utc::now().timestamp() + 3600
    });
    let token = build_mock_jwt(&payload);

    let res = manager.verify_id_token_claims(&token, &meta, "valid-session-nonce").await;
    assert!(res.is_err(), "Must reject token intended for a different audience");
    assert!(res.unwrap_err().to_string().contains("audience mismatch"));
}

#[tokio::test]
async fn test_oidc_rejects_expired_token() {
    let (manager, _tmp) = create_test_oidc_manager();
    let mut meta = OidcMetadata::default();
    meta.issuer = "https://auth.example.com".to_string();
    meta.jwks_uri = None;

    let payload = json!({
        "iss": "https://auth.example.com",
        "aud": "brum-test-client",
        "sub": "user123",
        "nonce": "valid-session-nonce",
        "exp": chrono::Utc::now().timestamp() - 3600
    });
    let token = build_mock_jwt(&payload);

    let res = manager.verify_id_token_claims(&token, &meta, "valid-session-nonce").await;
    assert!(res.is_err(), "Must reject expired token");
    assert!(res.unwrap_err().to_string().contains("expired"));
}

#[tokio::test]
async fn test_oidc_rejects_issuer_mismatch() {
    let (manager, _tmp) = create_test_oidc_manager();
    let mut meta = OidcMetadata::default();
    meta.issuer = "https://auth.example.com".to_string();
    meta.jwks_uri = None;

    let payload = json!({
        "iss": "https://malicious-idp.com",
        "aud": "brum-test-client",
        "sub": "user123",
        "nonce": "valid-session-nonce",
        "exp": chrono::Utc::now().timestamp() + 3600
    });
    let token = build_mock_jwt(&payload);

    let res = manager.verify_id_token_claims(&token, &meta, "valid-session-nonce").await;
    assert!(res.is_err(), "Must reject token with issuer mismatch");
    assert!(res.unwrap_err().to_string().contains("issuer mismatch"));
}

#[tokio::test]
async fn test_oidc_rejects_unverified_email() {
    let (manager, _tmp) = create_test_oidc_manager();
    let mut meta = OidcMetadata::default();
    meta.issuer = "https://auth.example.com".to_string();
    meta.jwks_uri = None;

    let payload = json!({
        "iss": "https://auth.example.com",
        "aud": "brum-test-client",
        "sub": "user123",
        "nonce": "valid-session-nonce",
        "exp": chrono::Utc::now().timestamp() + 3600,
        "email": "unverified@example.com",
        "email_verified": false
    });
    let token = build_mock_jwt(&payload);

    let res = manager.verify_id_token_claims(&token, &meta, "valid-session-nonce").await;
    assert!(res.is_err(), "Must reject token with email_verified: false");
    assert!(res.unwrap_err().to_string().contains("email address is not verified"));
}

#[tokio::test]
async fn test_oidc_accepts_valid_token() {
    let (manager, _tmp) = create_test_oidc_manager();
    let mut meta = OidcMetadata::default();
    meta.issuer = "https://auth.example.com".to_string();
    meta.jwks_uri = None;

    let payload = json!({
        "iss": "https://auth.example.com",
        "aud": "brum-test-client",
        "sub": "user123",
        "preferred_username": "validuser",
        "email": "user@example.com",
        "email_verified": true,
        "nonce": "valid-session-nonce",
        "exp": chrono::Utc::now().timestamp() + 3600
    });
    let token = build_mock_jwt(&payload);

    let res = manager.verify_id_token_claims(&token, &meta, "valid-session-nonce").await;
    assert!(res.is_ok(), "Valid token must be accepted");
    let claims = res.unwrap();
    assert_eq!(claims.preferred_username.as_deref(), Some("validuser"));
    assert_eq!(claims.sub.as_deref(), Some("user123"));
}

#[tokio::test]
async fn test_oidc_rejects_unsupported_algorithm() {
    let (manager, _tmp) = create_test_oidc_manager();
    let mut meta = OidcMetadata::default();
    meta.issuer = "https://auth.example.com".to_string();
    meta.jwks_uri = Some("https://auth.example.com/jwks.json".to_string());

    // Token with HS256 instead of RS family
    let header = json!({
        "alg": "HS256",
        "typ": "JWT",
        "kid": "key1"
    });
    let header_b64 = URL_SAFE_NO_PAD.encode(header.to_string().as_bytes());
    let payload = json!({
        "iss": "https://auth.example.com",
        "aud": "brum-test-client",
        "sub": "user123",
        "nonce": "test-nonce-123",
        "exp": chrono::Utc::now().timestamp() + 3600
    });
    let payload_b64 = URL_SAFE_NO_PAD.encode(payload.to_string().as_bytes());
    let sig_b64 = URL_SAFE_NO_PAD.encode(b"mock_signature_bytes");
    let token = format!("{}.{}.{}", header_b64, payload_b64, sig_b64);

    let res = manager.verify_id_token_claims(&token, &meta, "test-nonce-123").await;
    assert!(res.is_err(), "Must reject non-RS algorithm");
    let err = res.unwrap_err().to_string();
    assert!(err.contains("Unsupported OIDC token algorithm"), "Unexpected error: {}", err);
}

#[tokio::test]
async fn test_oidc_callback_missing_code_or_state_redirects() {
    use axum::response::IntoResponse;
    let tmp = tempdir().unwrap();
    let db_path = tmp.path().join("test_cb.db").to_string_lossy().to_string();
    let mut config = brum::config::AppConfig::default();
    config.server.database_path = db_path;
    config.auth.oidc.enabled = true;
    let state = brum::create_app_state(&config, None).unwrap();

    // 1. Missing code -> /?error=missing_code
    let query_no_code = brum::server::auth_handlers::OidcCallbackQuery {
        code: None,
        state: Some("test-state".to_string()),
        error: None,
        error_description: None,
    };
    let resp = brum::server::auth_handlers::handle_oidc_callback(
        axum::extract::State(state.clone()),
        None,
        reqwest::header::HeaderMap::new(),
        axum::extract::Query(query_no_code),
    ).await.into_response();
    assert_eq!(resp.status(), axum::http::StatusCode::TEMPORARY_REDIRECT);
    let loc = resp.headers().get(axum::http::header::LOCATION).unwrap().to_str().unwrap();
    assert_eq!(loc, "/?error=missing_code");

    // 2. Missing state -> /?error=missing_state
    let query_no_state = brum::server::auth_handlers::OidcCallbackQuery {
        code: Some("test-code".to_string()),
        state: None,
        error: None,
        error_description: None,
    };
    let resp = brum::server::auth_handlers::handle_oidc_callback(
        axum::extract::State(state.clone()),
        None,
        reqwest::header::HeaderMap::new(),
        axum::extract::Query(query_no_state),
    ).await.into_response();
    assert_eq!(resp.status(), axum::http::StatusCode::TEMPORARY_REDIRECT);
    let loc = resp.headers().get(axum::http::header::LOCATION).unwrap().to_str().unwrap();
    assert_eq!(loc, "/?error=missing_state");

    // 3. Provider error -> /?error=access_denied
    let query_err = brum::server::auth_handlers::OidcCallbackQuery {
        code: None,
        state: None,
        error: Some("access_denied".to_string()),
        error_description: Some("User denied access".to_string()),
    };
    let resp = brum::server::auth_handlers::handle_oidc_callback(
        axum::extract::State(state),
        None,
        reqwest::header::HeaderMap::new(),
        axum::extract::Query(query_err),
    ).await.into_response();
    assert_eq!(resp.status(), axum::http::StatusCode::TEMPORARY_REDIRECT);
    let loc = resp.headers().get(axum::http::header::LOCATION).unwrap().to_str().unwrap();
    assert_eq!(loc, "/?error=access_denied");
}

