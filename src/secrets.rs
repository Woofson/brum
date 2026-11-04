//! Encryption for passwords and tokens Brum has to store and use again (remote logins, backup
//! destinations, Fleet tokens). They are kept encrypted in the database with a key that lives
//! outside it, so a copy of the database alone does not reveal them.
//!
//! The key comes from `BRUM_SECRETS_KEY` (64 hex characters) when set, otherwise from a key file
//! next to the database (`brum-secrets.key`, readable only by Brum's user), created on first start.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tracing::{info, warn};

const PREFIX: &str = "enc:v1:";
static KEY: OnceLock<Option<[u8; 32]>> = OnceLock::new();

/// Where the key file for a database lives
pub fn key_file_for(db_path: &str) -> Option<PathBuf> {
    if db_path.is_empty() || db_path == ":memory:" {
        return None;
    }
    let parent = Path::new(db_path).parent().map(|p| p.to_path_buf()).unwrap_or_default();
    Some(if parent.as_os_str().is_empty() { PathBuf::from("brum-secrets.key") } else { parent.join("brum-secrets.key") })
}

fn parse_hex_key(s: &str) -> Option<[u8; 32]> {
    let bytes = hex::decode(s.trim()).ok()?;
    bytes.try_into().ok()
}

fn load_or_create(db_path: &str) -> Option<[u8; 32]> {
    if let Ok(env) = std::env::var("BRUM_SECRETS_KEY") {
        match parse_hex_key(&env) {
            Some(k) => return Some(k),
            None => warn!("BRUM_SECRETS_KEY is set but is not 64 hex characters; using the key file instead"),
        }
    }
    let Some(path) = key_file_for(db_path) else {
        // In-memory databases (tests): a key for this run only
        use rand::RngCore;
        let mut k = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut k);
        return Some(k);
    };
    if let Ok(text) = std::fs::read_to_string(&path) {
        if let Some(k) = parse_hex_key(&text) {
            return Some(k);
        }
        warn!("{} is not a valid key; stored passwords stay readable until it is fixed", path.display());
        return None;
    }
    use rand::RngCore;
    let mut k = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut k);
    if let Err(e) = write_private(&path, &hex::encode(k)) {
        warn!("Could not create {}: {e}. Stored passwords stay readable.", path.display());
        return None;
    }
    info!("Created {} to encrypt stored passwords. Keep it with the database, but out of shared backups of the database alone.", path.display());
    Some(k)
}

fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)?;
        f.write_all(text.as_bytes())?;
        f.write_all(b"\n")
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, format!("{text}\n"))
    }
}

/// Loads (or creates) the key for this database. Called once at start-up; later calls do nothing.
pub fn init(db_path: &str) {
    KEY.get_or_init(|| load_or_create(db_path));
}

fn key() -> Option<&'static [u8; 32]> {
    // Code that runs before init (some tests) gets a key for this run only
    KEY.get_or_init(|| load_or_create(":memory:")).as_ref()
}

/// A secret for one purpose (for example signing logins), derived from the key, so it never
/// has to be stored anywhere. None when there is no key.
pub fn derived_secret(purpose: &str) -> Option<String> {
    use sha2::{Digest, Sha256};
    let k = key()?;
    let mut h = Sha256::new();
    h.update(b"brum-derived-secret:");
    h.update(purpose.as_bytes());
    h.update(k);
    Some(hex::encode(h.finalize()))
}

pub fn is_sealed(s: &str) -> bool {
    s.starts_with(PREFIX)
}

/// Encrypts a value for storing. Empty values and values already encrypted are left as they are.
pub fn seal(plain: &str) -> String {
    if plain.is_empty() || is_sealed(plain) {
        return plain.to_string();
    }
    let Some(k) = key() else { return plain.to_string() };
    let cipher = Aes256Gcm::new(k.into());
    let mut nonce = [0u8; 12];
    {
        use rand::RngCore;
        rand::rngs::OsRng.fill_bytes(&mut nonce);
    }
    match cipher.encrypt(Nonce::from_slice(&nonce), plain.as_bytes()) {
        Ok(ct) => {
            let mut all = nonce.to_vec();
            all.extend(ct);
            format!("{PREFIX}{}", base64::engine::general_purpose::STANDARD.encode(all))
        }
        Err(_) => plain.to_string(),
    }
}

/// Decrypts a stored value. Values stored before encryption existed come back unchanged.
pub fn open(stored: &str) -> String {
    let Some(b64) = stored.strip_prefix(PREFIX) else { return stored.to_string() };
    let Some(k) = key() else {
        warn!("A stored password cannot be read: the encryption key is missing");
        return String::new();
    };
    let Ok(all) = base64::engine::general_purpose::STANDARD.decode(b64) else { return String::new() };
    if all.len() < 13 {
        return String::new();
    }
    let (nonce, ct) = all.split_at(12);
    Aes256Gcm::new(k.into())
        .decrypt(Nonce::from_slice(nonce), ct)
        .ok()
        .and_then(|p| String::from_utf8(p).ok())
        .unwrap_or_else(|| {
            warn!("A stored password cannot be read: it was encrypted with a different key");
            String::new()
        })
}

/// True when an address carries a password: `scheme://user:password@host/...`
pub fn has_embedded_password(uri: &str) -> bool {
    let Some((_, rest)) = uri.split_once("://") else { return false };
    let authority = rest.split('/').next().unwrap_or("");
    authority.rsplit_once('@').is_some_and(|(userinfo, _)| userinfo.contains(':'))
}

/// Encrypts an address only when it carries a password, so plain addresses stay readable
pub fn seal_uri(uri: &str) -> String {
    if has_embedded_password(uri) { seal(uri) } else { uri.to_string() }
}

pub fn seal_opt(v: Option<&str>) -> Option<String> {
    v.map(seal)
}

pub fn open_opt(v: Option<String>) -> Option<String> {
    v.map(|s| open(&s))
}

/// Encrypts the `auth_token` of each saved Fleet machine in a user's preferences (JSON text)
pub fn seal_preferences(json: &str) -> String {
    map_fleet_tokens(json, seal)
}

/// Decrypts the Fleet tokens in a user's preferences
pub fn open_preferences(json: &str) -> String {
    map_fleet_tokens(json, open)
}

fn map_fleet_tokens(json: &str, f: fn(&str) -> String) -> String {
    let Ok(mut v) = serde_json::from_str::<serde_json::Value>(json) else { return json.to_string() };
    let mut changed = false;
    if let Some(nodes) = v.get_mut("fleet_nodes").and_then(|n| n.as_array_mut()) {
        for node in nodes {
            if let Some(tok) = node.get("auth_token").and_then(|t| t.as_str()).map(String::from) {
                let mapped = f(&tok);
                if mapped != tok {
                    node["auth_token"] = serde_json::Value::String(mapped);
                    changed = true;
                }
            }
        }
    }
    if changed { v.to_string() } else { json.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_old_values() {
        let s = seal("s3cr@t");
        assert!(is_sealed(&s) && !s.contains("s3cr@t"));
        assert_ne!(s, seal("s3cr@t"), "a fresh nonce each time");
        assert_eq!(open(&s), "s3cr@t");
        assert_eq!(open("plain-from-before"), "plain-from-before");
        assert_eq!(seal(""), "");
        assert_eq!(seal(&s), s, "never encrypted twice");
        assert_eq!(open("enc:v1:garbage!!"), "");
    }

    #[test]
    fn addresses_with_passwords() {
        assert!(has_embedded_password("sftp://bolt:pw@nas:22/x"));
        assert!(!has_embedded_password("sftp://bolt@nas/x"));
        assert!(!has_embedded_password("/home/bolt/a:b@c"));
        assert_eq!(seal_uri("smb://nas/share"), "smb://nas/share");
        assert!(is_sealed(&seal_uri("smb://u:p@nas/share")));
    }

    #[test]
    fn fleet_tokens_in_preferences() {
        let prefs = r#"{"theme":"zink","fleet_nodes":[{"id":"nas","auth_token":"tok123"},{"id":"pc","auth_token":""}]}"#;
        let sealed = seal_preferences(prefs);
        assert!(!sealed.contains("tok123") && sealed.contains("zink"));
        let opened: serde_json::Value = serde_json::from_str(&open_preferences(&sealed)).unwrap();
        assert_eq!(opened["fleet_nodes"][0]["auth_token"], "tok123");
        assert_eq!(opened["fleet_nodes"][1]["auth_token"], "");
        assert_eq!(seal_preferences("not json"), "not json");
    }

    #[test]
    fn key_file_beside_the_database() {
        assert_eq!(key_file_for("/var/lib/brum/brum.db").unwrap(), PathBuf::from("/var/lib/brum/brum-secrets.key"));
        assert!(key_file_for(":memory:").is_none());
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("k");
        write_private(&p, "abc").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }
}
