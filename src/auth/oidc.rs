use std::collections::HashMap;
use std::sync::Arc;
use base64::Engine;
use chrono::Utc;
use parking_lot::Mutex;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;
use tracing::info;

use crate::auth::{AuthManager, User};
use crate::config::OidcConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OidcMetadata {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    #[serde(default)]
    pub userinfo_endpoint: Option<String>,
    #[serde(default)]
    pub jwks_uri: Option<String>,
    #[serde(default)]
    pub end_session_endpoint: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PendingOidcState {
    pub state: String,
    pub nonce: String,
    pub code_verifier: String,
    pub redirect_uri: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OidcPublicConfig {
    pub enabled: bool,
    pub provider_name: String,
    pub button_icon: String,
    pub force_sso_only: bool,
    pub login_url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OidcTokenResponse {
    pub access_token: String,
    pub token_type: Option<String>,
    pub id_token: Option<String>,
    pub expires_in: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwkKey {
    pub kty: String,
    #[serde(default)]
    pub alg: Option<String>,
    #[serde(default)]
    pub kid: Option<String>,
    #[serde(default)]
    pub n: Option<String>,
    #[serde(default)]
    pub e: Option<String>,
    #[serde(rename = "use", default)]
    pub key_use: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct JwksResponse {
    #[serde(default)]
    pub keys: Vec<JwkKey>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OidcClaims {
    pub iss: Option<String>,
    pub aud: Option<serde_json::Value>,
    pub exp: Option<i64>,
    pub sub: Option<String>,
    pub nonce: Option<String>,
    pub preferred_username: Option<String>,
    pub username: Option<String>,
    pub nickname: Option<String>,
    pub name: Option<String>,
    pub given_name: Option<String>,
    pub email: Option<String>,
    pub email_verified: Option<bool>,
    pub picture: Option<String>,
    pub avatar_url: Option<String>,
    #[serde(default, deserialize_with = "deserialize_string_or_vec")]
    pub groups: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_string_or_vec")]
    pub roles: Vec<String>,
    pub is_superuser: Option<bool>,
}

fn deserialize_string_or_vec<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct StringOrVec;

    impl<'de> serde::de::Visitor<'de> for StringOrVec {
        type Value = Vec<String>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("string or list of strings")
        }

        fn visit_str<E>(self, value: &str) -> Result<Vec<String>, E>
        where
            E: serde::de::Error,
        {
            Ok(value.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
        }

        fn visit_seq<S>(self, mut seq: S) -> Result<Vec<String>, S::Error>
        where
            S: serde::de::SeqAccess<'de>,
        {
            let mut vec = Vec::new();
            while let Some(elem) = seq.next_element::<serde_json::Value>()? {
                if let Some(s) = elem.as_str() {
                    vec.push(s.to_string());
                } else if let Some(n) = elem.get("name").and_then(|v| v.as_str()) {
                    vec.push(n.to_string());
                }
            }
            Ok(vec)
        }
    }

    deserializer.deserialize_any(StringOrVec)
}

pub struct OidcManager {
    config: OidcConfig,
    auth: Arc<AuthManager>,
    http_client: reqwest::Client,
    metadata_cache: Arc<RwLock<Option<OidcMetadata>>>,
    jwks_cache: Arc<RwLock<Option<(i64, JwksResponse)>>>,
    pending_states: Arc<Mutex<HashMap<String, PendingOidcState>>>,
}

impl OidcManager {
    pub fn new(config: OidcConfig, auth: Arc<AuthManager>) -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(12))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self {
            config,
            auth,
            http_client,
            metadata_cache: Arc::new(RwLock::new(None)),
            jwks_cache: Arc::new(RwLock::new(None)),
            pending_states: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    pub fn get_public_config(&self) -> OidcPublicConfig {
        OidcPublicConfig {
            enabled: self.config.enabled,
            provider_name: if self.config.provider_name.trim().is_empty() {
                "Authentik".to_string()
            } else {
                self.config.provider_name.clone()
            },
            button_icon: self.config.button_icon.clone(),
            force_sso_only: self.config.force_sso_only,
            login_url: "/api/auth/oidc/login".to_string(),
        }
    }

    pub async fn discover_metadata(&self) -> Result<OidcMetadata, Box<dyn std::error::Error + Send + Sync>> {
        {
            let cache = self.metadata_cache.read().await;
            if let Some(meta) = &*cache {
                return Ok(meta.clone());
            }
        }

        let issuer = self.config.issuer_url.trim().trim_end_matches('/');
        if issuer.is_empty() {
            return Err("OIDC issuer_url is not configured in brum.toml".into());
        }

        let discovery_url = format!("{}/.well-known/openid-configuration", issuer);
        info!("Fetching OIDC discovery metadata from: {}", discovery_url);

        let resp = self.http_client.get(&discovery_url).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Failed to fetch OIDC discovery from {}: {} - {}", discovery_url, status, body).into());
        }

        let metadata: OidcMetadata = resp.json().await?;
        {
            let mut cache = self.metadata_cache.write().await;
            *cache = Some(metadata.clone());
        }

        Ok(metadata)
    }

    pub async fn generate_auth_url(&self, dynamic_redirect_uri: &str) -> Result<(String, String), Box<dyn std::error::Error + Send + Sync>> {
        let meta = self.discover_metadata().await?;

        let redirect_uri = if !self.config.redirect_url.trim().is_empty() {
            self.config.redirect_url.trim().to_string()
        } else {
            dynamic_redirect_uri.to_string()
        };

        let state = generate_random_string(32);
        let nonce = generate_random_string(32);
        let code_verifier = generate_random_string(48);
        let code_challenge = generate_code_challenge(&code_verifier);

        // Store pending state with 15m expiration
        {
            let mut pending = self.pending_states.lock();
            let now = Utc::now().timestamp();
            pending.retain(|_, v| now - v.created_at < 900);

            pending.insert(
                state.clone(),
                PendingOidcState {
                    state: state.clone(),
                    nonce: nonce.clone(),
                    code_verifier,
                    redirect_uri: redirect_uri.clone(),
                    created_at: now,
                },
            );
        }

        let mut url = reqwest::Url::parse(&meta.authorization_endpoint)?;
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("response_type", "code");
            q.append_pair("client_id", &self.config.client_id);
            q.append_pair("redirect_uri", &redirect_uri);
            q.append_pair("scope", &self.config.scopes.join(" "));
            q.append_pair("state", &state);
            q.append_pair("nonce", &nonce);
            q.append_pair("code_challenge", &code_challenge);
            q.append_pair("code_challenge_method", "S256");
        }

        Ok((url.to_string(), state))
    }

    pub async fn fetch_jwks(&self, jwks_uri: &str, force_refresh: bool) -> Result<JwksResponse, Box<dyn std::error::Error + Send + Sync>> {
        let now = Utc::now().timestamp();
        if !force_refresh {
            let cache = self.jwks_cache.read().await;
            if let Some((ts, jwks)) = &*cache {
                if now - ts < 3600 {
                    return Ok(jwks.clone());
                }
            }
        }

        info!("Fetching OIDC JWKS from: {}", jwks_uri);
        let resp = self.http_client.get(jwks_uri).send().await?;
        if !resp.status().is_success() {
            return Err(format!("Failed to fetch JWKS from {}: HTTP {}", jwks_uri, resp.status()).into());
        }

        let jwks: JwksResponse = resp.json().await?;
        {
            let mut cache = self.jwks_cache.write().await;
            *cache = Some((now, jwks.clone()));
        }

        Ok(jwks)
    }

    pub async fn exchange_code_and_login(&self, code: &str, state: &str) -> Result<(String, User), Box<dyn std::error::Error + Send + Sync>> {
        let pending = {
            let mut map = self.pending_states.lock();
            map.remove(state).ok_or_else(|| "Invalid or expired OIDC state parameter (CSRF protection)".to_string())?
        };

        let meta = self.discover_metadata().await?;

        // 1. Exchange authorization code at token_endpoint
        let mut form_params: Vec<(&str, &str)> = vec![
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", &pending.redirect_uri),
            ("client_id", &self.config.client_id),
            ("code_verifier", &pending.code_verifier),
        ];
        if !self.config.client_secret.is_empty() {
            form_params.push(("client_secret", &self.config.client_secret));
        }

        let token_resp = self.http_client
            .post(&meta.token_endpoint)
            .form(&form_params)
            .send()
            .await?;

        if !token_resp.status().is_success() {
            let status = token_resp.status();
            let body = token_resp.text().await.unwrap_or_default();
            return Err(format!("Token exchange failed ({}): {}", status, body).into());
        }

        let token_data: OidcTokenResponse = token_resp.json().await?;

        // 2. Decode & Validate claims from ID Token and/or userinfo endpoint
        let mut claims = OidcClaims::default();

        if let Some(id_token) = &token_data.id_token {
            let mut validated = false;
            if let Some(jwks_uri) = &meta.jwks_uri {
                if let Ok(jwt_header) = jsonwebtoken::decode_header(id_token) {
                    let jwks = self.fetch_jwks(jwks_uri, false).await.ok();
                    let find_key = |jwks: &JwksResponse, kid: Option<&str>| -> Option<JwkKey> {
                        if let Some(target_kid) = kid {
                            jwks.keys.iter().find(|k| k.kid.as_deref() == Some(target_kid)).cloned()
                        } else {
                            jwks.keys.iter().find(|k| k.kty.eq_ignore_ascii_case("RSA")).cloned()
                        }
                    };

                    let mut matched_key = jwks.as_ref().and_then(|j| find_key(j, jwt_header.kid.as_deref()));
                    if matched_key.is_none() {
                        // Refresh JWKS once in case keys rotated
                        if let Ok(refreshed) = self.fetch_jwks(jwks_uri, true).await {
                            matched_key = find_key(&refreshed, jwt_header.kid.as_deref());
                        }
                    }

                    if let Some(key) = matched_key {
                        if let (Some(n), Some(e)) = (&key.n, &key.e) {
                            if let Ok(decoding_key) = jsonwebtoken::DecodingKey::from_rsa_components(n, e) {
                                let mut validation = jsonwebtoken::Validation::new(jwt_header.alg);
                                validation.set_issuer(&[&meta.issuer, meta.issuer.trim_end_matches('/')]);
                                validation.set_audience(&[&self.config.client_id]);

                                match jsonwebtoken::decode::<OidcClaims>(id_token, &decoding_key, &validation) {
                                    Ok(token_data) => {
                                        claims = token_data.claims;
                                        validated = true;
                                        info!("Successfully verified OIDC ID token cryptographic signature.");
                                    }
                                    Err(err) => {
                                        return Err(format!("OIDC ID token signature verification failed: {}", err).into());
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if !validated {
                let parts: Vec<&str> = id_token.split('.').collect();
                if parts.len() >= 2 {
                    if let Ok(decoded_bytes) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(parts[1])
                        .or_else(|_| base64::engine::general_purpose::STANDARD.decode(parts[1])) {
                        if let Ok(parsed_claims) = serde_json::from_slice::<OidcClaims>(&decoded_bytes) {
                            let now = Utc::now().timestamp();
                            if let Some(exp) = parsed_claims.exp {
                                if exp < now - 60 {
                                    return Err("OIDC ID token is expired".into());
                                }
                            }
                            if let Some(iss) = &parsed_claims.iss {
                                if iss.trim_end_matches('/') != meta.issuer.trim_end_matches('/') {
                                    return Err(format!("OIDC ID token issuer mismatch: expected '{}', got '{}'", meta.issuer, iss).into());
                                }
                            }
                            claims = parsed_claims;
                        }
                    }
                }
            }

            // Nonce verification to prevent replay / CSRF attacks
            if let Some(token_nonce) = &claims.nonce {
                if token_nonce != &pending.nonce {
                    return Err("OIDC ID token nonce mismatch (possible replay or CSRF attack)".into());
                }
            }
        }

        if let Some(userinfo_url) = &meta.userinfo_endpoint {
            if let Ok(u_resp) = self.http_client.get(userinfo_url)
                .bearer_auth(&token_data.access_token)
                .send()
                .await {
                if u_resp.status().is_success() {
                    if let Ok(u_claims) = u_resp.json::<OidcClaims>().await {
                        if u_claims.sub.is_some() { claims.sub = u_claims.sub; }
                        if u_claims.preferred_username.is_some() { claims.preferred_username = u_claims.preferred_username; }
                        if u_claims.username.is_some() { claims.username = u_claims.username; }
                        if u_claims.email.is_some() { claims.email = u_claims.email; }
                        if u_claims.name.is_some() { claims.name = u_claims.name; }
                        if u_claims.nickname.is_some() { claims.nickname = u_claims.nickname; }
                        if u_claims.avatar_url.is_some() { claims.avatar_url = u_claims.avatar_url; }
                        if u_claims.picture.is_some() { claims.picture = u_claims.picture; }
                        if !u_claims.groups.is_empty() { claims.groups.extend(u_claims.groups); }
                        if !u_claims.roles.is_empty() { claims.roles.extend(u_claims.roles); }
                        if u_claims.is_superuser.is_some() { claims.is_superuser = u_claims.is_superuser; }
                    }
                }
            }
        }

        // 3. Resolve username
        let raw_username = claims.preferred_username
            .or(claims.username)
            .or_else(|| claims.email.as_ref().map(|e| e.split('@').next().unwrap_or("sso_user").to_string()))
            .or(claims.sub.clone())
            .unwrap_or_else(|| "sso_user".to_string());

        let sanitized_username: String = raw_username
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_' || *c == '.')
            .collect();

        let username = if sanitized_username.is_empty() {
            let prefix = if pending.nonce.len() >= 8 { &pending.nonce[..8] } else { "user" };
            format!("sso_{}", prefix)
        } else {
            sanitized_username
        };

        // 4. Resolve Admin Group & Role Mapping
        let admin_group = self.config.admin_group.trim();
        let is_admin = claims.is_superuser.unwrap_or(false)
            || (!admin_group.is_empty() && claims.groups.iter().any(|g| g.eq_ignore_ascii_case(admin_group)))
            || claims.roles.iter().any(|r| r.eq_ignore_ascii_case("admin") || r.eq_ignore_ascii_case("superuser"))
            || (admin_group.is_empty() && self.config.default_user_role == "admin");

        let target_role = if is_admin {
            "admin"
        } else {
            &self.config.default_user_role
        };

        let nickname = claims.nickname.or(claims.name);
        let email = claims.email;
        let avatar_url = claims.avatar_url.or(claims.picture);

        // 5. Match or Auto-Provision User in Brum SQLite Database
        let existing_user = self.auth.get_user_by_username(&username)?;

        let user = match existing_user {
            Some(mut u) => {
                // Prevent Account Takeover: Native local / PAM / database users cannot be taken over by SSO
                if u.auth_source.as_deref() != Some("oidc") {
                    tracing::warn!(
                        username = %username,
                        auth_source = ?u.auth_source,
                        "SSO login rejected: external OIDC user tried to claim existing native local account"
                    );
                    return Err(format!(
                        "SSO identity cannot take over existing local account '{}'. Account takeover prevented.",
                        username
                    ).into());
                }

                info!(username = %username, role = %target_role, "User successfully authenticated via OIDC/SSO");
                if u.role != target_role {
                    let _ = self.auth.update_user_rbac(
                        &username,
                        target_role,
                        &u.allowed_services,
                        Some(&u.allowed_roots),
                        Some(&u.home_dir),
                        Some(u.can_install_plugins),
                        Some(&u.allowed_plugins),
                        Some(&u.blocked_plugins),
                        u.is_disabled,
                    );
                    u.role = target_role.to_string();
                }
                if (u.nickname.is_none() && nickname.is_some())
                    || (u.email.is_none() && email.is_some())
                    || (u.avatar_url.is_none() && avatar_url.is_some())
                {
                    let _ = self.auth.update_user_profile(
                        &username,
                        nickname.as_deref().or(u.nickname.as_deref()),
                        None,
                        None,
                        email.as_deref().or(u.email.as_deref()),
                        avatar_url.as_deref().or(u.avatar_url.as_deref()),
                    );
                }
                u
            }
            None => {
                if !self.config.auto_provision {
                    return Err(format!("User '{}' authenticated via SSO, but automatic provisioning is disabled", username).into());
                }

                info!(username = %username, role = %target_role, "Auto-provisioning new user profile from OIDC/SSO");

                let home_dir = self.config.default_home_template.replace("{username}", &username);
                let _ = std::fs::create_dir_all(&home_dir);

                let mut created = self.auth.create_oidc_user(
                    &username,
                    target_role,
                    &home_dir,
                    Some("[\"*\"]"),
                )?;

                if nickname.is_some() || email.is_some() || avatar_url.is_some() {
                    let _ = self.auth.update_user_profile(
                        &username,
                        nickname.as_deref(),
                        None,
                        None,
                        email.as_deref(),
                        avatar_url.as_deref(),
                    );
                    created.nickname = nickname;
                    created.email = email;
                    created.avatar_url = avatar_url;
                }

                created
            }
        };

        if user.is_disabled {
            return Err(format!("Account '{}' is disabled", username).into());
        }

        // 6. Generate Brum Session JWT Token
        let token = self.auth.generate_token(&user)?;

        Ok((token, user))
    }
}

fn generate_random_string(len: usize) -> String {
    let mut bytes = vec![0u8; len];
    rand::thread_rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&bytes)
}

fn generate_code_challenge(verifier: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let hash = hasher.finalize();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&hash)
}
