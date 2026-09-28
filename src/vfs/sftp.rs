use super::{is_archive_file, DirectoryListing, FileEntry};
use ssh2::{KeyboardInteractivePrompt, Prompt, Session};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct SftpParams {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: Option<String>,
    pub key_path: Option<String>,
    pub remote_path: String,
}

struct KbdInteractiveHelper<'a> {
    password: &'a str,
}

impl<'a> KeyboardInteractivePrompt for KbdInteractiveHelper<'a> {
    fn prompt<'b>(
        &mut self,
        _username: &str,
        _instructions: &str,
        prompts: &[Prompt<'b>],
    ) -> Vec<String> {
        prompts.iter().map(|_| self.password.to_string()).collect()
    }
}

pub struct SftpClient;

impl SftpClient {
    pub fn connect(params: &SftpParams) -> Result<Session, std::io::Error> {
        let addr_str = format!("{}:{}", params.host, params.port);
        let socket_addrs: Vec<_> = addr_str.to_socket_addrs()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::AddrNotAvailable, format!("Cannot resolve host {}: {}", params.host, e)))?
            .collect();

        if socket_addrs.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AddrNotAvailable,
                format!("No IP addresses found for host '{}'", params.host),
            ));
        }

        let tcp = TcpStream::connect_timeout(&socket_addrs[0], Duration::from_secs(10))
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::TimedOut, format!("TCP connection to {}:{} failed: {}", params.host, params.port, e)))?;

        let mut sess = Session::new()?;
        sess.set_tcp_stream(tcp);
        sess.set_timeout(15000); // 15s timeout

        // Configure modern Key Exchange (KEX), HostKey, Cipher, and MAC algorithm preferences
        // Modern OpenSSH servers (8.8+, 9.x, 10.x) require modern SHA-256/512 and elliptic curve ciphers
        let _ = sess.method_pref(
            ssh2::MethodType::Kex,
            "curve25519-sha256,curve25519-sha256@libssh.org,ecdh-sha2-nistp256,ecdh-sha2-nistp384,ecdh-sha2-nistp521,diffie-hellman-group-exchange-sha256,diffie-hellman-group16-sha512,diffie-hellman-group18-sha512,diffie-hellman-group14-sha256,diffie-hellman-group14-sha1,diffie-hellman-group-exchange-sha1,diffie-hellman-group1-sha1",
        );
        let _ = sess.method_pref(
            ssh2::MethodType::HostKey,
            "ssh-ed25519,ecdsa-sha2-nistp256,ecdsa-sha2-nistp384,ecdsa-sha2-nistp521,rsa-sha2-512,rsa-sha2-256,ssh-rsa,ssh-dss",
        );
        let _ = sess.method_pref(
            ssh2::MethodType::CryptCs,
            "chacha20-poly1305@openssh.com,aes256-gcm@openssh.com,aes128-gcm@openssh.com,aes256-ctr,aes192-ctr,aes128-ctr,aes256-cbc,aes192-cbc,aes128-cbc,3des-cbc",
        );
        let _ = sess.method_pref(
            ssh2::MethodType::CryptSc,
            "chacha20-poly1305@openssh.com,aes256-gcm@openssh.com,aes128-gcm@openssh.com,aes256-ctr,aes192-ctr,aes128-ctr,aes256-cbc,aes192-cbc,aes128-cbc,3des-cbc",
        );
        let _ = sess.method_pref(
            ssh2::MethodType::MacCs,
            "hmac-sha2-256-etm@openssh.com,hmac-sha2-512-etm@openssh.com,hmac-sha2-256,hmac-sha2-512,hmac-sha1,hmac-sha1-96,umac-64-etm@openssh.com,umac-128-etm@openssh.com,umac-64@openssh.com,umac-128@openssh.com",
        );
        let _ = sess.method_pref(
            ssh2::MethodType::MacSc,
            "hmac-sha2-256-etm@openssh.com,hmac-sha2-512-etm@openssh.com,hmac-sha2-256,hmac-sha2-512,hmac-sha1,hmac-sha1-96,umac-64-etm@openssh.com,umac-128-etm@openssh.com,umac-64@openssh.com,umac-128@openssh.com",
        );
        let _ = sess.method_pref(
            ssh2::MethodType::CompCs,
            "none,zlib@openssh.com,zlib",
        );
        let _ = sess.method_pref(
            ssh2::MethodType::CompSc,
            "none,zlib@openssh.com,zlib",
        );

        sess.handshake()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::ConnectionRefused, format!("SSH handshake failed with {}:{}: {}", params.host, params.port, e)))?;

        // 1. Password authentication
        if let Some(password) = params.password.as_deref().filter(|p| !p.is_empty()) {
            if sess.userauth_password(&params.user, password).is_ok() && sess.authenticated() {
                return Ok(sess);
            }

            // 1b. Fallback to keyboard-interactive authentication with provided password
            let mut prompt_handler = KbdInteractiveHelper { password };
            if sess.userauth_keyboard_interactive(&params.user, &mut prompt_handler).is_ok() && sess.authenticated() {
                return Ok(sess);
            }
        }

        // 2. Explicit key file authentication
        if let Some(ref key_path) = params.key_path {
            let p = Path::new(key_path);
            if p.exists() {
                if sess.userauth_pubkey_file(&params.user, None, p, params.password.as_deref()).is_ok() && sess.authenticated() {
                    return Ok(sess);
                }
            }
        }

        // 3. SSH Agent authentication (gracefully ignore if socket does not exist or agent fails)
        if !sess.authenticated() {
            if let Ok(mut agent) = sess.agent() {
                if agent.connect().is_ok() && agent.list_identities().is_ok() {
                    if let Ok(identities) = agent.identities() {
                        for identity in identities {
                            if agent.userauth(&params.user, &identity).is_ok() && sess.authenticated() {
                                break;
                            }
                        }
                    }
                }
            }
        }

        // 4. Default user SSH keys in ~/.ssh/
        if !sess.authenticated() {
            if let Some(home) = dirs::home_dir() {
                let ssh_dir = home.join(".ssh");
                for key_name in &["id_ed25519", "id_rsa", "id_ecdsa", "id_dsa"] {
                    let priv_key = ssh_dir.join(key_name);
                    if priv_key.exists() {
                        let pub_key = ssh_dir.join(format!("{}.pub", key_name));
                        let pub_ref = if pub_key.exists() { Some(pub_key.as_path()) } else { None };
                        if sess.userauth_pubkey_file(&params.user, pub_ref, &priv_key, params.password.as_deref()).is_ok() && sess.authenticated() {
                            break;
                        }
                    }
                }
            }
        }

        if !sess.authenticated() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!(
                    "SFTP authentication failed for user '{}' at {}:{}. Please verify password or SSH key.",
                    params.user, params.host, params.port
                ),
            ));
        }

        Ok(sess)
    }

    pub fn resolve_remote_path(sftp: &ssh2::Sftp, raw_path: &str) -> String {
        let clean = raw_path.trim();
        if clean.is_empty() || clean == "." || clean == "~" {
            if let Ok(real) = sftp.realpath(Path::new(".")) {
                return real.to_string_lossy().to_string();
            }
            "/".to_string()
        } else if clean.starts_with("~/") {
            let sub = clean.trim_start_matches("~/");
            if let Ok(real) = sftp.realpath(Path::new(".")) {
                let home_str = real.to_string_lossy().to_string();
                format!("{}/{}", home_str.trim_end_matches('/'), sub)
            } else {
                format!("/{}", sub)
            }
        } else {
            clean.to_string()
        }
    }

    pub fn list_dir(params: &SftpParams) -> Result<DirectoryListing, std::io::Error> {
        let sess = Self::connect(params)?;
        let sftp = sess.sftp().map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("SFTP subsystem initialization failed: {}", e)))?;

        let resolved_path = Self::resolve_remote_path(&sftp, &params.remote_path);
        let path = if resolved_path.is_empty() { "/" } else { &resolved_path };
        let entries_raw = sftp.readdir(Path::new(path))
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("Failed to read SFTP directory '{}': {}", path, e)))?;

        let mut entries = Vec::new();
        let mut total_files = 0;
        let mut total_dirs = 0;
        let mut total_size = 0u64;

        for (p, stat) in entries_raw {
            let name = p.file_name().unwrap_or_default().to_string_lossy().to_string();
            if name == "." || name == ".." {
                continue;
            }

            let is_dir = stat.is_dir();
            let size = stat.size.unwrap_or(0);
            let modified = stat.mtime;
            let raw_perm = stat.perm.unwrap_or(0o644);
            let mode_octal = format!("{:04o}", raw_perm & 0o7777);
            let permissions = if is_dir { format!("d{:o}", raw_perm & 0o777) } else { format!("-{:o}", raw_perm & 0o777) };
            let is_archive = !is_dir && is_archive_file(&name);

            let uid = stat.uid.unwrap_or(1000);
            let gid = stat.gid.unwrap_or(1000);

            if is_dir {
                total_dirs += 1;
            } else {
                total_files += 1;
                total_size += size;
            }

            let full_path = if path == "/" {
                format!("/{}", name)
            } else {
                format!("{}/{}", path.trim_end_matches('/'), name)
            };

            let entry_uri = format!("sftp://{}@{}:{}{}", percent_encode(&params.user), params.host, params.port, full_path);

            entries.push(FileEntry {
                name: name.clone(),
                path: entry_uri,
                is_dir,
                is_symlink: false,
                is_empty: None,
                size,
                modified,
                permissions,
                mode_octal,
                owner: params.user.clone(),
                group: "remote".to_string(),
                uid,
                gid,
                mime_type: if is_dir { None } else { Some(mime_guess::from_path(&name).first_or_octet_stream().to_string()) },
                is_archive,
            });
        }

        entries.sort_by(|a, b| {
            if a.is_dir == b.is_dir {
                a.name.to_lowercase().cmp(&b.name.to_lowercase())
            } else if a.is_dir {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Greater
            }
        });

        let parent_path = Path::new(path).parent().map(|p| {
            let p_str = p.to_string_lossy();
            let final_p = if p_str.is_empty() { "/" } else { &p_str };
            format!("sftp://{}@{}:{}{}", percent_encode(&params.user), params.host, params.port, final_p)
        });

        let current_uri = format!("sftp://{}@{}:{}{}", percent_encode(&params.user), params.host, params.port, path);

        Ok(DirectoryListing {
            current_path: current_uri,
            parent_path,
            entries,
            total_files,
            total_dirs,
            total_size,
            protocol: "sftp".to_string(),
            is_truncated: None,
            max_limit: None,
        })
    }

    pub fn download_file(
        host: &str,
        port: u16,
        user: &str,
        pass: Option<&str>,
        remote_path: &str,
        max_bytes: usize,
    ) -> Result<Vec<u8>, std::io::Error> {
        let params = SftpParams {
            host: host.to_string(),
            port,
            user: user.to_string(),
            password: pass.map(|s| s.to_string()),
            key_path: None,
            remote_path: remote_path.to_string(),
        };
        let sess = Self::connect(&params)?;
        let sftp = sess.sftp().map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("SFTP subsystem error: {}", e)))?;
        let resolved = Self::resolve_remote_path(&sftp, remote_path);
        let mut file = sftp.open(Path::new(&resolved))
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("SFTP open error for '{}': {}", resolved, e)))?;

        let mut buffer = Vec::new();
        if max_bytes > 0 {
            let mut handle = (&mut file).take(max_bytes as u64);
            handle.read_to_end(&mut buffer)?;
        } else {
            file.read_to_end(&mut buffer)?;
        }

        Ok(buffer)
    }

    pub fn write_file(params: &SftpParams, data: &[u8]) -> Result<(), std::io::Error> {
        let sess = Self::connect(params)?;
        let sftp = sess.sftp().map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("SFTP subsystem error: {}", e)))?;
        let resolved = Self::resolve_remote_path(&sftp, &params.remote_path);
        let mut file = sftp.create(Path::new(&resolved))
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("SFTP create file error for '{}': {}", resolved, e)))?;
        file.write_all(data)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("SFTP write error for '{}': {}", resolved, e)))?;
        Ok(())
    }

    pub fn parse_uri(uri: &str, default_user: Option<&str>, default_pass: Option<&str>) -> Result<SftpParams, String> {
        let clean = if let Some(stripped) = uri.strip_prefix("sftp://") {
            stripped
        } else if let Some(stripped) = uri.strip_prefix("ssh://") {
            stripped
        } else {
            return Err("Invalid SFTP/SSH URI: must start with 'sftp://' or 'ssh://'".to_string());
        };

        let (auth_part, host_and_path) = if let Some(at) = clean.rfind('@') {
            (&clean[..at], &clean[at + 1..])
        } else {
            ("", clean)
        };

        let current_sys_user = std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_else(|_| "root".to_string());
        let mut user = default_user.unwrap_or(&current_sys_user).to_string();
        let mut password = default_pass.filter(|p| !p.trim().is_empty()).map(|s| s.to_string());

        if !auth_part.is_empty() {
            if let Some(colon) = auth_part.find(':') {
                user = percent_decode(&auth_part[..colon]);
                password = Some(percent_decode(&auth_part[colon + 1..]));
            } else {
                user = percent_decode(auth_part);
            }
        }

        let (host_port, remote_path) = if let Some(slash) = host_and_path.find('/') {
            (&host_and_path[..slash], &host_and_path[slash..])
        } else {
            (host_and_path, "")
        };

        let (host, port) = if let Some(colon) = host_port.rfind(':') {
            let h = &host_port[..colon];
            let p = host_port[colon + 1..].parse::<u16>().unwrap_or(22);
            (h.to_string(), p)
        } else {
            (host_port.to_string(), 22)
        };

        let final_path = remote_path.to_string();

        Ok(SftpParams {
            host,
            port,
            user,
            password,
            key_path: None,
            remote_path: final_path,
        })
    }

    pub fn stat_path(params: &SftpParams) -> Result<(u64, bool), String> {
        let sess = Self::connect(params)
            .map_err(|e| format!("SFTP connect error: {}", e))?;
        let sftp = sess.sftp().map_err(|e| format!("SFTP session error: {}", e))?;
        let resolved = Self::resolve_remote_path(&sftp, &params.remote_path);
        let stat = sftp.stat(Path::new(&resolved))
            .map_err(|e| format!("SFTP stat error for {}: {}", resolved, e))?;
        let is_dir = stat.is_dir();
        let size = stat.size.unwrap_or(0);
        Ok((size, is_dir))
    }

    pub fn scan_dir_totals(params: &SftpParams) -> Result<(u64, u64), String> {
        let mut total_files = 0u64;
        let mut total_bytes = 0u64;
        let listing = Self::list_dir(params).map_err(|e| e.to_string())?;
        for entry in listing.entries {
            if entry.is_dir {
                let child_remote = if params.remote_path == "/" {
                    format!("/{}", entry.name)
                } else {
                    format!("{}/{}", params.remote_path.trim_end_matches('/'), entry.name)
                };
                let mut child_params = params.clone();
                child_params.remote_path = child_remote;
                if let Ok((c, b)) = Self::scan_dir_totals(&child_params) {
                    total_files += c;
                    total_bytes += b;
                }
            } else {
                total_files += 1;
                total_bytes += entry.size;
            }
        }
        Ok((total_files, total_bytes))
    }

    pub fn download_to_file(params: &SftpParams, local_dest: &Path) -> Result<(), String> {
        Self::download_to_file_streaming(params, local_dest, false, |_, _, _| Ok(())).map(|_| ())
    }

    pub fn download_to_file_streaming<F>(
        params: &SftpParams,
        local_dest: &Path,
        verify: bool,
        mut on_progress: F,
    ) -> Result<Option<String>, String>
    where
        F: FnMut(u64, u64, u64) -> Result<(), std::io::Error>,
    {
        use std::io::{Read, Write};
        use sha2::{Digest, Sha256};

        let sess = Self::connect(params)
            .map_err(|e| format!("SFTP connect error: {}", e))?;
        let sftp = sess.sftp().map_err(|e| format!("SFTP session error: {}", e))?;
        let resolved = Self::resolve_remote_path(&sftp, &params.remote_path);
        let mut remote_file = sftp.open(Path::new(&resolved))
            .map_err(|e| format!("SFTP open error for {}: {}", resolved, e))?;

        let stat = sftp.stat(Path::new(&resolved)).ok();
        let total_file_bytes = stat.and_then(|s| s.size).unwrap_or(0);

        if let Some(parent) = local_dest.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let mut local_file = std::fs::File::create(local_dest)
            .map_err(|e| format!("Failed to create local file {}: {}", local_dest.display(), e))?;

        let mut buffer = vec![0u8; 128 * 1024]; // 128 KB buffer
        let mut bytes_copied = 0u64;
        let mut hasher = if verify { Some(Sha256::new()) } else { None };

        loop {
            let n = remote_file.read(&mut buffer)
                .map_err(|e| format!("SFTP read stream error: {}", e))?;
            if n == 0 {
                break;
            }
            local_file.write_all(&buffer[..n])
                .map_err(|e| format!("Local write error: {}", e))?;
            bytes_copied += n as u64;

            if let Some(ref mut h) = hasher {
                h.update(&buffer[..n]);
            }

            on_progress(n as u64, bytes_copied, total_file_bytes)
                .map_err(|e| format!("Transfer interrupted: {}", e))?;
        }

        local_file.flush().map_err(|e| format!("Local flush error: {}", e))?;

        let verified_hash = hasher.map(|h| hex::encode(h.finalize()));
        Ok(verified_hash)
    }

    pub fn upload_from_file(params: &SftpParams, local_src: &Path) -> Result<(), String> {
        Self::upload_from_file_streaming(params, local_src, false, |_, _, _| Ok(())).map(|_| ())
    }

    pub fn upload_from_file_streaming<F>(
        params: &SftpParams,
        local_src: &Path,
        verify: bool,
        mut on_progress: F,
    ) -> Result<Option<String>, String>
    where
        F: FnMut(u64, u64, u64) -> Result<(), std::io::Error>,
    {
        use std::io::{Read, Write};
        use sha2::{Digest, Sha256};

        let sess = Self::connect(params)
            .map_err(|e| format!("SFTP connect error: {}", e))?;
        let sftp = sess.sftp().map_err(|e| format!("SFTP session error: {}", e))?;
        let resolved = Self::resolve_remote_path(&sftp, &params.remote_path);
        let mut remote_file = sftp.create(Path::new(&resolved))
            .map_err(|e| format!("SFTP create remote file error for {}: {}", resolved, e))?;

        let mut local_file = std::fs::File::open(local_src)
            .map_err(|e| format!("Failed to open local source {}: {}", local_src.display(), e))?;
        let total_file_bytes = local_src.metadata().map(|m| m.len()).unwrap_or(0);

        let mut buffer = vec![0u8; 128 * 1024]; // 128 KB buffer
        let mut bytes_copied = 0u64;
        let mut hasher = if verify { Some(Sha256::new()) } else { None };

        loop {
            let n = local_file.read(&mut buffer)
                .map_err(|e| format!("Local read stream error: {}", e))?;
            if n == 0 {
                break;
            }
            remote_file.write_all(&buffer[..n])
                .map_err(|e| format!("SFTP write error: {}", e))?;
            bytes_copied += n as u64;

            if let Some(ref mut h) = hasher {
                h.update(&buffer[..n]);
            }

            on_progress(n as u64, bytes_copied, total_file_bytes)
                .map_err(|e| format!("Transfer interrupted: {}", e))?;
        }

        remote_file.flush().map_err(|e| format!("SFTP flush error: {}", e))?;

        let verified_hash = hasher.map(|h| hex::encode(h.finalize()));
        Ok(verified_hash)
    }

    pub fn mkdir(params: &SftpParams) -> Result<(), String> {
        let sess = Self::connect(params)
            .map_err(|e| format!("SFTP connect error: {}", e))?;
        let sftp = sess.sftp().map_err(|e| format!("SFTP session error: {}", e))?;
        let resolved = Self::resolve_remote_path(&sftp, &params.remote_path);
        sftp.mkdir(Path::new(&resolved), 0o755)
            .map_err(|e| format!("SFTP mkdir error: {}", e))?;
        Ok(())
    }

    pub fn rename(params: &SftpParams, new_remote_path: &str) -> Result<(), String> {
        let sess = Self::connect(params)
            .map_err(|e| format!("SFTP connect error: {}", e))?;
        let sftp = sess.sftp().map_err(|e| format!("SFTP session error: {}", e))?;
        let resolved_from = Self::resolve_remote_path(&sftp, &params.remote_path);
        let resolved_to = Self::resolve_remote_path(&sftp, new_remote_path);
        sftp.rename(Path::new(&resolved_from), Path::new(&resolved_to), None)
            .map_err(|e| format!("SFTP rename error: {}", e))?;
        Ok(())
    }

    pub fn delete(params: &SftpParams, is_dir: bool) -> Result<(), String> {
        let sess = Self::connect(params)
            .map_err(|e| format!("SFTP connect error: {}", e))?;
        let sftp = sess.sftp().map_err(|e| format!("SFTP session error: {}", e))?;
        let resolved = Self::resolve_remote_path(&sftp, &params.remote_path);
        if is_dir {
            sftp.rmdir(Path::new(&resolved))
                .map_err(|e| format!("SFTP rmdir error: {}", e))?;
        } else {
            sftp.unlink(Path::new(&resolved))
                .map_err(|e| format!("SFTP unlink error: {}", e))?;
        }
        Ok(())
    }
}

fn percent_decode(s: &str) -> String {
    let mut bytes = Vec::new();
    let mut chars = s.bytes();
    while let Some(b) = chars.next() {
        if b == b'%' {
            let h1 = chars.next();
            let h2 = chars.next();
            if let (Some(h1), Some(h2)) = (h1, h2) {
                if let (Some(d1), Some(d2)) = (hex_val(h1), hex_val(h2)) {
                    bytes.push((d1 << 4) | d2);
                    continue;
                }
            }
        } else if b == b'+' {
            bytes.push(b' ');
            continue;
        }
        bytes.push(b);
    }
    String::from_utf8_lossy(&bytes).to_string()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b'~' {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sftp_parse_uri_variations() {
        let p1 = SftpClient::parse_uri("sftp://user:secret@192.168.1.100:2222/var/log", None, None).unwrap();
        assert_eq!(p1.host, "192.168.1.100");
        assert_eq!(p1.port, 2222);
        assert_eq!(p1.user, "user");
        assert_eq!(p1.password.as_deref(), Some("secret"));
        assert_eq!(p1.remote_path, "/var/log");

        let p2 = SftpClient::parse_uri("sftp://admin@myserver.com/home/admin", None, None).unwrap();
        assert_eq!(p2.host, "myserver.com");
        assert_eq!(p2.port, 22);
        assert_eq!(p2.user, "admin");
        assert_eq!(p2.password, None);
        assert_eq!(p2.remote_path, "/home/admin");

        let p3 = SftpClient::parse_uri("sftp://user%40boop:p%40ss%3Aword@nas.local/data", None, None).unwrap();
        assert_eq!(p3.user, "user@boop");
        assert_eq!(p3.password.as_deref(), Some("p@ss:word"));
        assert_eq!(p3.host, "nas.local");
        assert_eq!(p3.remote_path, "/data");

        let p4 = SftpClient::parse_uri("ssh://root:secret@10.0.0.1:2222/etc", None, None).unwrap();
        assert_eq!(p4.host, "10.0.0.1");
        assert_eq!(p4.port, 2222);
        assert_eq!(p4.user, "root");
        assert_eq!(p4.password.as_deref(), Some("secret"));
        assert_eq!(p4.remote_path, "/etc");
    }

    #[test]
    fn test_sftp_supported_methods() {
        let sess = Session::new().unwrap();
        let kex_pref = "curve25519-sha256,curve25519-sha256@libssh.org,ecdh-sha2-nistp256,ecdh-sha2-nistp384,ecdh-sha2-nistp521,diffie-hellman-group-exchange-sha256,diffie-hellman-group16-sha512,diffie-hellman-group18-sha512,diffie-hellman-group14-sha256,diffie-hellman-group14-sha1,diffie-hellman-group-exchange-sha1,diffie-hellman-group1-sha1";
        let hostkey_pref = "ssh-ed25519,ecdsa-sha2-nistp256,ecdsa-sha2-nistp384,ecdsa-sha2-nistp521,rsa-sha2-512,rsa-sha2-256,ssh-rsa,ssh-dss";
        let crypt_pref = "aes256-ctr,aes192-ctr,aes128-ctr,aes256-cbc,aes192-cbc,aes128-cbc,3des-cbc";
        let mac_pref = "hmac-sha2-256,hmac-sha2-512,hmac-sha1,hmac-sha1-96";

        let r_kex = sess.method_pref(ssh2::MethodType::Kex, kex_pref);
        let r_hk = sess.method_pref(ssh2::MethodType::HostKey, hostkey_pref);
        let r_ccs = sess.method_pref(ssh2::MethodType::CryptCs, crypt_pref);
        let r_csc = sess.method_pref(ssh2::MethodType::CryptSc, crypt_pref);
        let r_mcs = sess.method_pref(ssh2::MethodType::MacCs, mac_pref);
        let r_msc = sess.method_pref(ssh2::MethodType::MacSc, mac_pref);

        println!("r_kex: {:?}", r_kex);
        println!("r_hk: {:?}", r_hk);
        println!("r_ccs: {:?}", r_ccs);
        println!("r_csc: {:?}", r_csc);
        println!("r_mcs: {:?}", r_mcs);
        println!("r_msc: {:?}", r_msc);

        assert!(r_kex.is_ok());
        assert!(r_hk.is_ok());
        assert!(r_ccs.is_ok());
        assert!(r_csc.is_ok());
        assert!(r_mcs.is_ok());
        assert!(r_msc.is_ok());
    }
}
