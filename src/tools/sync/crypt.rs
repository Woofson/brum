//! Encrypted files in git collections.
//!
//! A collection has one random data key. Files that match the collection's "encrypt" patterns are
//! stored encrypted with it (XChaCha20-Poly1305), so a pushed repository does not reveal them.
//! The data key itself is stored in the repository locked with the passphrase (Argon2id), in
//! `brum-encryption.toml`; the passphrase is never stored there. Changing the passphrase only
//! locks the same data key again, so every saved version stays readable.
//!
//! The nonce of a file comes from its path and contents (keyed), so an unchanged file encrypts to
//! the same bytes and git sees no change. The price: someone with the repository can tell that two
//! versions of the same file are identical, nothing more.

use super::exclude::Exclusions;
use super::store::{DirItem, FileMeta, Listing, Store};
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// The start of every encrypted file in a collection
pub const MAGIC: &[u8] = b"BRUM-ENCRYPTED-1\n";
const NONCE_LEN: usize = 24;
const TAG_LEN: usize = 16;
/// How much larger an encrypted file is than its contents
pub const OVERHEAD: u64 = (MAGIC.len() + NONCE_LEN + TAG_LEN) as u64;
/// Written into the repository: the locked data key and the patterns
pub const KEY_FILE: &str = "brum-encryption.toml";
/// Encrypted files are read into memory whole; secrets are small, this keeps it that way
pub const MAX_ENCRYPTED_SIZE: u64 = 64 * 1024 * 1024;

type HmacSha256 = Hmac<Sha256>;

/// The unlocked data key of a collection
pub struct Cipher {
    enc: [u8; 32],
    mac: [u8; 32],
}

/// Never prints the key
impl std::fmt::Debug for Cipher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Cipher(..)")
    }
}

impl Cipher {
    fn from_key(key: &[u8]) -> Self {
        let mut enc = [0u8; 32];
        let mut mac = [0u8; 32];
        enc.copy_from_slice(&key[..32]);
        mac.copy_from_slice(&key[32..64]);
        Self { enc, mac }
    }

    pub fn encrypt(&self, rel: &str, plain: &[u8]) -> Vec<u8> {
        let mut m = <HmacSha256 as Mac>::new_from_slice(&self.mac).expect("any key length");
        m.update(rel.as_bytes());
        m.update(&[0]);
        m.update(plain);
        let tag = m.finalize().into_bytes();
        let nonce = XNonce::from_slice(&tag[..NONCE_LEN]);
        let sealed = XChaCha20Poly1305::new(self.enc.as_ref().into()).encrypt(nonce, plain).expect("in-memory encryption");
        let mut out = Vec::with_capacity(plain.len() + OVERHEAD as usize);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&tag[..NONCE_LEN]);
        out.extend_from_slice(&sealed);
        out
    }

    pub fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>, String> {
        if !is_encrypted(data) || data.len() < OVERHEAD as usize {
            return Err("Not an encrypted file".into());
        }
        let nonce = XNonce::from_slice(&data[MAGIC.len()..MAGIC.len() + NONCE_LEN]);
        XChaCha20Poly1305::new(self.enc.as_ref().into())
            .decrypt(nonce, &data[MAGIC.len() + NONCE_LEN..])
            .map_err(|_| "This file could not be decrypted: it was changed, or encrypted with another collection's key".to_string())
    }
}

pub fn is_encrypted(head: &[u8]) -> bool {
    head.starts_with(MAGIC)
}

fn kek(passphrase: &str, salt: &[u8]) -> Result<[u8; 32], String> {
    let mut key = [0u8; 32];
    argon2::Argon2::default().hash_password_into(passphrase.as_bytes(), salt, &mut key).map_err(|e| format!("Key derivation failed: {e}"))?;
    Ok(key)
}

fn random<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    getrandom::getrandom(&mut b).expect("system random numbers");
    b
}

/// What `brum-encryption.toml` holds
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct KeyFile {
    /// Argon2id salt for the passphrase (hex)
    pub salt: String,
    /// The data key, locked with the passphrase: nonce and ciphertext (hex)
    pub locked_key: String,
    /// Which files are encrypted, so a new machine knows too
    #[serde(default)]
    pub patterns: Vec<String>,
}

impl KeyFile {
    /// A new data key, locked with `passphrase`
    pub fn create(passphrase: &str, patterns: Vec<String>) -> Result<(Self, Cipher), String> {
        let key: [u8; 64] = random();
        let kf = Self::lock(&key, passphrase, patterns)?;
        Ok((kf, Cipher::from_key(&key)))
    }

    fn lock(key: &[u8], passphrase: &str, patterns: Vec<String>) -> Result<Self, String> {
        let salt: [u8; 16] = random();
        let nonce: [u8; NONCE_LEN] = random();
        let sealed = XChaCha20Poly1305::new(kek(passphrase, &salt)?.as_ref().into())
            .encrypt(XNonce::from_slice(&nonce), key)
            .map_err(|_| "Could not lock the key".to_string())?;
        Ok(Self { salt: hex::encode(salt), locked_key: format!("{}{}", hex::encode(nonce), hex::encode(sealed)), patterns })
    }

    fn data_key(&self, passphrase: &str) -> Result<Vec<u8>, String> {
        let salt = hex::decode(&self.salt).map_err(|_| format!("{KEY_FILE} is damaged"))?;
        let locked = hex::decode(&self.locked_key).map_err(|_| format!("{KEY_FILE} is damaged"))?;
        if locked.len() < NONCE_LEN + 64 {
            return Err(format!("{KEY_FILE} is damaged"));
        }
        let key = XChaCha20Poly1305::new(kek(passphrase, &salt)?.as_ref().into())
            .decrypt(XNonce::from_slice(&locked[..NONCE_LEN]), &locked[NONCE_LEN..])
            .map_err(|_| "Wrong passphrase for this collection's encrypted files".to_string())?;
        if key.len() != 64 {
            return Err(format!("{KEY_FILE} is damaged"));
        }
        Ok(key)
    }

    pub fn unlock(&self, passphrase: &str) -> Result<Cipher, String> {
        Ok(Cipher::from_key(&self.data_key(passphrase)?))
    }

    /// The same data key, locked with a new passphrase
    pub fn relock(&self, old: &str, new: &str) -> Result<Self, String> {
        Self::lock(&self.data_key(old)?, new, self.patterns.clone())
    }

    pub fn read(repo: &Path) -> Result<Option<Self>, String> {
        match std::fs::read_to_string(repo.join(KEY_FILE)) {
            Ok(text) => toml::from_str(&text).map(Some).map_err(|e| format!("{KEY_FILE} could not be read: {e}")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    pub fn write(&self, repo: &Path) -> Result<(), String> {
        let body = toml::to_string(self).map_err(|e| e.to_string())?;
        let text = format!("# Made by Brum Backup & Sync. The key for this collection's encrypted files,\n# locked with a passphrase that is not stored here.\n{body}");
        std::fs::write(repo.join(KEY_FILE), text).map_err(|e| e.to_string())
    }
}

/// Which files to encrypt: patterns like `.ssh/id_*`, `.env` or `home/.config/app/token`.
/// A name matches anywhere; a path matches any run of folders, wherever it starts.
pub fn wants(patterns: &Exclusions, rel: &str) -> bool {
    patterns.matches_anywhere(rel)
}

/// A collection repository (or one saved version of it) seen with its files decrypted: reading
/// decrypts encrypted files, writing encrypts the files that match the patterns.
pub struct CryptStore {
    inner: Arc<dyn Store>,
    cipher: Option<Arc<Cipher>>,
    patterns: Exclusions,
    /// Look inside each file to see whether it is encrypted (the working folder). Otherwise the
    /// patterns decide (saved versions, where reading every file would be slow).
    probe: bool,
}

impl CryptStore {
    pub fn new(inner: Arc<dyn Store>, cipher: Option<Arc<Cipher>>, patterns: Exclusions, probe: bool) -> Self {
        Self { inner, cipher, patterns, probe }
    }

    fn encrypted_on_disk(&self, rel: &str) -> bool {
        let Ok(mut r) = self.inner.open_read(rel) else { return false };
        let mut head = [0u8; MAGIC.len()];
        r.read_exact(&mut head).is_ok() && is_encrypted(&head)
    }

    fn encrypted(&self, rel: &str) -> bool {
        if self.probe { self.encrypted_on_disk(rel) } else { wants(&self.patterns, rel) }
    }

    /// The size of the contents. In the working folder a file stored the wrong way (encrypted but
    /// no longer wanted, or the other way round) shows an impossible time, so it is written again.
    fn meta(&self, rel: &str, m: FileMeta) -> FileMeta {
        let enc = self.encrypted(rel);
        let size = if enc { m.size.saturating_sub(OVERHEAD) } else { m.size };
        let mtime = if self.probe && enc != wants(&self.patterns, rel) { 0 } else { m.mtime };
        FileMeta { size, mtime }
    }

    fn cipher(&self) -> Result<&Cipher, String> {
        self.cipher.as_deref().ok_or_else(|| "This file is encrypted; the collection's passphrase is needed".to_string())
    }
}

impl Store for CryptStore {
    fn label(&self) -> String {
        self.inner.label()
    }
    fn root_exists(&self) -> bool {
        self.inner.root_exists()
    }
    fn ensure_root(&self) -> Result<(), String> {
        self.inner.ensure_root()
    }
    fn list(&self, base: &str, skip: &dyn Fn(&str, bool) -> bool, cancel: &AtomicBool, warnings: &mut Vec<String>) -> Result<Listing, String> {
        let listing = self.inner.list(base, skip, cancel, warnings)?;
        Ok(listing.into_iter().map(|(rel, m)| {
            let full = super::store::join_rel(base, &rel);
            let m = self.meta(&full, m);
            (rel, m)
        }).collect())
    }
    fn list_dir(&self, rel: &str) -> Result<Vec<DirItem>, String> {
        let mut items = self.inner.list_dir(rel)?;
        for it in items.iter_mut().filter(|i| !i.is_dir) {
            if self.encrypted(&super::store::join_rel(rel, &it.name)) {
                it.size = it.size.saturating_sub(OVERHEAD);
            }
        }
        Ok(items)
    }
    fn stat(&self, rel: &str) -> Option<FileMeta> {
        self.inner.stat(rel).map(|m| self.meta(rel, m))
    }
    fn is_dir(&self, rel: &str) -> bool {
        self.inner.is_dir(rel)
    }
    fn open_read(&self, rel: &str) -> Result<Box<dyn Read + Send>, String> {
        let mut r = self.inner.open_read(rel)?;
        let mut head = Vec::with_capacity(MAGIC.len());
        (&mut r).take(MAGIC.len() as u64).read_to_end(&mut head).map_err(|e| e.to_string())?;
        if !is_encrypted(&head) {
            return Ok(Box::new(Cursor::new(head).chain(r)));
        }
        let mut data = head;
        r.take(MAX_ENCRYPTED_SIZE + OVERHEAD).read_to_end(&mut data).map_err(|e| e.to_string())?;
        Ok(Box::new(Cursor::new(self.cipher()?.decrypt(&data)?)))
    }
    fn write_part(&self, rel: &str, data: &mut dyn Read, mtime: i64, on_bytes: &mut dyn FnMut(u64) -> bool) -> Result<String, String> {
        if !wants(&self.patterns, rel) {
            return self.inner.write_part(rel, data, mtime, on_bytes);
        }
        let cipher = self.cipher().map_err(|_| "This file is to be encrypted, but the collection has no passphrase".to_string())?;
        let mut plain = Vec::new();
        data.take(MAX_ENCRYPTED_SIZE + 1).read_to_end(&mut plain).map_err(|e| e.to_string())?;
        if plain.len() as u64 > MAX_ENCRYPTED_SIZE {
            return Err(format!("Too large to encrypt (over {} MB); encrypted files are meant for keys and settings", MAX_ENCRYPTED_SIZE / 1024 / 1024));
        }
        if !on_bytes(plain.len() as u64) {
            return Err("Cancelled".into());
        }
        let sealed = cipher.encrypt(rel, &plain);
        self.inner.write_part(rel, &mut Cursor::new(sealed), mtime, &mut |_| true)
    }
    fn rename(&self, from: &str, to: &str) -> Result<(), String> {
        self.inner.rename(from, to)
    }
    fn remove_file(&self, rel: &str) -> Result<(), String> {
        self.inner.remove_file(rel)
    }
    fn remove_dir_all(&self, rel: &str) -> Result<(), String> {
        self.inner.remove_dir_all(rel)
    }
    fn remove_empty_parents(&self, rel: &str) {
        self.inner.remove_empty_parents(rel)
    }
    /// No shortcuts that would bypass encryption (partial updates work on the raw files)
    fn local_path(&self, rel: &str) -> Option<PathBuf> {
        if wants(&self.patterns, rel) || self.encrypted_on_disk(rel) {
            return None;
        }
        self.inner.local_path(rel)
    }
    fn part_names(&self) -> Vec<String> {
        self.inner.part_names()
    }
    fn is_encrypted(&self, rel: &str) -> bool {
        self.encrypted(rel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_file_same_bytes_and_round_trip() {
        let (kf, c) = KeyFile::create("correct horse", vec![".ssh/id_*".into()]).unwrap();
        let a = c.encrypt("home/.ssh/id_ed25519", b"PRIVATE KEY");
        assert_eq!(a, c.encrypt("home/.ssh/id_ed25519", b"PRIVATE KEY"), "unchanged file, unchanged bytes");
        assert_ne!(a, c.encrypt("home/.ssh/id_rsa", b"PRIVATE KEY"), "another path, other bytes");
        assert!(!a.windows(11).any(|w| w == b"PRIVATE KEY"));
        assert_eq!(a.len() as u64, 11 + OVERHEAD);
        assert_eq!(c.decrypt(&a).unwrap(), b"PRIVATE KEY");
        // Tampering is noticed
        let mut bad = a.clone();
        *bad.last_mut().unwrap() ^= 1;
        assert!(c.decrypt(&bad).is_err());
        // The passphrase unlocks the same key; a wrong one does not
        assert_eq!(kf.unlock("correct horse").unwrap().decrypt(&a).unwrap(), b"PRIVATE KEY");
        assert!(kf.unlock("wrong").unwrap_err().contains("Wrong passphrase"));
        // A new passphrase keeps the key: old files still open
        let kf2 = kf.relock("correct horse", "battery staple").unwrap();
        assert_ne!(kf2.salt, kf.salt);
        assert_eq!(kf2.unlock("battery staple").unwrap().decrypt(&a).unwrap(), b"PRIVATE KEY");
        assert!(kf2.unlock("correct horse").is_err());
        assert!(kf.relock("wrong", "x").is_err());
    }

    #[test]
    fn patterns_match_anywhere() {
        let p = Exclusions::parse(".ssh/id_*\n.env\nhome/.config/app/token");
        assert!(wants(&p, "home/.ssh/id_ed25519"));
        assert!(wants(&p, "home/proj/.env"));
        assert!(wants(&p, "home/.config/app/token"));
        assert!(!wants(&p, "home/.ssh/config"));
        assert!(!wants(&p, "home/.envrc"));
        // A folder pattern covers everything inside
        let p = Exclusions::parse(".gnupg");
        assert!(wants(&p, "home/.gnupg/private-keys-v1.d/abc.key"));
    }
}
