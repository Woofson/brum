use crate::vfs::{DirectoryListing, FileContentResponse, FileEntry};
use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Key, Nonce,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tracing::info;

pub const CDVAULT_MAGIC: &[u8; 8] = b"CDVAULT1";
pub const KEY_LEN: usize = 32; // 256-bit AES
pub const NONCE_LEN: usize = 12; // 96-bit standard GCM nonce
pub const TAG_LEN: usize = 16; // 128-bit GCM auth tag
pub const SALT_LEN: usize = 16;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultHeader {
    pub magic: String,          // "CDVAULT1"
    pub version: u32,           // 1
    pub salt_hex: String,       // 16 bytes salt for Argon2id
    pub master_nonce_hex: String,// 12 bytes IV
    pub master_tag_hex: String,  // 16 bytes GCM tag
    pub master_key_enc_hex: String, // Encrypted 32-byte master key
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultManifestItem {
    pub id: String,             // Random UUID / file storage ID
    pub name: String,           // Decrypted virtual file name (e.g. "passwords.txt")
    pub path: String,           // Decrypted relative virtual path (e.g. "finance/2026.xlsx")
    pub is_dir: bool,
    pub size: u64,              // Unencrypted size in bytes
    pub modified: u64,          // Unix timestamp
    pub mime_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultManifest {
    pub items: Vec<VaultManifestItem>,
}

pub struct UnlockedVaultSession {
    pub vault_path: PathBuf,
    pub master_key: [u8; KEY_LEN],
    pub manifest: VaultManifest,
    pub last_accessed: Instant,
    pub auto_lock_duration: Duration,
}

pub struct VaultManager {
    sessions: Arc<RwLock<HashMap<String, UnlockedVaultSession>>>,
}

impl VaultManager {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Derive 32-byte key from password and salt using Argon2id
    pub fn derive_key(password: &str, salt: &[u8]) -> Result<[u8; KEY_LEN], String> {
        let mut key = [0u8; KEY_LEN];
        let argon2 = argon2::Argon2::default();
        argon2
            .hash_password_into(password.as_bytes(), salt, &mut key)
            .map_err(|e| format!("Argon2id key derivation failed: {}", e))?;
        Ok(key)
    }

    /// Encrypt plaintext bytes with AES-256-GCM
    pub fn encrypt_aes_gcm(key: &[u8; KEY_LEN], plaintext: &[u8]) -> Result<(Vec<u8>, [u8; NONCE_LEN], [u8; TAG_LEN]), String> {
        let mut nonce_bytes = [0u8; NONCE_LEN];
        getrandom::getrandom(&mut nonce_bytes).map_err(|e| format!("Random bytes error: {}", e))?;

        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext_with_tag = cipher
            .encrypt(nonce, plaintext)
            .map_err(|e| format!("AES-256-GCM encryption failed: {}", e))?;

        if ciphertext_with_tag.len() < TAG_LEN {
            return Err("Ciphertext shorter than authentication tag".to_string());
        }
        let split_idx = ciphertext_with_tag.len() - TAG_LEN;
        let ciphertext = ciphertext_with_tag[..split_idx].to_vec();
        let mut tag = [0u8; TAG_LEN];
        tag.copy_from_slice(&ciphertext_with_tag[split_idx..]);

        Ok((ciphertext, nonce_bytes, tag))
    }

    /// Decrypt ciphertext bytes with AES-256-GCM
    pub fn decrypt_aes_gcm(
        key: &[u8; KEY_LEN],
        nonce: &[u8; NONCE_LEN],
        tag: &[u8; TAG_LEN],
        ciphertext: &[u8],
    ) -> Result<Vec<u8>, String> {
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
        let nonce_ref = Nonce::from_slice(nonce);

        let mut ct_with_tag = Vec::with_capacity(ciphertext.len() + TAG_LEN);
        ct_with_tag.extend_from_slice(ciphertext);
        ct_with_tag.extend_from_slice(tag);

        let plaintext = cipher
            .decrypt(nonce_ref, ct_with_tag.as_slice())
            .map_err(|_| "AES-256-GCM decryption / authentication failed (Invalid password or corrupted vault)".to_string())?;

        Ok(plaintext)
    }

    /// Initialize and create a brand new encrypted vault container file
    pub fn create_vault(file_path: &Path, password: &str) -> Result<(), String> {
        if file_path.exists() {
            return Err("Vault file already exists".to_string());
        }

        let mut salt = [0u8; SALT_LEN];
        getrandom::getrandom(&mut salt).map_err(|e| format!("Failed to generate salt: {}", e))?;

        let user_key = Self::derive_key(password, &salt)?;

        // Generate random 256-bit Master Key for the vault
        let mut master_key = [0u8; KEY_LEN];
        getrandom::getrandom(&mut master_key).map_err(|e| format!("Failed to generate master key: {}", e))?;

        // Encrypt the Master Key with user's derived key
        let (enc_master_key, master_nonce, master_tag) = Self::encrypt_aes_gcm(&user_key, &master_key)?;

        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        let header = VaultHeader {
            magic: String::from_utf8_lossy(CDVAULT_MAGIC).to_string(),
            version: 1,
            salt_hex: hex::encode(salt),
            master_nonce_hex: hex::encode(master_nonce),
            master_tag_hex: hex::encode(master_tag),
            master_key_enc_hex: hex::encode(enc_master_key),
            created_at: now,
        };

        // Initial empty manifest
        let manifest = VaultManifest { items: Vec::new() };
        let manifest_bytes = serde_json::to_vec(&manifest).map_err(|e| e.to_string())?;
        let (enc_manifest, manifest_nonce, manifest_tag) = Self::encrypt_aes_gcm(&master_key, &manifest_bytes)?;

        let mut file = File::create(file_path).map_err(|e| format!("Failed to create vault file: {}", e))?;

        // Format: [MAGIC (8)] [Header JSON Len (4)] [Header JSON] [Manifest Nonce (12)] [Manifest Tag (16)] [Manifest Len (8)] [Enc Manifest] [File Data Blocks...]
        file.write_all(CDVAULT_MAGIC).map_err(|e| e.to_string())?;

        let header_json = serde_json::to_vec(&header).map_err(|e| e.to_string())?;
        file.write_all(&(header_json.len() as u32).to_le_bytes()).map_err(|e| e.to_string())?;
        file.write_all(&header_json).map_err(|e| e.to_string())?;

        file.write_all(&manifest_nonce).map_err(|e| e.to_string())?;
        file.write_all(&manifest_tag).map_err(|e| e.to_string())?;
        file.write_all(&(enc_manifest.len() as u64).to_le_bytes()).map_err(|e| e.to_string())?;
        file.write_all(&enc_manifest).map_err(|e| e.to_string())?;

        info!("Created new Brum encrypted vault at {:?}", file_path);
        Ok(())
    }

    /// Unlock a vault with password and cache session in memory
    pub fn unlock_vault(
        &self,
        vault_path: &Path,
        password: &str,
        auto_lock_secs: u64,
    ) -> Result<String, String> {
        let mut file = File::open(vault_path).map_err(|e| format!("Cannot open vault file: {}", e))?;

        let mut magic = [0u8; 8];
        file.read_exact(&mut magic).map_err(|e| format!("Cannot read vault magic: {}", e))?;
        if &magic != CDVAULT_MAGIC {
            return Err("Invalid vault header or unsupported vault version".to_string());
        }

        let mut header_len_bytes = [0u8; 4];
        file.read_exact(&mut header_len_bytes).map_err(|e| e.to_string())?;
        let header_len = u32::from_le_bytes(header_len_bytes) as usize;

        let mut header_bytes = vec![0u8; header_len];
        file.read_exact(&mut header_bytes).map_err(|e| e.to_string())?;
        let header: VaultHeader = serde_json::from_slice(&header_bytes).map_err(|e| format!("Corrupted vault header: {}", e))?;

        let salt = hex::decode(&header.salt_hex).map_err(|e| e.to_string())?;
        let master_nonce = hex::decode(&header.master_nonce_hex).map_err(|e| e.to_string())?;
        let master_tag = hex::decode(&header.master_tag_hex).map_err(|e| e.to_string())?;
        let master_key_enc = hex::decode(&header.master_key_enc_hex).map_err(|e| e.to_string())?;

        if master_nonce.len() != NONCE_LEN || master_tag.len() != TAG_LEN {
            return Err("Corrupted vault metadata lengths".to_string());
        }

        let mut nonce_arr = [0u8; NONCE_LEN];
        nonce_arr.copy_from_slice(&master_nonce);

        let mut tag_arr = [0u8; TAG_LEN];
        tag_arr.copy_from_slice(&master_tag);

        let user_key = Self::derive_key(password, &salt)?;
        let master_key_vec = Self::decrypt_aes_gcm(&user_key, &nonce_arr, &tag_arr, &master_key_enc)?;

        if master_key_vec.len() != KEY_LEN {
            return Err("Invalid decrypted master key length".to_string());
        }

        let mut master_key = [0u8; KEY_LEN];
        master_key.copy_from_slice(&master_key_vec);

        // Read manifest
        let mut manifest_nonce = [0u8; NONCE_LEN];
        file.read_exact(&mut manifest_nonce).map_err(|e| e.to_string())?;

        let mut manifest_tag = [0u8; TAG_LEN];
        file.read_exact(&mut manifest_tag).map_err(|e| e.to_string())?;

        let mut manifest_len_bytes = [0u8; 8];
        file.read_exact(&mut manifest_len_bytes).map_err(|e| e.to_string())?;
        let manifest_len = u64::from_le_bytes(manifest_len_bytes) as usize;

        let mut enc_manifest = vec![0u8; manifest_len];
        file.read_exact(&mut enc_manifest).map_err(|e| e.to_string())?;

        let manifest_bytes = Self::decrypt_aes_gcm(&master_key, &manifest_nonce, &manifest_tag, &enc_manifest)?;
        let manifest: VaultManifest = serde_json::from_slice(&manifest_bytes).map_err(|e| format!("Corrupted vault manifest: {}", e))?;

        let norm_path_str = vault_path.to_string_lossy().to_string();
        let auto_lock = Duration::from_secs(if auto_lock_secs == 0 { 900 } else { auto_lock_secs }); // Default 15 mins

        let mut sessions = self.sessions.write().map_err(|_| "Vault lock error".to_string())?;
        sessions.insert(norm_path_str.clone(), UnlockedVaultSession {
            vault_path: vault_path.to_path_buf(),
            master_key,
            manifest,
            last_accessed: Instant::now(),
            auto_lock_duration: auto_lock,
        });

        info!("Successfully unlocked vault: {}", norm_path_str);
        Ok(norm_path_str)
    }

    /// Check if a vault is currently unlocked and valid in memory
    pub fn is_unlocked(&self, vault_path_str: &str) -> bool {
        if let Ok(sessions) = self.sessions.read() {
            if let Some(session) = sessions.get(vault_path_str) {
                return session.last_accessed.elapsed() < session.auto_lock_duration;
            }
        }
        false
    }

    /// Lock a vault and purge master key from memory immediately
    pub fn lock_vault(&self, vault_path_str: &str) {
        if let Ok(mut sessions) = self.sessions.write() {
            if sessions.remove(vault_path_str).is_some() {
                info!("Locked vault and purged key from RAM: {}", vault_path_str);
            }
        }
    }

    /// List contents of unlocked vault under virtual subpath
    pub fn list_vault_contents(&self, vault_path_str: &str, subpath_filter: &str) -> Result<DirectoryListing, String> {
        let mut sessions = self.sessions.write().map_err(|_| "Vault lock error".to_string())?;
        let session = sessions.get_mut(vault_path_str).ok_or_else(|| "Vault is locked. Enter password to access.".to_string())?;

        if session.last_accessed.elapsed() >= session.auto_lock_duration {
            sessions.remove(vault_path_str);
            return Err("Vault session expired and locked automatically.".to_string());
        }
        session.last_accessed = Instant::now();

        let clean_sub = subpath_filter.trim_matches('/');
        let mut entries = Vec::new();
        let mut total_files = 0;
        let mut total_dirs = 0;
        let mut total_size = 0u64;

        for item in &session.manifest.items {
            let item_path = item.path.trim_matches('/');
            if !clean_sub.is_empty() && !item_path.starts_with(clean_sub) {
                continue;
            }

            let rel = if clean_sub.is_empty() {
                item_path
            } else {
                item_path.strip_prefix(clean_sub).unwrap_or("").trim_matches('/')
            };

            if rel.is_empty() {
                continue;
            }

            let first_segment = rel.split('/').next().unwrap_or("");
            if first_segment.is_empty() {
                continue;
            }

            let is_child_dir = rel.contains('/') || item.is_dir;
            let display_name = first_segment.to_string();

            if entries.iter().any(|e: &FileEntry| e.name == display_name) {
                continue;
            }

            let full_virtual_path = if clean_sub.is_empty() {
                format!("vault://{}#{}", vault_path_str, display_name)
            } else {
                format!("vault://{}#{}/{}", vault_path_str, clean_sub, display_name)
            };

            if is_child_dir {
                total_dirs += 1;
            } else {
                total_files += 1;
                total_size += item.size;
            }

            entries.push(FileEntry {
                name: display_name,
                path: full_virtual_path,
                is_dir: is_child_dir,
                is_symlink: false,
                is_empty: None,
                size: if is_child_dir { 0 } else { item.size },
                modified: Some(item.modified),
                permissions: if is_child_dir { "drwx------".to_string() } else { "-rw-------".to_string() },
                mode_octal: if is_child_dir { "0700".to_string() } else { "0600".to_string() },
                owner: "vault".to_string(),
                group: "vault".to_string(),
                uid: 0,
                gid: 0,
                mime_type: item.mime_type.clone(),
                is_archive: false,
            });
        }

        // Sort directories first then files
        entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        });

        let parent_path = if clean_sub.is_empty() {
            None
        } else if let Some((parent, _)) = clean_sub.rsplit_once('/') {
            Some(format!("vault://{}#{}", vault_path_str, parent))
        } else {
            Some(format!("vault://{}#", vault_path_str))
        };

        Ok(DirectoryListing {
            current_path: format!("vault://{}#{}", vault_path_str, clean_sub),
            parent_path,
            entries,
            total_files,
            total_dirs,
            total_size,
            protocol: "vault".to_string(),
            is_truncated: None,
            max_limit: None,
        })
    }

    /// Read and decrypt file content on-the-fly from vault
    pub fn read_vault_file(&self, vault_path_str: &str, file_subpath: &str) -> Result<FileContentResponse, String> {
        let mut sessions = self.sessions.write().map_err(|_| "Vault lock error".to_string())?;
        let session = sessions.get_mut(vault_path_str).ok_or_else(|| "Vault is locked. Enter password to access.".to_string())?;

        if session.last_accessed.elapsed() >= session.auto_lock_duration {
            sessions.remove(vault_path_str);
            return Err("Vault session expired and locked automatically.".to_string());
        }
        session.last_accessed = Instant::now();

        let clean_path = file_subpath.trim_matches('/');
        let item = session.manifest.items.iter().find(|i| i.path.trim_matches('/') == clean_path)
            .ok_or_else(|| format!("File not found in encrypted vault: {}", file_subpath))?;

        if item.is_dir {
            return Err("Target is a directory, not a file".to_string());
        }

        // Read encrypted data block from file
        let decrypted_bytes = Self::read_encrypted_block(&session.vault_path, &session.master_key, &item.id)?;

        let is_binary = decrypted_bytes.iter().take(1024).any(|&b| b == 0);
        let content_str = if is_binary {
            format!("data:application/octet-stream;base64,{}", base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &decrypted_bytes))
        } else {
            String::from_utf8_lossy(&decrypted_bytes).to_string()
        };

        Ok(FileContentResponse {
            path: format!("vault://{}#{}", vault_path_str, clean_path),
            name: item.name.clone(),
            content: content_str,
            is_binary,
            size: item.size,
            mime_type: item.mime_type.clone().unwrap_or_else(|| "text/plain".to_string()),
        })
    }

    /// Write or update file inside encrypted vault
    pub fn write_vault_file(
        &self,
        vault_path_str: &str,
        file_subpath: &str,
        plaintext: &[u8],
    ) -> Result<(), String> {
        let mut sessions = self.sessions.write().map_err(|_| "Vault lock error".to_string())?;
        let session = sessions.get_mut(vault_path_str).ok_or_else(|| "Vault is locked. Enter password to access.".to_string())?;

        if session.last_accessed.elapsed() >= session.auto_lock_duration {
            sessions.remove(vault_path_str);
            return Err("Vault session expired and locked automatically.".to_string());
        }
        session.last_accessed = Instant::now();

        let clean_path = file_subpath.trim_matches('/');
        let file_name = clean_path.split('/').last().unwrap_or(clean_path).to_string();
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();

        // Check if item already exists or create new
        let storage_id = if let Some(existing) = session.manifest.items.iter_mut().find(|i| i.path.trim_matches('/') == clean_path) {
            existing.size = plaintext.len() as u64;
            existing.modified = now;
            existing.id.clone()
        } else {
            let new_id = uuid::Uuid::new_v4().to_string();
            session.manifest.items.push(VaultManifestItem {
                id: new_id.clone(),
                name: file_name,
                path: clean_path.to_string(),
                is_dir: false,
                size: plaintext.len() as u64,
                modified: now,
                mime_type: None,
            });
            new_id
        };

        // Write encrypted block & manifest to vault
        Self::save_vault_with_manifest_and_block(
            &session.vault_path,
            &session.master_key,
            &session.manifest,
            &storage_id,
            plaintext,
        )?;

        info!("Encrypted and wrote file '{}' ({} bytes) to vault {}", clean_path, plaintext.len(), vault_path_str);
        Ok(())
    }

    /// Delete file or directory inside encrypted vault
    pub fn delete_vault_file(&self, vault_path_str: &str, file_subpath: &str) -> Result<(), String> {
        let mut sessions = self.sessions.write().map_err(|_| "Vault lock error".to_string())?;
        let session = sessions.get_mut(vault_path_str).ok_or_else(|| "Vault is locked. Enter password to access.".to_string())?;

        let clean_path = file_subpath.trim_matches('/');
        session.manifest.items.retain(|i| {
            let p = i.path.trim_matches('/');
            p != clean_path && !p.starts_with(&format!("{}/", clean_path))
        });

        Self::rewrite_manifest_in_vault(&session.vault_path, &session.master_key, &session.manifest)?;
        info!("Deleted '{}' from encrypted vault {}", clean_path, vault_path_str);
        Ok(())
    }

    /// Create directory inside encrypted vault
    pub fn mkdir_vault(&self, vault_path_str: &str, dir_subpath: &str) -> Result<(), String> {
        let mut sessions = self.sessions.write().map_err(|_| "Vault lock error".to_string())?;
        let session = sessions.get_mut(vault_path_str).ok_or_else(|| "Vault is locked. Enter password to access.".to_string())?;

        let clean_path = dir_subpath.trim_matches('/');
        let dir_name = clean_path.split('/').last().unwrap_or(clean_path).to_string();
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();

        if !session.manifest.items.iter().any(|i| i.path.trim_matches('/') == clean_path) {
            session.manifest.items.push(VaultManifestItem {
                id: uuid::Uuid::new_v4().to_string(),
                name: dir_name,
                path: clean_path.to_string(),
                is_dir: true,
                size: 0,
                modified: now,
                mime_type: None,
            });
            Self::rewrite_manifest_in_vault(&session.vault_path, &session.master_key, &session.manifest)?;
        }
        Ok(())
    }

    // --- Low-Level Block Storage Helpers ---

    fn read_encrypted_block(vault_path: &Path, master_key: &[u8; KEY_LEN], target_id: &str) -> Result<Vec<u8>, String> {
        let mut file = File::open(vault_path).map_err(|e| e.to_string())?;
        let mut magic = [0u8; 8];
        file.read_exact(&mut magic).map_err(|e| e.to_string())?;

        let mut header_len_bytes = [0u8; 4];
        file.read_exact(&mut header_len_bytes).map_err(|e| e.to_string())?;
        let header_len = u32::from_le_bytes(header_len_bytes) as usize;

        let mut header_bytes = vec![0u8; header_len];
        file.read_exact(&mut header_bytes).map_err(|e| e.to_string())?;

        let mut manifest_nonce = [0u8; NONCE_LEN];
        file.read_exact(&mut manifest_nonce).map_err(|e| e.to_string())?;

        let mut manifest_tag = [0u8; TAG_LEN];
        file.read_exact(&mut manifest_tag).map_err(|e| e.to_string())?;

        let mut manifest_len_bytes = [0u8; 8];
        file.read_exact(&mut manifest_len_bytes).map_err(|e| e.to_string())?;
        let manifest_len = u64::from_le_bytes(manifest_len_bytes) as usize;

        let mut enc_manifest = vec![0u8; manifest_len];
        file.read_exact(&mut enc_manifest).map_err(|e| e.to_string())?;

        // Stream through blocks: [Block ID len (2)] [Block ID] [Block Nonce (12)] [Block Tag (16)] [Data Len (8)] [Enc Data]
        loop {
            let mut id_len_bytes = [0u8; 2];
            if file.read_exact(&mut id_len_bytes).is_err() {
                break;
            }
            let id_len = u16::from_le_bytes(id_len_bytes) as usize;
            let mut id_bytes = vec![0u8; id_len];
            file.read_exact(&mut id_bytes).map_err(|e| e.to_string())?;
            let block_id = String::from_utf8_lossy(&id_bytes).to_string();

            let mut block_nonce = [0u8; NONCE_LEN];
            file.read_exact(&mut block_nonce).map_err(|e| e.to_string())?;

            let mut block_tag = [0u8; TAG_LEN];
            file.read_exact(&mut block_tag).map_err(|e| e.to_string())?;

            let mut data_len_bytes = [0u8; 8];
            file.read_exact(&mut data_len_bytes).map_err(|e| e.to_string())?;
            let data_len = u64::from_le_bytes(data_len_bytes) as usize;

            let mut enc_data = vec![0u8; data_len];
            file.read_exact(&mut enc_data).map_err(|e| e.to_string())?;

            if block_id == target_id {
                return Self::decrypt_aes_gcm(master_key, &block_nonce, &block_tag, &enc_data);
            }
        }

        Err(format!("Data block '{}' not found in vault container", target_id))
    }

    fn rewrite_manifest_in_vault(vault_path: &Path, master_key: &[u8; KEY_LEN], manifest: &VaultManifest) -> Result<(), String> {
        let mut blocks = Vec::new();

        // Read all existing file data blocks first
        if let Ok(mut file) = File::open(vault_path) {
            let mut magic = [0u8; 8];
            if file.read_exact(&mut magic).is_ok() && &magic == CDVAULT_MAGIC {
                let mut h_len_b = [0u8; 4];
                if file.read_exact(&mut h_len_b).is_ok() {
                    let h_len = u32::from_le_bytes(h_len_b) as usize;
                    let mut h_b = vec![0u8; h_len];
                    let _ = file.read_exact(&mut h_b);
                    let mut m_nonce = [0u8; NONCE_LEN];
                    let _ = file.read_exact(&mut m_nonce);
                    let mut m_tag = [0u8; TAG_LEN];
                    let _ = file.read_exact(&mut m_tag);
                    let mut m_len_b = [0u8; 8];
                    if file.read_exact(&mut m_len_b).is_ok() {
                        let m_len = u64::from_le_bytes(m_len_b) as usize;
                        let mut enc_m = vec![0u8; m_len];
                        let _ = file.read_exact(&mut enc_m);

                        // Read remaining raw blocks
                        let mut remaining = Vec::new();
                        let _ = file.read_to_end(&mut remaining);
                        blocks = remaining;
                    }
                }
            }
        }

        // Re-read header to preserve salt & master key wrapping
        let mut orig_file = File::open(vault_path).map_err(|e| e.to_string())?;
        let mut magic = [0u8; 8];
        orig_file.read_exact(&mut magic).map_err(|e| e.to_string())?;
        let mut h_len_b = [0u8; 4];
        orig_file.read_exact(&mut h_len_b).map_err(|e| e.to_string())?;
        let h_len = u32::from_le_bytes(h_len_b) as usize;
        let mut h_bytes = vec![0u8; h_len];
        orig_file.read_exact(&mut h_bytes).map_err(|e| e.to_string())?;

        // Encrypt new manifest
        let manifest_bytes = serde_json::to_vec(manifest).map_err(|e| e.to_string())?;
        let (enc_manifest, manifest_nonce, manifest_tag) = Self::encrypt_aes_gcm(master_key, &manifest_bytes)?;

        // Write atomic temp file
        let temp_path = vault_path.with_extension("cdvault.tmp");
        let mut out = File::create(&temp_path).map_err(|e| e.to_string())?;

        out.write_all(CDVAULT_MAGIC).map_err(|e| e.to_string())?;
        out.write_all(&(h_len as u32).to_le_bytes()).map_err(|e| e.to_string())?;
        out.write_all(&h_bytes).map_err(|e| e.to_string())?;

        out.write_all(&manifest_nonce).map_err(|e| e.to_string())?;
        out.write_all(&manifest_tag).map_err(|e| e.to_string())?;
        out.write_all(&(enc_manifest.len() as u64).to_le_bytes()).map_err(|e| e.to_string())?;
        out.write_all(&enc_manifest).map_err(|e| e.to_string())?;

        out.write_all(&blocks).map_err(|e| e.to_string())?;
        out.flush().map_err(|e| e.to_string())?;

        fs::rename(&temp_path, vault_path).map_err(|e| format!("Failed to replace vault file: {}", e))?;
        Ok(())
    }

    fn save_vault_with_manifest_and_block(
        vault_path: &Path,
        master_key: &[u8; KEY_LEN],
        manifest: &VaultManifest,
        target_block_id: &str,
        new_block_data: &[u8],
    ) -> Result<(), String> {
        let (enc_block_data, block_nonce, block_tag) = Self::encrypt_aes_gcm(master_key, new_block_data)?;

        // Read all existing blocks except target_block_id
        let mut other_blocks = Vec::new();
        if let Ok(mut file) = File::open(vault_path) {
            let mut magic = [0u8; 8];
            if file.read_exact(&mut magic).is_ok() && &magic == CDVAULT_MAGIC {
                let mut h_len_b = [0u8; 4];
                if file.read_exact(&mut h_len_b).is_ok() {
                    let h_len = u32::from_le_bytes(h_len_b) as usize;
                    let mut h_b = vec![0u8; h_len];
                    let _ = file.read_exact(&mut h_b);
                    let mut m_nonce = [0u8; NONCE_LEN];
                    let _ = file.read_exact(&mut m_nonce);
                    let mut m_tag = [0u8; TAG_LEN];
                    let _ = file.read_exact(&mut m_tag);
                    let mut m_len_b = [0u8; 8];
                    if file.read_exact(&mut m_len_b).is_ok() {
                        let m_len = u64::from_le_bytes(m_len_b) as usize;
                        let mut enc_m = vec![0u8; m_len];
                        let _ = file.read_exact(&mut enc_m);

                        loop {
                            let mut id_len_bytes = [0u8; 2];
                            if file.read_exact(&mut id_len_bytes).is_err() {
                                break;
                            }
                            let id_len = u16::from_le_bytes(id_len_bytes) as usize;
                            let mut id_bytes = vec![0u8; id_len];
                            if file.read_exact(&mut id_bytes).is_err() {
                                break;
                            }
                            let b_id = String::from_utf8_lossy(&id_bytes).to_string();

                            let mut b_nonce = [0u8; NONCE_LEN];
                            let _ = file.read_exact(&mut b_nonce);

                            let mut b_tag = [0u8; TAG_LEN];
                            let _ = file.read_exact(&mut b_tag);

                            let mut d_len_b = [0u8; 8];
                            let _ = file.read_exact(&mut d_len_b);
                            let d_len = u64::from_le_bytes(d_len_b) as usize;

                            let mut enc_d = vec![0u8; d_len];
                            let _ = file.read_exact(&mut enc_d);

                            if b_id != target_block_id {
                                other_blocks.extend_from_slice(&(id_len as u16).to_le_bytes());
                                other_blocks.extend_from_slice(&id_bytes);
                                other_blocks.extend_from_slice(&b_nonce);
                                other_blocks.extend_from_slice(&b_tag);
                                other_blocks.extend_from_slice(&(d_len as u64).to_le_bytes());
                                other_blocks.extend_from_slice(&enc_d);
                            }
                        }
                    }
                }
            }
        }

        // Re-read header to preserve salt & master key wrapping
        let mut orig_file = File::open(vault_path).map_err(|e| e.to_string())?;
        let mut magic = [0u8; 8];
        orig_file.read_exact(&mut magic).map_err(|e| e.to_string())?;
        let mut h_len_b = [0u8; 4];
        orig_file.read_exact(&mut h_len_b).map_err(|e| e.to_string())?;
        let h_len = u32::from_le_bytes(h_len_b) as usize;
        let mut h_bytes = vec![0u8; h_len];
        orig_file.read_exact(&mut h_bytes).map_err(|e| e.to_string())?;

        // Encrypt new manifest
        let manifest_bytes = serde_json::to_vec(manifest).map_err(|e| e.to_string())?;
        let (enc_manifest, manifest_nonce, manifest_tag) = Self::encrypt_aes_gcm(master_key, &manifest_bytes)?;

        // Write atomic temp file
        let temp_path = vault_path.with_extension("cdvault.tmp");
        let mut out = File::create(&temp_path).map_err(|e| e.to_string())?;

        out.write_all(CDVAULT_MAGIC).map_err(|e| e.to_string())?;
        out.write_all(&(h_len as u32).to_le_bytes()).map_err(|e| e.to_string())?;
        out.write_all(&h_bytes).map_err(|e| e.to_string())?;

        out.write_all(&manifest_nonce).map_err(|e| e.to_string())?;
        out.write_all(&manifest_tag).map_err(|e| e.to_string())?;
        out.write_all(&(enc_manifest.len() as u64).to_le_bytes()).map_err(|e| e.to_string())?;
        out.write_all(&enc_manifest).map_err(|e| e.to_string())?;

        // Write other blocks
        out.write_all(&other_blocks).map_err(|e| e.to_string())?;

        // Write new block
        let id_bytes = target_block_id.as_bytes();
        out.write_all(&(id_bytes.len() as u16).to_le_bytes()).map_err(|e| e.to_string())?;
        out.write_all(id_bytes).map_err(|e| e.to_string())?;
        out.write_all(&block_nonce).map_err(|e| e.to_string())?;
        out.write_all(&block_tag).map_err(|e| e.to_string())?;
        out.write_all(&(enc_block_data.len() as u64).to_le_bytes()).map_err(|e| e.to_string())?;
        out.write_all(&enc_block_data).map_err(|e| e.to_string())?;

        out.flush().map_err(|e| e.to_string())?;
        fs::rename(&temp_path, vault_path).map_err(|e| format!("Failed to replace vault file: {}", e))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vault_create_unlock_write_read_cycle() {
        let temp_dir = tempfile::tempdir().unwrap();
        let vault_file = temp_dir.path().join("test_secrets.cdvault");
        let password = "SuperSecretPassword123!";

        // 1. Create new vault
        VaultManager::create_vault(&vault_file, password).unwrap();
        assert!(vault_file.exists());

        // 2. Unlock vault
        let mgr = VaultManager::new();
        let norm_path = mgr.unlock_vault(&vault_file, password, 60).unwrap();
        assert!(mgr.is_unlocked(&norm_path));

        // 3. Write file into encrypted vault
        let file_data = b"Hello from inside the encrypted vault! Sensitive Data 2026.";
        mgr.write_vault_file(&norm_path, "passwords/wifi.txt", file_data).unwrap();

        // 4. List vault directory
        let listing = mgr.list_vault_contents(&norm_path, "passwords").unwrap();
        assert_eq!(listing.entries.len(), 1);
        assert_eq!(listing.entries[0].name, "wifi.txt");
        assert_eq!(listing.entries[0].size, file_data.len() as u64);

        // 5. Read and decrypt file content in memory
        let read_res = mgr.read_vault_file(&norm_path, "passwords/wifi.txt").unwrap();
        assert_eq!(read_res.content, String::from_utf8_lossy(file_data));

        // 6. Lock vault
        mgr.lock_vault(&norm_path);
        assert!(!mgr.is_unlocked(&norm_path));

        // 7. Reading when locked must fail
        assert!(mgr.read_vault_file(&norm_path, "passwords/wifi.txt").is_err());

        // 8. Unlocking with wrong password must fail authentication
        assert!(mgr.unlock_vault(&vault_file, "WrongPassword!", 60).is_err());
    }
}
