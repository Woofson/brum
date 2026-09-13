pub mod archive;
pub mod checksum;
pub mod local;
pub mod nfs;
pub mod proton;
pub mod s3;
pub mod sftp;
pub mod smb;
pub mod transfer;
pub mod vault;
pub mod webdav;

use serde::{Deserialize, Serialize};

pub type VfsResult<T> = Result<T, String>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub is_empty: Option<bool>,
    pub size: u64,
    pub modified: Option<u64>, // Unix timestamp in seconds
    pub permissions: String,   // "rwxr-xr-x"
    pub mode_octal: String,    // "0755"
    pub owner: String,         // "bolt"
    pub group: String,         // "bolt"
    pub uid: u32,
    pub gid: u32,
    pub mime_type: Option<String>,
    pub is_archive: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DirectoryListing {
    pub current_path: String,
    pub parent_path: Option<String>,
    pub entries: Vec<FileEntry>,
    pub total_files: usize,
    pub total_dirs: usize,
    pub total_size: u64,
    pub protocol: String, // "local", "sftp", "webdav", "archive", "s3", "branch"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_truncated: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileContentResponse {
    pub path: String,
    pub name: String,
    pub content: String,
    pub is_binary: bool,
    pub size: u64,
    pub mime_type: String,
}

pub fn is_archive_file(path_or_name: &str) -> bool {
    let lower = path_or_name.to_lowercase();
    lower.ends_with(".zip")
        || lower.ends_with(".grr")
        || lower.ends_with(".cbz")
        || lower.ends_with(".epub")
        || lower.ends_with(".tar.gz")
        || lower.ends_with(".tgz")
        || lower.ends_with(".tar.bz2")
        || lower.ends_with(".tbz2")
        || lower.ends_with(".tar.xz")
        || lower.ends_with(".txz")
        || lower.ends_with(".tar")
        || lower.ends_with(".7z")
        || lower.ends_with(".rar")
}

pub fn sanitize_uri(uri: &str) -> String {
    if !uri.contains("://") || !uri.contains('@') {
        return uri.to_string();
    }
    if let Some((scheme_userinfo, host_path)) = uri.split_once('@') {
        if let Some((scheme, userinfo)) = scheme_userinfo.split_once("://") {
            if let Some((user, _pass)) = userinfo.split_once(':') {
                return format!("{}://{}@{}", scheme, user, host_path);
            }
        }
    }
    uri.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_uri() {
        assert_eq!(
            sanitize_uri("sftp://bolt:mypassword@192.168.1.100:22/home/bolt"),
            "sftp://bolt@192.168.1.100:22/home/bolt"
        );
        assert_eq!(
            sanitize_uri("smb://WORKGROUP;admin:Secret123@nas.local:445/data"),
            "smb://WORKGROUP;admin@nas.local:445/data"
        );
        assert_eq!(
            sanitize_uri("sftp://user@remote.host/dir"),
            "sftp://user@remote.host/dir"
        );
        assert_eq!(
            sanitize_uri("/local/path/to/file.txt"),
            "/local/path/to/file.txt"
        );
    }
}
