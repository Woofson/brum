use super::{DirectoryListing, FileContentResponse, FileEntry};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::Path;
use tar::Archive as TarArchive;
use tar::Builder as TarBuilder;
use tracing::info;
use zip::{ZipArchive, ZipWriter};

#[derive(Debug, Clone)]
pub struct IsoRecord {
    pub name: String,
    pub extent_lba: u32,
    pub data_length: u32,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub symlink_target: Option<String>,
    pub modified_unix: Option<u64>,
    pub mode_octal: Option<String>,
    pub uid: u32,
    pub gid: u32,
}

#[derive(Debug, Clone)]
pub struct IsoVolumeInfo {
    pub root_lba: u32,
    pub root_len: u32,
    pub is_joliet: bool,
    pub has_rock_ridge: bool,
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
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
        }
        bytes.push(b);
    }
    String::from_utf8_lossy(&bytes).to_string()
}

pub struct ArchiveHandler;

impl ArchiveHandler {
    pub fn normalize_archive_path(raw: &str) -> (std::path::PathBuf, String) {
        let mut s = raw.to_string();
        while s.starts_with("archive://") {
            s = s.strip_prefix("archive://").unwrap_or(&s).to_string();
        }
        if let Some((base, _)) = s.split_once('#') {
            s = base.to_string();
        }
        if s.contains('%') {
            s = percent_decode(&s);
        }
        if (s.starts_with('/') || s.starts_with('\\'))
            && s.len() >= 3
            && s.as_bytes()[1].is_ascii_alphabetic()
            && s.as_bytes()[2] == b':'
        {
            s = s[1..].to_string();
        }
        if s.len() > 1 && (s.ends_with('/') || s.ends_with('\\')) && !s.ends_with(":/") && !s.ends_with(r":\") {
            s = s.trim_end_matches(&['/', '\\'][..]).to_string();
        }
        (std::path::PathBuf::from(&s), s)
    }

    pub fn list_archive_contents(
        archive_path_str: &str,
        subpath_filter: &str,
    ) -> Result<DirectoryListing, std::io::Error> {
        let (path_buf, normalized_str) = Self::normalize_archive_path(archive_path_str);
        let path = path_buf.as_path();
        let archive_path_str = normalized_str.as_str();
        if !path.exists() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Archive not found: {}", archive_path_str),
            ));
        }

        let lower_name = archive_path_str.to_lowercase();
        let mut entries = Vec::new();
        let mut total_files = 0;
        let mut total_dirs = 0;
        let mut total_size = 0u64;

        let clean_subpath = subpath_filter.trim_matches('/');

        if lower_name.ends_with(".zip") || lower_name.ends_with(".grr") || lower_name.ends_with(".cbz") || lower_name.ends_with(".epub") {
            let file = File::open(path)?;
            let mut zip = ZipArchive::new(BufReader::new(file))
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

            for i in 0..zip.len() {
                let file = zip.by_index(i)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
                let raw_name = file.name().to_string();
                let is_dir = file.is_dir() || raw_name.ends_with('/');
                let trimmed = raw_name.trim_matches('/');

                if !clean_subpath.is_empty() {
                    if !trimmed.starts_with(clean_subpath) {
                        continue;
                    }
                }

                let display_name = if clean_subpath.is_empty() {
                    trimmed.split('/').next().unwrap_or("").to_string()
                } else {
                    let suffix = trimmed.strip_prefix(clean_subpath).unwrap_or("").trim_matches('/');
                    suffix.split('/').next().unwrap_or("").to_string()
                };

                if display_name.is_empty() || entries.iter().any(|e: &FileEntry| e.name == display_name) {
                    continue;
                }

                let size = file.size();
                if is_dir {
                    total_dirs += 1;
                } else {
                    total_files += 1;
                    total_size += size;
                }

                entries.push(FileEntry {
                    name: display_name.clone(),
                    path: format!("archive://{}#{}", archive_path_str, trimmed),
                    is_dir,
                    is_symlink: false,
                    is_empty: None,
                    size,
                    modified: None,
                    permissions: if is_dir { "drwxr-xr-x".to_string() } else { "-rw-r--r--".to_string() },
                    mode_octal: if is_dir { "0755".to_string() } else { "0644".to_string() },
                    owner: "archive".to_string(),
                    group: "archive".to_string(),
                    uid: 1000,
                    gid: 1000,
                    mime_type: if is_dir { None } else { Some(mime_guess::from_path(&display_name).first_or_octet_stream().to_string()) },
                    is_archive: false,
                });
            }
        } else if lower_name.ends_with(".tar.gz") || lower_name.ends_with(".tgz") || lower_name.ends_with(".tar.bz2") || lower_name.ends_with(".tbz2") || lower_name.ends_with(".tar") {
            let file = File::open(path)?;
            let reader: Box<dyn Read> = if lower_name.ends_with(".tar.gz") || lower_name.ends_with(".tgz") {
                Box::new(GzDecoder::new(file))
            } else if lower_name.ends_with(".tar.bz2") || lower_name.ends_with(".tbz2") {
                Box::new(bzip2::read::BzDecoder::new(file))
            } else {
                Box::new(file)
            };

            let mut tar = TarArchive::new(reader);
            if let Ok(entries_iter) = tar.entries() {
                for entry_res in entries_iter {
                    if let Ok(entry) = entry_res {
                        if let Ok(path_buf) = entry.path() {
                            let raw_name = path_buf.to_string_lossy().to_string();
                            let is_dir = entry.header().entry_type().is_dir();
                            let trimmed = raw_name.trim_matches('/');

                            if !clean_subpath.is_empty() && !trimmed.starts_with(clean_subpath) {
                                continue;
                            }

                            let display_name = if clean_subpath.is_empty() {
                                trimmed.split('/').next().unwrap_or("").to_string()
                            } else {
                                let suffix = trimmed.strip_prefix(clean_subpath).unwrap_or("").trim_matches('/');
                                suffix.split('/').next().unwrap_or("").to_string()
                            };

                            if display_name.is_empty() || entries.iter().any(|e: &FileEntry| e.name == display_name) {
                                continue;
                            }

                            let size = entry.header().size().unwrap_or(0);
                            if is_dir {
                                total_dirs += 1;
                            } else {
                                total_files += 1;
                                total_size += size;
                            }

                            entries.push(FileEntry {
                                name: display_name.clone(),
                                path: format!("archive://{}#{}", archive_path_str, trimmed),
                                is_dir,
                                is_symlink: false,
                                is_empty: None,
                                size,
                                modified: None,
                                permissions: if is_dir { "drwxr-xr-x".to_string() } else { "-rw-r--r--".to_string() },
                                mode_octal: if is_dir { "0755".to_string() } else { "0644".to_string() },
                                owner: "archive".to_string(),
                                group: "archive".to_string(),
                                uid: 1000,
                                gid: 1000,
                                mime_type: if is_dir { None } else { Some(mime_guess::from_path(&display_name).first_or_octet_stream().to_string()) },
                                is_archive: false,
                            });
                        }
                    }
                }
            }
        } else if super::disk_image::DiskImageHandler::is_supported_image(archive_path_str) || path.is_dir() {
            match super::disk_image::DiskImageHandler::list_image_contents(archive_path_str, clean_subpath) {
                Ok(listing) => return Ok(listing),
                Err(err) => {
                    tracing::debug!("DiskImageHandler fallback to ISO reader for {}: {}", archive_path_str, err);
                    return Self::list_iso_contents(archive_path_str, clean_subpath);
                }
            }
        } else if lower_name.ends_with(".iso") || lower_name.ends_with(".udf") {
            return Self::list_iso_contents(archive_path_str, clean_subpath);
        } else if lower_name.ends_with(".squashfs") || lower_name.ends_with(".snap") || lower_name.ends_with(".appimage") {
            return Self::list_squashfs_contents(archive_path_str, clean_subpath);
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

        Ok(DirectoryListing {
            current_path: format!("archive://{}#{}", archive_path_str, clean_subpath),
            parent_path: if !clean_subpath.is_empty() {
                Some(format!("archive://{}#", archive_path_str))
            } else {
                Some(Path::new(archive_path_str).parent().unwrap_or(Path::new("/")).to_string_lossy().to_string())
            },
            entries,
            total_files,
            total_dirs,
            total_size,
            protocol: "archive".to_string(),
            is_truncated: None,
            max_limit: None,
        })
    }

    pub(crate) fn list_iso_contents(archive_path_str: &str, clean_subpath: &str) -> Result<DirectoryListing, std::io::Error> {
        // 1. First attempt to parse as UDF (e.g. Windows installer ISOs, DVD images, UDF bridges)
        if let Ok(listing) = Self::list_udf_contents(archive_path_str, clean_subpath) {
            if !listing.entries.is_empty() || !clean_subpath.is_empty() {
                return Ok(listing);
            }
        }

        // 2. Fall back to ISO 9660 (with Joliet SVD and Rock Ridge SUSP support)
        Self::list_iso9660_contents(archive_path_str, clean_subpath)
    }

    fn list_udf_contents(archive_path_str: &str, clean_subpath: &str) -> Result<DirectoryListing, std::io::Error> {
        let path = Path::new(archive_path_str);
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        let volume = hadris_udf::UdfVolume::open(reader)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("UDF open error: {:?}", e)))?;

        let root_dir = volume.root_dir()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("UDF root dir error: {:?}", e)))?;

        let mut current_dir = root_dir;
        if !clean_subpath.is_empty() {
            let parts: Vec<&str> = clean_subpath.split('/').filter(|p| !p.is_empty()).collect();
            for part in parts {
                let entry = current_dir.entries()
                    .find(|e| e.name().eq_ignore_ascii_case(part))
                    .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, format!("Directory not found: {}", part)))?
                    .clone();
                if !entry.is_dir() {
                    return Err(std::io::Error::new(std::io::ErrorKind::Other, format!("Path component is not a directory: {}", part)));
                }
                current_dir = volume.read_directory(&entry.icb)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("UDF read_directory error: {:?}", e)))?;
            }
        }

        let mut entries = Vec::new();
        let mut total_files = 0;
        let mut total_dirs = 0;
        let mut total_size = 0u64;

        for entry in current_dir.entries() {
            let name = entry.name().to_string();
            if name.is_empty() || name == "." || name == ".." {
                continue;
            }
            let is_dir = entry.is_dir();
            let size = entry.size;
            if is_dir {
                total_dirs += 1;
            } else {
                total_files += 1;
                total_size += size;
            }

            let full_inner = if clean_subpath.is_empty() {
                name.clone()
            } else {
                format!("{}/{}", clean_subpath, name)
            };

            entries.push(FileEntry {
                name: name.clone(),
                path: format!("archive://{}#{}", archive_path_str, full_inner),
                is_dir,
                is_symlink: false,
                is_empty: None,
                size,
                modified: None,
                permissions: if is_dir { "drwxr-xr-x".to_string() } else { "-rw-r--r--".to_string() },
                mode_octal: if is_dir { "0755".to_string() } else { "0644".to_string() },
                owner: "udf".to_string(),
                group: "udf".to_string(),
                uid: 0,
                gid: 0,
                mime_type: if is_dir { None } else { Some(mime_guess::from_path(&name).first_or_octet_stream().to_string()) },
                is_archive: false,
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

        Ok(DirectoryListing {
            current_path: format!("archive://{}#{}", archive_path_str, clean_subpath),
            parent_path: if !clean_subpath.is_empty() {
                let parent_sub = Path::new(clean_subpath).parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
                Some(format!("archive://{}#{}", archive_path_str, parent_sub.trim_matches('/')))
            } else {
                Some(Path::new(archive_path_str).parent().unwrap_or(Path::new("/")).to_string_lossy().to_string())
            },
            entries,
            total_files,
            total_dirs,
            total_size,
            protocol: "archive".to_string(),
            is_truncated: None,
            max_limit: None,
        })
    }

    fn list_iso9660_contents(archive_path_str: &str, clean_subpath: &str) -> Result<DirectoryListing, std::io::Error> {
        let path = Path::new(archive_path_str);
        let mut file = File::open(path)?;
        let vol = Self::read_iso_volume_info(&mut file)?;

        let target_dir_record = if clean_subpath.is_empty() {
            IsoRecord {
                name: "/".to_string(),
                extent_lba: vol.root_lba,
                data_length: vol.root_len,
                is_dir: true,
                is_symlink: false,
                symlink_target: None,
                modified_unix: None,
                mode_octal: None,
                uid: 0,
                gid: 0,
            }
        } else {
            let parts: Vec<&str> = clean_subpath.split('/').filter(|p| !p.is_empty()).collect();
            Self::find_iso_record(&mut file, &vol, &parts)?
        };

        if !target_dir_record.is_dir {
            return Err(std::io::Error::new(std::io::ErrorKind::Other, "Target subpath is not a directory"));
        }

        let records = Self::read_directory_records(&mut file, target_dir_record.extent_lba, target_dir_record.data_length, vol.is_joliet)?;

        let mut entries = Vec::new();
        let mut total_files = 0;
        let mut total_dirs = 0;
        let mut total_size = 0u64;

        for item in records {
            let raw_name = item.name.trim();
            if raw_name.is_empty() || raw_name == "." || raw_name == ".." || raw_name == "\0" || raw_name == "\u{1}" {
                continue;
            }

            let display_name = raw_name.to_string();
            if display_name.is_empty() || entries.iter().any(|e: &FileEntry| e.name.eq_ignore_ascii_case(&display_name)) {
                continue;
            }

            let is_dir = item.is_dir;
            let size = item.data_length as u64;

            if is_dir {
                total_dirs += 1;
            } else {
                total_files += 1;
                total_size += size;
            }

            let full_inner_path = if clean_subpath.is_empty() {
                display_name.clone()
            } else {
                format!("{}/{}", clean_subpath, display_name)
            };

            let permissions = if is_dir {
                "drwxr-xr-x".to_string()
            } else if item.is_symlink {
                "lrwxrwxrwx".to_string()
            } else {
                "-rw-r--r--".to_string()
            };

            entries.push(FileEntry {
                name: display_name.clone(),
                path: format!("archive://{}#{}", archive_path_str, full_inner_path),
                is_dir,
                is_symlink: item.is_symlink,
                is_empty: None,
                size,
                modified: item.modified_unix,
                permissions,
                mode_octal: item.mode_octal.unwrap_or_else(|| if is_dir { "0755".to_string() } else { "0644".to_string() }),
                owner: if item.uid == 0 { "root".to_string() } else { item.uid.to_string() },
                group: if item.gid == 0 { "root".to_string() } else { item.gid.to_string() },
                uid: item.uid,
                gid: item.gid,
                mime_type: if is_dir { None } else { Some(mime_guess::from_path(&display_name).first_or_octet_stream().to_string()) },
                is_archive: false,
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

        Ok(DirectoryListing {
            current_path: format!("archive://{}#{}", archive_path_str, clean_subpath),
            parent_path: if !clean_subpath.is_empty() {
                let parent_sub = Path::new(clean_subpath).parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
                Some(format!("archive://{}#{}", archive_path_str, parent_sub.trim_matches('/')))
            } else {
                Some(Path::new(archive_path_str).parent().unwrap_or(Path::new("/")).to_string_lossy().to_string())
            },
            entries,
            total_files,
            total_dirs,
            total_size,
            protocol: "archive".to_string(),
            is_truncated: None,
            max_limit: None,
        })
    }

    pub(crate) fn list_squashfs_contents(archive_path_str: &str, clean_subpath: &str) -> Result<DirectoryListing, std::io::Error> {
        let path = Path::new(archive_path_str);
        let file = BufReader::new(File::open(path)?);
        let filesystem = backhand::FilesystemReader::from_reader(file)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("SquashFS read error: {}", e)))?;

        let mut entries = Vec::new();
        let mut total_files = 0;
        let mut total_dirs = 0;
        let mut total_size = 0u64;

        for node in filesystem.files() {
            let node_path = node.fullpath.to_string_lossy();
            let trimmed = node_path.trim_matches('/');
            if trimmed.is_empty() {
                continue;
            }

            if !clean_subpath.is_empty() {
                if !trimmed.starts_with(clean_subpath) {
                    continue;
                }
                if trimmed == clean_subpath {
                    continue;
                }
            }

            let rel_path = if clean_subpath.is_empty() {
                trimmed
            } else {
                trimmed.strip_prefix(clean_subpath).unwrap_or("").trim_matches('/')
            };

            if rel_path.is_empty() {
                continue;
            }

            let first_seg = rel_path.split('/').next().unwrap_or("");
            if first_seg.is_empty() {
                continue;
            }

            let is_direct_child = !rel_path.contains('/');
            let full_child_path = if clean_subpath.is_empty() {
                first_seg.to_string()
            } else {
                format!("{}/{}", clean_subpath, first_seg)
            };

            let is_dir = !is_direct_child || matches!(node.inner, backhand::InnerNode::Dir(_));
            let size = match &node.inner {
                backhand::InnerNode::File(f) if is_direct_child => f.file_len() as u64,
                _ => 0,
            };

            if entries.iter().any(|e: &FileEntry| e.name == first_seg) {
                continue;
            }

            if is_dir {
                total_dirs += 1;
            } else {
                total_files += 1;
                total_size += size;
            }

            let octal_mode = format!("{:04o}", node.header.permissions & 0o7777);
            let mode_str = if is_dir { "drwxr-xr-x".to_string() } else { "-rw-r--r--".to_string() };

            entries.push(FileEntry {
                name: first_seg.to_string(),
                path: format!("archive://{}#{}", archive_path_str, full_child_path),
                is_dir,
                is_symlink: matches!(node.inner, backhand::InnerNode::Symlink(_)),
                is_empty: None,
                size,
                modified: Some(node.header.mtime as u64),
                permissions: mode_str,
                mode_octal: octal_mode,
                owner: node.header.uid.to_string(),
                group: node.header.gid.to_string(),
                uid: node.header.uid,
                gid: node.header.gid,
                mime_type: if is_dir { None } else { Some(mime_guess::from_path(first_seg).first_or_octet_stream().to_string()) },
                is_archive: false,
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

        Ok(DirectoryListing {
            current_path: format!("archive://{}#{}", archive_path_str, clean_subpath),
            parent_path: if !clean_subpath.is_empty() {
                let parent_sub = Path::new(clean_subpath).parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
                Some(format!("archive://{}#{}", archive_path_str, parent_sub.trim_matches('/')))
            } else {
                Some(Path::new(archive_path_str).parent().unwrap_or(Path::new("/")).to_string_lossy().to_string())
            },
            entries,
            total_files,
            total_dirs,
            total_size,
            protocol: "archive".to_string(),
            is_truncated: None,
            max_limit: None,
        })
    }

    pub fn read_archive_entry(
        archive_path_str: &str,
        inner_path: &str,
        max_bytes: usize,
    ) -> Result<FileContentResponse, std::io::Error> {
        let (path_buf, normalized_str) = Self::normalize_archive_path(archive_path_str);
        let path = path_buf.as_path();
        let archive_path_str = normalized_str.as_str();
        let lower = archive_path_str.to_lowercase();
        let clean_inner = inner_path.trim_matches('/');

        if super::disk_image::DiskImageHandler::is_supported_image(archive_path_str) || path.is_dir() {
            let limit_opt = if max_bytes > 0 { Some(max_bytes) } else { None };
            match super::disk_image::DiskImageHandler::read_image_entry(archive_path_str, clean_inner, limit_opt) {
                Ok(resp) => return Ok(resp),
                Err(err) => {
                    tracing::debug!("DiskImageHandler fallback to ISO reader for {}: {}", clean_inner, err);
                    return Self::read_iso_entry(archive_path_str, clean_inner, max_bytes);
                }
            }
        } else if lower.ends_with(".iso") || lower.ends_with(".udf") {
            return Self::read_iso_entry(archive_path_str, clean_inner, max_bytes);
        } else if lower.ends_with(".squashfs") || lower.ends_with(".snap") || lower.ends_with(".appimage") {
            return Self::read_squashfs_entry(archive_path_str, clean_inner, max_bytes);
        }

        if lower.ends_with(".zip") || lower.ends_with(".grr") || lower.ends_with(".cbz") || lower.ends_with(".epub") {
            let file = File::open(path)?;
            let mut zip = ZipArchive::new(BufReader::new(file))
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

            for i in 0..zip.len() {
                let mut f = zip.by_index(i)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
                if f.name().trim_matches('/') == clean_inner {
                    let mut buffer = Vec::new();
                    let size = f.size();
                    if max_bytes > 0 {
                        let mut handle = (&mut f).take(max_bytes as u64);
                        handle.read_to_end(&mut buffer)?;
                    } else {
                        f.read_to_end(&mut buffer)?;
                    }

                    let mime_type = mime_guess::from_path(clean_inner).first_or_octet_stream().to_string();
                    let name = Path::new(clean_inner).file_name().unwrap_or_default().to_string_lossy().to_string();

                    return match String::from_utf8(buffer.clone()) {
                        Ok(text) => Ok(FileContentResponse {
                            path: format!("archive://{}#{}", archive_path_str, clean_inner),
                            name,
                            content: text,
                            is_binary: false,
                            size,
                            mime_type,
                        }),
                        Err(_) => {
                            use base64::Engine;
                            let b64 = base64::engine::general_purpose::STANDARD.encode(&buffer);
                            Ok(FileContentResponse {
                                path: format!("archive://{}#{}", archive_path_str, clean_inner),
                                name,
                                content: b64,
                                is_binary: true,
                                size,
                                mime_type,
                            })
                        }
                    };
                }
            }
        } else if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") || lower.ends_with(".tar.bz2") || lower.ends_with(".tbz2") || lower.ends_with(".tar") {
            let file = File::open(path)?;
            let reader: Box<dyn Read> = if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
                Box::new(GzDecoder::new(file))
            } else if lower.ends_with(".tar.bz2") || lower.ends_with(".tbz2") {
                Box::new(bzip2::read::BzDecoder::new(file))
            } else {
                Box::new(file)
            };

            let mut tar = TarArchive::new(reader);
            if let Ok(entries_iter) = tar.entries() {
                for entry_res in entries_iter {
                    if let Ok(mut entry) = entry_res {
                        if let Ok(path_buf) = entry.path() {
                            let raw_name = path_buf.to_string_lossy().to_string();
                            if raw_name.trim_matches('/') == clean_inner {
                                let mut buffer = Vec::new();
                                let size = entry.header().size().unwrap_or(0);
                                if max_bytes > 0 {
                                    let mut handle = (&mut entry).take(max_bytes as u64);
                                    handle.read_to_end(&mut buffer)?;
                                } else {
                                    entry.read_to_end(&mut buffer)?;
                                }

                                let mime_type = mime_guess::from_path(clean_inner).first_or_octet_stream().to_string();
                                let name = Path::new(clean_inner).file_name().unwrap_or_default().to_string_lossy().to_string();

                                return match String::from_utf8(buffer.clone()) {
                                    Ok(text) => Ok(FileContentResponse {
                                        path: format!("archive://{}#{}", archive_path_str, clean_inner),
                                        name,
                                        content: text,
                                        is_binary: false,
                                        size,
                                        mime_type,
                                    }),
                                    Err(_) => {
                                        use base64::Engine;
                                        let b64 = base64::engine::general_purpose::STANDARD.encode(&buffer);
                                        Ok(FileContentResponse {
                                            path: format!("archive://{}#{}", archive_path_str, clean_inner),
                                            name,
                                            content: b64,
                                            is_binary: true,
                                            size,
                                            mime_type,
                                        })
                                    }
                                };
                            }
                        }
                    }
                }
            }
        }

        Err(std::io::Error::new(std::io::ErrorKind::NotFound, "Entry not found in archive"))
    }

    pub(crate) fn read_iso_entry(archive_path_str: &str, clean_inner: &str, max_bytes: usize) -> Result<FileContentResponse, std::io::Error> {
        // 1. Try reading via UDF
        if let Ok(res) = Self::read_udf_entry(archive_path_str, clean_inner, max_bytes) {
            return Ok(res);
        }

        // 2. Fall back to ISO 9660 (with Joliet and Rock Ridge support)
        Self::read_iso9660_entry(archive_path_str, clean_inner, max_bytes)
    }

    fn read_udf_entry(archive_path_str: &str, clean_inner: &str, max_bytes: usize) -> Result<FileContentResponse, std::io::Error> {
        let path = Path::new(archive_path_str);
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        let volume = hadris_udf::UdfVolume::open(reader)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("UDF open error: {:?}", e)))?;

        let root_dir = volume.root_dir()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("UDF root dir error: {:?}", e)))?;

        let mut current_dir = root_dir;
        let parts: Vec<&str> = clean_inner.split('/').filter(|p| !p.is_empty()).collect();
        if parts.is_empty() {
            return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "Empty path"));
        }

        for (idx, part) in parts.iter().enumerate() {
            let is_last = idx == parts.len() - 1;
            let entry = current_dir.entries()
                .find(|e| e.name().eq_ignore_ascii_case(part))
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, format!("Entry not found: {}", part)))?
                .clone();

            if is_last {
                if entry.is_dir() {
                    return Err(std::io::Error::new(std::io::ErrorKind::Other, "Target is a directory, not a file"));
                }
                let mut data = volume.read_file(&entry)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("UDF read_file error: {:?}", e)))?;
                let size = entry.size;
                if max_bytes > 0 && data.len() > max_bytes {
                    data.truncate(max_bytes);
                }

                let mime_type = mime_guess::from_path(clean_inner).first_or_octet_stream().to_string();
                let name = Path::new(clean_inner).file_name().unwrap_or_default().to_string_lossy().to_string();

                return match String::from_utf8(data.clone()) {
                    Ok(text) => Ok(FileContentResponse {
                        path: format!("archive://{}#{}", archive_path_str, clean_inner),
                        name,
                        content: text,
                        is_binary: false,
                        size,
                        mime_type,
                    }),
                    Err(_) => {
                        use base64::Engine;
                        let b64 = base64::engine::general_purpose::STANDARD.encode(&data);
                        Ok(FileContentResponse {
                            path: format!("archive://{}#{}", archive_path_str, clean_inner),
                            name,
                            content: b64,
                            is_binary: true,
                            size,
                            mime_type,
                        })
                    }
                };
            } else {
                if !entry.is_dir() {
                    return Err(std::io::Error::new(std::io::ErrorKind::Other, "Path segment is not a directory"));
                }
                current_dir = volume.read_directory(&entry.icb)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("UDF read_directory error: {:?}", e)))?;
            }
        }

        Err(std::io::Error::new(std::io::ErrorKind::NotFound, "Entry not found in UDF"))
    }

    fn read_iso9660_entry(archive_path_str: &str, clean_inner: &str, max_bytes: usize) -> Result<FileContentResponse, std::io::Error> {
        let path = Path::new(archive_path_str);
        let mut file = File::open(path)?;
        let vol = Self::read_iso_volume_info(&mut file)?;

        let parts: Vec<&str> = clean_inner.split('/').filter(|p| !p.is_empty()).collect();
        let record = Self::find_iso_record(&mut file, &vol, &parts)?;

        if record.is_dir {
            return Err(std::io::Error::new(std::io::ErrorKind::Other, "Target is a directory, not a file"));
        }

        let buffer = Self::read_iso_record_bytes(&mut file, &record, max_bytes)?;
        let mime_type = mime_guess::from_path(clean_inner).first_or_octet_stream().to_string();
        let name = Path::new(clean_inner).file_name().unwrap_or_default().to_string_lossy().to_string();

        match String::from_utf8(buffer.clone()) {
            Ok(text) => Ok(FileContentResponse {
                path: format!("archive://{}#{}", archive_path_str, clean_inner),
                name,
                content: text,
                is_binary: false,
                size: record.data_length as u64,
                mime_type,
            }),
            Err(_) => {
                use base64::Engine;
                let b64 = base64::engine::general_purpose::STANDARD.encode(&buffer);
                Ok(FileContentResponse {
                    path: format!("archive://{}#{}", archive_path_str, clean_inner),
                    name,
                    content: b64,
                    is_binary: true,
                    size: record.data_length as u64,
                    mime_type,
                })
            }
        }
    }

    pub(crate) fn read_squashfs_entry(archive_path_str: &str, clean_inner: &str, max_bytes: usize) -> Result<FileContentResponse, std::io::Error> {
        let path = Path::new(archive_path_str);
        let file = BufReader::new(File::open(path)?);
        let filesystem = backhand::FilesystemReader::from_reader(file)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("SquashFS read error: {}", e)))?;

        let target_path = format!("/{}", clean_inner);

        for node in filesystem.files() {
            let node_path_str = node.fullpath.to_string_lossy();
            if node_path_str == target_path || node_path_str.trim_matches('/') == clean_inner {
                match &node.inner {
                    backhand::InnerNode::File(sq_file) => {
                        let mut reader = filesystem.file(sq_file).reader();
                        let mut buffer = Vec::new();
                        let size = sq_file.file_len() as u64;
                        if max_bytes > 0 {
                            let mut handle = reader.take(max_bytes as u64);
                            handle.read_to_end(&mut buffer)?;
                        } else {
                            reader.read_to_end(&mut buffer)?;
                        }

                        let mime_type = mime_guess::from_path(clean_inner).first_or_octet_stream().to_string();
                        let name = Path::new(clean_inner).file_name().unwrap_or_default().to_string_lossy().to_string();

                        return match String::from_utf8(buffer.clone()) {
                            Ok(text) => Ok(FileContentResponse {
                                path: format!("archive://{}#{}", archive_path_str, clean_inner),
                                name,
                                content: text,
                                is_binary: false,
                                size,
                                mime_type,
                            }),
                            Err(_) => {
                                use base64::Engine;
                                let b64 = base64::engine::general_purpose::STANDARD.encode(&buffer);
                                Ok(FileContentResponse {
                                    path: format!("archive://{}#{}", archive_path_str, clean_inner),
                                    name,
                                    content: b64,
                                    is_binary: true,
                                    size,
                                    mime_type,
                                })
                            }
                        };
                    }
                    backhand::InnerNode::Symlink(symlink) => {
                        let target_str = symlink.link.to_string_lossy().to_string();
                        return Ok(FileContentResponse {
                            path: format!("archive://{}#{}", archive_path_str, clean_inner),
                            name: Path::new(clean_inner).file_name().unwrap_or_default().to_string_lossy().to_string(),
                            content: format!("Symlink -> {}", target_str),
                            is_binary: false,
                            size: target_str.len() as u64,
                            mime_type: "text/plain".to_string(),
                        });
                    }
                    _ => return Err(std::io::Error::new(std::io::ErrorKind::Other, "Target is not a regular file")),
                }
            }
        }

        Err(std::io::Error::new(std::io::ErrorKind::NotFound, "Entry not found in SquashFS image"))
    }

    fn should_strip_root_prefix(prefix: &str, target_name: &str) -> bool {
        if prefix.is_empty() || target_name.is_empty() {
            return false;
        }
        let t_norm = target_name.to_lowercase().replace(['-', '_', '.'], "");
        let p_norm = prefix.to_lowercase().replace(['-', '_', '.'], "");
        t_norm == p_norm || t_norm.starts_with(&p_norm) || p_norm.starts_with(&t_norm)
    }

    fn find_zip_strip_prefix<R: Read + std::io::Seek>(zip: &mut ZipArchive<R>, target_name: &str) -> Option<String> {
        if target_name.is_empty() {
            return None;
        }
        let mut common_prefix: Option<String> = None;
        let mut has_entries = false;

        for i in 0..zip.len() {
            let name = match zip.name_for_index(i) {
                Some(n) => n.replace('\\', "/"),
                None => continue,
            };
            let clean = name.trim_start_matches('/');
            if clean.is_empty() {
                continue;
            }
            has_entries = true;
            let parts: Vec<&str> = clean.split('/').collect();
            if parts.len() <= 1 && !clean.ends_with('/') {
                return None; // Loose file at root
            }
            let root = parts[0];
            if root.is_empty() {
                return None;
            }
            match &common_prefix {
                None => common_prefix = Some(root.to_string()),
                Some(p) => {
                    if p != root {
                        return None;
                    }
                }
            }
        }

        if !has_entries {
            return None;
        }

        if let Some(prefix) = common_prefix {
            if Self::should_strip_root_prefix(&prefix, target_name) {
                return Some(format!("{}/", prefix));
            }
        }

        None
    }

    fn find_tar_strip_prefix<R: Read>(reader: R, target_name: &str) -> Option<String> {
        if target_name.is_empty() {
            return None;
        }
        let mut archive = TarArchive::new(reader);
        let entries = archive.entries().ok()?;
        let mut common_prefix: Option<String> = None;
        let mut has_entries = false;

        for entry_res in entries {
            let entry = match entry_res {
                Ok(e) => e,
                Err(_) => return None,
            };
            let path = match entry.path() {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            let clean = path.trim_start_matches('/').to_string();
            if clean.is_empty() {
                continue;
            }
            has_entries = true;
            let parts: Vec<&str> = clean.split('/').collect();
            let is_dir = entry.header().entry_type().is_dir();
            if parts.len() <= 1 && !is_dir && !clean.ends_with('/') {
                return None; // Loose file at root
            }
            let root = parts[0];
            if root.is_empty() {
                return None;
            }
            match &common_prefix {
                None => common_prefix = Some(root.to_string()),
                Some(p) => {
                    if p != root {
                        return None;
                    }
                }
            }
        }

        if !has_entries {
            return None;
        }

        if let Some(prefix) = common_prefix {
            if Self::should_strip_root_prefix(&prefix, target_name) {
                return Some(format!("{}/", prefix));
            }
        }

        None
    }

    fn unpack_tar_with_strip<R: Read>(
        archive: &mut TarArchive<R>,
        target: &Path,
        strip_prefix: Option<&str>,
    ) -> Result<(), std::io::Error> {
        const MAX_EXTRACTION_BYTES: u64 = 50 * 1024 * 1024 * 1024; // 50 GB
        const MAX_EXTRACTION_ENTRIES: usize = 100_000;

        let normalized_target = crate::server::normalize_path(target);
        let entries = archive.entries()?;
        let mut total_bytes: u64 = 0;
        let mut entry_count: usize = 0;

        for entry_res in entries {
            let mut entry = entry_res?;
            entry_count += 1;
            if entry_count > MAX_EXTRACTION_ENTRIES {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Archive extraction exceeded maximum entry limit (possible zip/tar bomb)",
                ));
            }

            let entry_size = entry.header().size()?;
            total_bytes += entry_size;
            if total_bytes > MAX_EXTRACTION_BYTES {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Archive extraction exceeded maximum uncompressed size (possible zip/tar bomb)",
                ));
            }

            let raw_path = entry.path()?.to_string_lossy().replace('\\', "/");
            let clean = raw_path.trim_start_matches('/').to_string();
            if clean.is_empty() {
                continue;
            }

            // Reject paths attempting directory traversal
            for component in Path::new(&clean).components() {
                if matches!(component, std::path::Component::ParentDir) {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        format!("Security error: path traversal attempt in archive entry '{}'", raw_path),
                    ));
                }
            }

            let out_rel = if let Some(p) = strip_prefix {
                if clean == p.trim_end_matches('/') || clean == p {
                    continue; // Skip root folder itself
                }
                if let Some(stripped) = clean.strip_prefix(p) {
                    if stripped.is_empty() {
                        continue;
                    }
                    stripped.to_string()
                } else {
                    clean
                }
            } else {
                clean
            };

            // Verify traversal on stripped path
            for component in Path::new(&out_rel).components() {
                if matches!(component, std::path::Component::ParentDir | std::path::Component::RootDir | std::path::Component::Prefix(..)) {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        format!("Security error: path traversal attempt in archive entry '{}'", out_rel),
                    ));
                }
            }

            let out_path = target.join(&out_rel);
            let normalized_out = crate::server::normalize_path(&out_path);
            if !crate::server::path_starts_with_case_insensitive(&normalized_out, &normalized_target) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    format!("Security error: destination '{}' escapes extraction directory", out_rel),
                ));
            }

            // Check symlink / hardlink targets
            let entry_type = entry.header().entry_type();
            if entry_type.is_symlink() || entry_type.is_hard_link() {
                if let Ok(Some(link_target)) = entry.link_name() {
                    let link_str = link_target.to_string_lossy().replace('\\', "/");
                    let resolved_link = if link_target.is_absolute() {
                        std::path::PathBuf::from(&link_str)
                    } else if let Some(parent) = out_path.parent() {
                        parent.join(&link_target)
                    } else {
                        target.join(&link_target)
                    };
                    let norm_link = crate::server::normalize_path(&resolved_link);
                    if !crate::server::path_starts_with_case_insensitive(&norm_link, &normalized_target) {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::PermissionDenied,
                            format!("Security error: symlink/hardlink target '{}' escapes extraction directory", link_str),
                        ));
                    }
                }
            }

            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)?;
            }
            entry.unpack(&out_path)?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Ok(metadata) = fs::symlink_metadata(&out_path) {
                    if !metadata.file_type().is_symlink() {
                        let mode = metadata.permissions().mode();
                        let safe_mode = mode & 0o777; // strip setuid (0o4000) and setgid (0o2000)
                        if mode != safe_mode {
                            let _ = fs::set_permissions(&out_path, fs::Permissions::from_mode(safe_mode));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub fn extract_archive(archive_path_str: &str, target_dir_str: &str) -> Result<(), std::io::Error> {
        let (path_buf, normalized_str) = Self::normalize_archive_path(archive_path_str);
        let path = path_buf.as_path();
        let archive_path_str = normalized_str.as_str();
        let target = Path::new(target_dir_str);
        fs::create_dir_all(target)?;
        let target_name = target.file_name().and_then(|n| n.to_str()).unwrap_or("");

        let lower = archive_path_str.to_lowercase();

        if lower.ends_with(".squashfs") || lower.ends_with(".snap") || lower.ends_with(".appimage") {
            let file = BufReader::new(File::open(path)?);
            let filesystem = backhand::FilesystemReader::from_reader(file)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("SquashFS error: {}", e)))?;

            let mut common_prefix: Option<String> = None;
            let mut single_root = true;
            let mut has_entries = false;

            for node in filesystem.files() {
                let rel = node.fullpath.to_string_lossy().trim_start_matches('/').to_string();
                if rel.is_empty() {
                    continue;
                }
                has_entries = true;
                let parts: Vec<&str> = rel.split('/').collect();
                let is_dir = matches!(node.inner, backhand::InnerNode::Dir(_));
                if parts.len() <= 1 && !is_dir && !rel.ends_with('/') {
                    single_root = false;
                } else {
                    let root = parts[0];
                    match &common_prefix {
                        None => common_prefix = Some(root.to_string()),
                        Some(p) => {
                            if p != root {
                                single_root = false;
                            }
                        }
                    }
                }
            }

            let strip_prefix = if single_root && has_entries {
                if let Some(prefix) = common_prefix {
                    if Self::should_strip_root_prefix(&prefix, target_name) {
                        Some(format!("{}/", prefix))
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };

            for node in filesystem.files() {
                let rel_path = node.fullpath.to_string_lossy().trim_start_matches('/').to_string();
                if rel_path.is_empty() {
                    continue;
                }

                let out_rel = if let Some(ref p) = strip_prefix {
                    if rel_path == p.trim_end_matches('/') || rel_path == *p {
                        continue;
                    }
                    if let Some(stripped) = rel_path.strip_prefix(p) {
                        if stripped.is_empty() {
                            continue;
                        }
                        stripped.to_string()
                    } else {
                        rel_path
                    }
                } else {
                    rel_path
                };

                let out_path = target.join(&out_rel);

                match &node.inner {
                    backhand::InnerNode::Dir(_) => {
                        fs::create_dir_all(&out_path)?;
                    }
                    backhand::InnerNode::File(sq_file) => {
                        if let Some(parent) = out_path.parent() {
                            fs::create_dir_all(parent)?;
                        }
                        let mut reader = filesystem.file(sq_file).reader();
                        let mut out_file = File::create(&out_path)?;
                        std::io::copy(&mut reader, &mut out_file)?;
                    }
                    _ => {}
                }
            }
            info!("Extracted SquashFS {} to {}", archive_path_str, target_dir_str);
            return Ok(());
        } else if super::disk_image::DiskImageHandler::is_supported_image(archive_path_str) || path.is_dir() {
            match super::disk_image::DiskImageHandler::extract_image(archive_path_str, target_dir_str) {
                Ok(()) => return Ok(()),
                Err(err) => {
                    tracing::debug!("DiskImageHandler fallback to UDF/ISO extractor for {}: {}", archive_path_str, err);
                }
            }

            // 1. Try UDF extraction
            if let Ok(file) = File::open(path) {
                let reader = BufReader::new(file);
                if let Ok(volume) = hadris_udf::UdfVolume::open(reader) {
                    if let Ok(root_dir) = volume.root_dir() {
                        if root_dir.len() > 0 {
                            Self::extract_udf_dir(&volume, &root_dir, target)?;
                            info!("Extracted UDF {} to {}", archive_path_str, target_dir_str);
                            return Ok(());
                        }
                    }
                }
            }

            // 2. Fall back to ISO 9660 extraction
            let mut file = File::open(path)?;
            let vol = Self::read_iso_volume_info(&mut file)?;
            Self::extract_iso_dir_rec(&mut file, &vol, vol.root_lba, vol.root_len, target)?;
            info!("Extracted ISO {} to {}", archive_path_str, target_dir_str);
            return Ok(());
        } else if lower.ends_with(".iso") || lower.ends_with(".udf") {
            // 1. Try UDF extraction first
            if let Ok(file) = File::open(path) {
                let reader = BufReader::new(file);
                if let Ok(volume) = hadris_udf::UdfVolume::open(reader) {
                    if let Ok(root_dir) = volume.root_dir() {
                        if root_dir.len() > 0 {
                            Self::extract_udf_dir(&volume, &root_dir, target)?;
                            info!("Extracted UDF {} to {}", archive_path_str, target_dir_str);
                            return Ok(());
                        }
                    }
                }
            }

            // 2. Fall back to ISO 9660 extraction
            let mut file = File::open(path)?;
            let vol = Self::read_iso_volume_info(&mut file)?;
            Self::extract_iso_dir_rec(&mut file, &vol, vol.root_lba, vol.root_len, target)?;
            info!("Extracted ISO {} to {}", archive_path_str, target_dir_str);
            return Ok(());
        }

        if lower.ends_with(".zip") || lower.ends_with(".cbz") || lower.ends_with(".epub") {
            const MAX_EXTRACTION_BYTES: u64 = 50 * 1024 * 1024 * 1024; // 50 GB
            const MAX_EXTRACTION_ENTRIES: usize = 100_000;

            let file = File::open(path)?;
            let mut zip = ZipArchive::new(BufReader::new(file))
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
            let strip_prefix = Self::find_zip_strip_prefix(&mut zip, target_name);
            let normalized_target = crate::server::normalize_path(target);

            if zip.len() > MAX_EXTRACTION_ENTRIES {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Archive extraction exceeded maximum entry limit (possible zip bomb)",
                ));
            }

            let mut total_bytes: u64 = 0;

            for i in 0..zip.len() {
                let mut file = zip.by_index(i)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
                let raw_name = match file.enclosed_name() {
                    Some(p) => p.to_string_lossy().replace('\\', "/"),
                    None => {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::PermissionDenied,
                            format!("Security error: path traversal attempt in zip entry '{}'", file.name()),
                        ));
                    }
                };
                let clean_name = raw_name.trim_start_matches('/');
                if clean_name.is_empty() {
                    continue;
                }

                let out_rel_path = if let Some(ref p) = strip_prefix {
                    if clean_name == p.trim_end_matches('/') || clean_name == *p {
                        continue;
                    }
                    if let Some(stripped) = clean_name.strip_prefix(p) {
                        if stripped.is_empty() {
                            continue;
                        }
                        stripped.to_string()
                    } else {
                        clean_name.to_string()
                    }
                } else {
                    clean_name.to_string()
                };

                // Check directory traversal
                for component in Path::new(&out_rel_path).components() {
                    if matches!(component, std::path::Component::ParentDir | std::path::Component::RootDir | std::path::Component::Prefix(..)) {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::PermissionDenied,
                            format!("Security error: path traversal attempt in zip entry '{}'", out_rel_path),
                        ));
                    }
                }

                let out_path = target.join(&out_rel_path);
                let normalized_out = crate::server::normalize_path(&out_path);
                if !crate::server::path_starts_with_case_insensitive(&normalized_out, &normalized_target) {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        format!("Security error: destination '{}' escapes extraction directory", out_rel_path),
                    ));
                }

                if file.is_dir() {
                    fs::create_dir_all(&out_path)?;
                } else {
                    if let Some(parent) = out_path.parent() {
                        fs::create_dir_all(parent)?;
                    }

                    let file_size = file.size();
                    total_bytes += file_size;
                    if total_bytes > MAX_EXTRACTION_BYTES {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "Archive extraction exceeded maximum uncompressed size (possible zip bomb)",
                        ));
                    }

                    let mut out_file = File::create(&out_path)?;
                    std::io::copy(&mut file, &mut out_file)?;
                }

                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Some(mode) = file.unix_mode() {
                        let safe_mode = mode & 0o777; // strip setuid / setgid
                        let _ = fs::set_permissions(&out_path, fs::Permissions::from_mode(safe_mode));
                    }
                }
            }
            info!("Extracted zip {} to {}", archive_path_str, target_dir_str);
            Ok(())
        } else if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
            let file = File::open(path)?;
            let strip_prefix = Self::find_tar_strip_prefix(GzDecoder::new(BufReader::new(file)), target_name);
            let file2 = File::open(path)?;
            let mut archive = TarArchive::new(GzDecoder::new(BufReader::new(file2)));
            Self::unpack_tar_with_strip(&mut archive, target, strip_prefix.as_deref())?;
            info!("Extracted tar.gz {} to {}", archive_path_str, target_dir_str);
            Ok(())
        } else if lower.ends_with(".tar") {
            let file = File::open(path)?;
            let strip_prefix = Self::find_tar_strip_prefix(BufReader::new(file), target_name);
            let file2 = File::open(path)?;
            let mut archive = TarArchive::new(BufReader::new(file2));
            Self::unpack_tar_with_strip(&mut archive, target, strip_prefix.as_deref())?;
            info!("Extracted tar {} to {}", archive_path_str, target_dir_str);
            Ok(())
        } else if lower.ends_with(".tar.bz2") || lower.ends_with(".tbz2") {
            let file = File::open(path)?;
            let strip_prefix = Self::find_tar_strip_prefix(bzip2::read::BzDecoder::new(BufReader::new(file)), target_name);
            let file2 = File::open(path)?;
            let mut archive = TarArchive::new(bzip2::read::BzDecoder::new(BufReader::new(file2)));
            Self::unpack_tar_with_strip(&mut archive, target, strip_prefix.as_deref())?;
            info!("Extracted tar.bz2 {} to {}", archive_path_str, target_dir_str);
            Ok(())
        } else {
            let status = std::process::Command::new("tar")
                .arg("-xf")
                .arg(archive_path_str)
                .arg("-C")
                .arg(target_dir_str)
                .status();

            if let Ok(s) = status {
                if s.success() {
                    return Ok(());
                }
            }

            let status_7z = std::process::Command::new("7z")
                .arg("x")
                .arg(format!("-o{}", target_dir_str))
                .arg(archive_path_str)
                .arg("-y")
                .status();

            if let Ok(s) = status_7z {
                if s.success() {
                    return Ok(());
                }
            }

            Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Unsupported archive format for extraction: {}", archive_path_str),
            ))
        }
    }

    pub fn create_zip(source_paths: &[String], target_archive: &str) -> Result<(), std::io::Error> {
        let file = File::create(target_archive)?;
        let mut zip = ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(0o755);

        for src_str in source_paths {
            let src_path = Path::new(src_str);
            let base_name = src_path.file_name().unwrap_or_default().to_string_lossy();

            if src_path.is_dir() {
                for entry in walkdir::WalkDir::new(src_path) {
                    let entry = entry.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
                    let rel = entry.path().strip_prefix(src_path).unwrap();
                    let entry_name = if rel.as_os_str().is_empty() {
                        format!("{}/", base_name)
                    } else {
                        format!("{}/{}", base_name, rel.to_string_lossy().replace('\\', "/"))
                    };

                    if entry.file_type().is_dir() {
                        zip.add_directory(&entry_name, options)
                            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
                    } else {
                        zip.start_file(&entry_name, options)
                            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
                        let mut f = File::open(entry.path())?;
                        std::io::copy(&mut f, &mut zip)?;
                    }
                }
            } else if src_path.is_file() {
                zip.start_file(base_name.as_ref(), options)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
                let mut f = File::open(src_path)?;
                std::io::copy(&mut f, &mut zip)?;
            }
        }

        zip.finish().map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        info!("Created zip archive: {}", target_archive);
        Ok(())
    }

    pub fn create_targz(source_paths: &[String], target_archive: &str) -> Result<(), std::io::Error> {
        let tar_gz = File::create(target_archive)?;
        let enc = GzEncoder::new(tar_gz, Compression::default());
        let mut tar = TarBuilder::new(enc);

        for src_str in source_paths {
            let src_path = Path::new(src_str);
            let base_name = src_path.file_name().unwrap_or_default().to_string_lossy();

            if src_path.is_dir() {
                tar.append_dir_all(base_name.as_ref(), src_path)?;
            } else if src_path.is_file() {
                let mut file = File::open(src_path)?;
                tar.append_file(base_name.as_ref(), &mut file)?;
            }
        }

        tar.finish()?;
        info!("Created tar.gz archive: {}", target_archive);
        Ok(())
    }

    fn extract_udf_dir<R: hadris_udf::Read + hadris_udf::Seek>(
        volume: &hadris_udf::UdfVolume<R>,
        dir: &hadris_udf::UdfDir,
        target: &Path,
    ) -> Result<(), std::io::Error> {
        fs::create_dir_all(target)?;
        for entry in dir.entries() {
            let name = entry.name();
            if name.is_empty() || name == "." || name == ".." {
                continue;
            }
            let out_path = target.join(name);
            if entry.is_dir() {
                let child_dir = volume.read_directory(&entry.icb)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("UDF read_dir error: {:?}", e)))?;
                Self::extract_udf_dir(volume, &child_dir, &out_path)?;
            } else {
                if let Some(parent) = out_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                let data = volume.read_file(entry)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("UDF read_file error: {:?}", e)))?;
                fs::write(&out_path, &data)?;
            }
        }
        Ok(())
    }

    fn read_iso_volume_info<R: Read + std::io::Seek>(reader: &mut R) -> Result<IsoVolumeInfo, std::io::Error> {
        let sector_size = 2048u64;
        let mut sector = 16u64;
        let mut pvd_root: Option<(u32, u32)> = None;
        let mut svd_root: Option<(u32, u32)> = None;
        let mut pvd_has_rr = false;

        let mut buf = [0u8; 2048];
        while sector <= 100 {
            if reader.seek(std::io::SeekFrom::Start(sector * sector_size)).is_err() {
                break;
            }
            if reader.read_exact(&mut buf).is_err() {
                break;
            }

            // Validate CD001 identifier
            if &buf[1..6] != b"CD001" || buf[6] != 1 {
                sector += 1;
                continue;
            }

            let desc_type = buf[0];
            if desc_type == 255 {
                // Terminator
                break;
            }

            if desc_type == 1 {
                // Primary Volume Descriptor
                let root_rec = &buf[156..190];
                if root_rec[0] >= 34 {
                    let lba = u32::from_le_bytes(root_rec[2..6].try_into().unwrap_or_default());
                    let len = u32::from_le_bytes(root_rec[10..14].try_into().unwrap_or_default());
                    pvd_root = Some((lba, len));
                }
            } else if desc_type == 2 {
                // Supplementary Volume Descriptor (Joliet or other unicode)
                let root_rec = &buf[156..190];
                if root_rec[0] >= 34 {
                    let lba = u32::from_le_bytes(root_rec[2..6].try_into().unwrap_or_default());
                    let len = u32::from_le_bytes(root_rec[10..14].try_into().unwrap_or_default());
                    svd_root = Some((lba, len));
                }
            }

            sector += 1;
        }

        // Check if PVD has Rock Ridge by peeking into PVD root directory records
        if let Some((pvd_lba, pvd_len)) = pvd_root {
            if let Ok(records) = Self::read_directory_records(reader, pvd_lba, pvd_len, false) {
                for rec in records {
                    if rec.mode_octal.is_some() || rec.is_symlink {
                        pvd_has_rr = true;
                        break;
                    }
                }
            }
        }

        // If Rock Ridge is present, prefer PVD with Rock Ridge.
        // If Joliet SVD is present, use SVD with Joliet unicode.
        // Otherwise use PVD.
        if pvd_has_rr {
            let (lba, len) = pvd_root.unwrap();
            Ok(IsoVolumeInfo {
                root_lba: lba,
                root_len: len,
                is_joliet: false,
                has_rock_ridge: true,
            })
        } else if let Some((svd_lba, svd_len)) = svd_root {
            Ok(IsoVolumeInfo {
                root_lba: svd_lba,
                root_len: svd_len,
                is_joliet: true,
                has_rock_ridge: false,
            })
        } else if let Some((pvd_lba, pvd_len)) = pvd_root {
            Ok(IsoVolumeInfo {
                root_lba: pvd_lba,
                root_len: pvd_len,
                is_joliet: false,
                has_rock_ridge: false,
            })
        } else {
            Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "No valid ISO 9660 volume descriptor found"))
        }
    }

    fn read_directory_records<R: Read + std::io::Seek>(
        reader: &mut R,
        extent_lba: u32,
        extent_len: u32,
        is_joliet: bool,
    ) -> Result<Vec<IsoRecord>, std::io::Error> {
        let sector_size = 2048usize;
        let mut records = Vec::new();
        let num_sectors = (extent_len as usize).div_ceil(sector_size);

        let mut sector_buf = vec![0u8; sector_size];

        for s in 0..num_sectors {
            let sector_pos = (extent_lba as u64 + s as u64) * 2048;
            reader.seek(std::io::SeekFrom::Start(sector_pos))?;
            reader.read_exact(&mut sector_buf)?;

            let mut offset = 0;
            while offset < sector_size {
                let len_dr = sector_buf[offset] as usize;
                if len_dr == 0 {
                    // Zero length padding at end of sector, jump to next sector
                    break;
                }
                if offset + len_dr > sector_size {
                    break;
                }

                let dr_bytes = &sector_buf[offset..offset + len_dr];
                if dr_bytes.len() >= 33 {
                    let extent_lba = u32::from_le_bytes(dr_bytes[2..6].try_into().unwrap_or_default());
                    let data_length = u32::from_le_bytes(dr_bytes[10..14].try_into().unwrap_or_default());
                    let flags = dr_bytes[25];
                    let is_dir = (flags & 0x02) != 0;
                    let len_fi = dr_bytes[32] as usize;

                    if 33 + len_fi <= dr_bytes.len() {
                        let fi_bytes = &dr_bytes[33..33 + len_fi];

                        // Parse date/time
                        let year = dr_bytes[18] as i32 + 1900;
                        let month = dr_bytes[19] as u32;
                        let day = dr_bytes[20] as u32;
                        let hour = dr_bytes[21] as u32;
                        let min = dr_bytes[22] as u32;
                        let sec = dr_bytes[23] as u32;
                        let modified_unix = chrono::NaiveDate::from_ymd_opt(year, month, day)
                            .and_then(|d| d.and_hms_opt(hour, min, sec))
                            .map(|dt| dt.and_utc().timestamp() as u64);

                        // Parse SUSP / Rock Ridge
                        let susp_offset = 33 + len_fi + (if len_fi % 2 == 0 { 1 } else { 0 });
                        let (alt_name, px_mode, px_uid, px_gid, symlink) = if susp_offset < dr_bytes.len() {
                            Self::parse_susp_fields(&dr_bytes[susp_offset..])
                        } else {
                            (None, None, None, None, None)
                        };

                        let final_name = if len_fi == 1 && fi_bytes[0] == 0 {
                            ".".to_string()
                        } else if len_fi == 1 && fi_bytes[0] == 1 {
                            "..".to_string()
                        } else if let Some(rr_name) = alt_name {
                            rr_name
                        } else if is_joliet {
                            Self::decode_joliet_string(fi_bytes)
                        } else {
                            let ascii = String::from_utf8_lossy(fi_bytes).to_string();
                            if let Some(stripped) = ascii.strip_suffix(";1") {
                                stripped.to_string()
                            } else {
                                ascii
                            }
                        };

                        let is_symlink = symlink.is_some();
                        let mode_octal = px_mode.map(|m| format!("{:04o}", m & 0o7777));

                        records.push(IsoRecord {
                            name: final_name,
                            extent_lba,
                            data_length,
                            is_dir,
                            is_symlink,
                            symlink_target: symlink,
                            modified_unix,
                            mode_octal,
                            uid: px_uid.unwrap_or(0),
                            gid: px_gid.unwrap_or(0),
                        });
                    }
                }

                offset += len_dr;
            }
        }

        Ok(records)
    }

    fn find_iso_record<R: Read + std::io::Seek>(
        reader: &mut R,
        vol: &IsoVolumeInfo,
        path_components: &[&str],
    ) -> Result<IsoRecord, std::io::Error> {
        if path_components.is_empty() {
            return Ok(IsoRecord {
                name: "/".to_string(),
                extent_lba: vol.root_lba,
                data_length: vol.root_len,
                is_dir: true,
                is_symlink: false,
                symlink_target: None,
                modified_unix: None,
                mode_octal: None,
                uid: 0,
                gid: 0,
            });
        }

        let mut current_lba = vol.root_lba;
        let mut current_len = vol.root_len;

        for (idx, comp) in path_components.iter().enumerate() {
            let is_last = idx == path_components.len() - 1;
            let records = Self::read_directory_records(reader, current_lba, current_len, vol.is_joliet)?;

            let found = records.into_iter().find(|r| {
                if r.name == "." || r.name == ".." {
                    return false;
                }
                r.name.eq_ignore_ascii_case(comp)
            });

            match found {
                Some(rec) => {
                    if is_last {
                        return Ok(rec);
                    }
                    if !rec.is_dir {
                        return Err(std::io::Error::new(std::io::ErrorKind::Other, format!("Component is not a directory: {}", comp)));
                    }
                    current_lba = rec.extent_lba;
                    current_len = rec.data_length;
                }
                None => return Err(std::io::Error::new(std::io::ErrorKind::NotFound, format!("ISO entry not found: {}", comp))),
            }
        }

        Err(std::io::Error::new(std::io::ErrorKind::NotFound, "Entry not found"))
    }

    fn read_iso_record_bytes<R: Read + std::io::Seek>(
        reader: &mut R,
        record: &IsoRecord,
        max_bytes: usize,
    ) -> Result<Vec<u8>, std::io::Error> {
        let to_read = if max_bytes > 0 {
            std::cmp::min(max_bytes, record.data_length as usize)
        } else {
            record.data_length as usize
        };

        let start_pos = record.extent_lba as u64 * 2048;
        reader.seek(std::io::SeekFrom::Start(start_pos))?;
        let mut buffer = vec![0u8; to_read];
        reader.read_exact(&mut buffer)?;
        Ok(buffer)
    }

    fn extract_iso_dir_rec<R: Read + std::io::Seek>(
        reader: &mut R,
        vol: &IsoVolumeInfo,
        extent_lba: u32,
        extent_len: u32,
        target: &Path,
    ) -> Result<(), std::io::Error> {
        fs::create_dir_all(target)?;
        let records = Self::read_directory_records(reader, extent_lba, extent_len, vol.is_joliet)?;
        for rec in records {
            if rec.name == "." || rec.name == ".." || rec.name.is_empty() {
                continue;
            }
            let out_path = target.join(&rec.name);
            if rec.is_dir {
                Self::extract_iso_dir_rec(reader, vol, rec.extent_lba, rec.data_length, &out_path)?;
            } else {
                if let Some(parent) = out_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                let bytes = Self::read_iso_record_bytes(reader, &rec, 0)?;
                fs::write(&out_path, &bytes)?;
            }
        }
        Ok(())
    }

    fn decode_joliet_string(bytes: &[u8]) -> String {
        let mut chars = Vec::new();
        for chunk in bytes.chunks_exact(2) {
            let code = u16::from_be_bytes([chunk[0], chunk[1]]);
            if code != 0 {
                if let Some(c) = char::from_u32(code as u32) {
                    chars.push(c);
                }
            }
        }
        let s: String = chars.into_iter().collect();
        if let Some(stripped) = s.strip_suffix(";1") {
            stripped.to_string()
        } else {
            s
        }
    }

    fn parse_susp_fields(susp_bytes: &[u8]) -> (Option<String>, Option<u32>, Option<u32>, Option<u32>, Option<String>) {
        let mut alt_name: Option<String> = None;
        let mut mode: Option<u32> = None;
        let mut uid: Option<u32> = None;
        let mut gid: Option<u32> = None;
        let mut symlink: Option<String> = None;

        let mut i = 0;
        while i + 4 <= susp_bytes.len() {
            let sig = &susp_bytes[i..i+2];
            let len = susp_bytes[i+2] as usize;
            let _ver = susp_bytes[i+3];
            if len < 4 || i + len > susp_bytes.len() {
                break;
            }
            let payload = &susp_bytes[i+4..i+len];
            match sig {
                b"NM" => {
                    if !payload.is_empty() {
                        let _flags = payload[0];
                        let name_bytes = &payload[1..];
                        let name_part = String::from_utf8_lossy(name_bytes).to_string();
                        if let Some(ref mut existing) = alt_name {
                            existing.push_str(&name_part);
                        } else {
                            alt_name = Some(name_part);
                        }
                    }
                }
                b"PX" => {
                    if payload.len() >= 32 {
                        let file_mode = u32::from_le_bytes(payload[0..4].try_into().unwrap_or_default());
                        let _links = u32::from_le_bytes(payload[8..12].try_into().unwrap_or_default());
                        let file_uid = u32::from_le_bytes(payload[16..20].try_into().unwrap_or_default());
                        let file_gid = u32::from_le_bytes(payload[24..28].try_into().unwrap_or_default());
                        mode = Some(file_mode);
                        uid = Some(file_uid);
                        gid = Some(file_gid);
                    }
                }
                b"SL" => {
                    if !payload.is_empty() {
                        let mut sl_idx = 1;
                        let mut sl_target = String::new();
                        while sl_idx + 2 <= payload.len() {
                            let comp_flags = payload[sl_idx];
                            let comp_len = payload[sl_idx+1] as usize;
                            if sl_idx + 2 + comp_len > payload.len() {
                                break;
                            }
                            let comp_data = &payload[sl_idx+2..sl_idx+2+comp_len];
                            if comp_flags & 0x02 != 0 {
                                sl_target.push_str("./");
                            } else if comp_flags & 0x04 != 0 {
                                sl_target.push_str("../");
                            } else if comp_flags & 0x08 != 0 {
                                sl_target.push('/');
                            } else {
                                if !sl_target.is_empty() && !sl_target.ends_with('/') {
                                    sl_target.push('/');
                                }
                                sl_target.push_str(&String::from_utf8_lossy(comp_data));
                            }
                            sl_idx += 2 + comp_len;
                        }
                        if !sl_target.is_empty() {
                            symlink = Some(sl_target);
                        }
                    }
                }
                _ => {}
            }
            i += len;
        }
        (alt_name, mode, uid, gid, symlink)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use backhand::{FilesystemWriter, NodeHeader};
    use std::io::Cursor;

    #[test]
    fn test_is_archive_file_extensions() {
        assert!(crate::vfs::is_archive_file("ubuntu-24.04-live-server.iso"));
        assert!(crate::vfs::is_archive_file("optical_disc.udf"));
        assert!(crate::vfs::is_archive_file("disk_image.img"));
        assert!(crate::vfs::is_archive_file("rootfs.squashfs"));
        assert!(crate::vfs::is_archive_file("core22_123.snap"));
        assert!(crate::vfs::is_archive_file("cursor.AppImage"));
        assert!(crate::vfs::is_archive_file("windows11.qcow2"));
        assert!(crate::vfs::is_archive_file("debian_vm.vmdk"));
        assert!(crate::vfs::is_archive_file("hyperv_disk.vhd"));
        assert!(crate::vfs::is_archive_file("hyperv_gen2.vhdx"));
        assert!(crate::vfs::is_archive_file("partclone_backup.zst"));
        assert!(crate::vfs::is_archive_file("archive.tar.gz"));
        assert!(crate::vfs::is_archive_file("bundle.zip"));
        assert!(!crate::vfs::is_archive_file("document.pdf"));
        assert!(!crate::vfs::is_archive_file("script.sh"));
    }

    #[test]
    fn test_squashfs_lifecycle() {
        let temp_dir = tempfile::tempdir().unwrap();
        let squashfs_path = temp_dir.path().join("test_bundle.squashfs");

        let mut fs = FilesystemWriter::default();
        let header = NodeHeader {
            permissions: 0o644,
            uid: 1000,
            gid: 1000,
            mtime: 1700000000,
        };
        let dir_header = NodeHeader {
            permissions: 0o755,
            uid: 1000,
            gid: 1000,
            mtime: 1700000000,
        };

        fs.push_file(Cursor::new(b"Hello Brum SquashFS!"), "hello.txt", header).unwrap();
        fs.push_dir("subfolder", dir_header).unwrap();
        fs.push_file(Cursor::new(b"Nested file content in SquashFS"), "subfolder/nested.txt", header).unwrap();

        let mut out_file = File::create(&squashfs_path).unwrap();
        fs.write(&mut out_file).unwrap();
        drop(out_file);

        let path_str = squashfs_path.to_string_lossy().to_string();

        // 1. List Root
        let root_listing = ArchiveHandler::list_archive_contents(&path_str, "").unwrap();
        assert_eq!(root_listing.protocol, "archive");
        assert!(root_listing.entries.iter().any(|e| e.name == "hello.txt" && !e.is_dir && e.size == 20));
        assert!(root_listing.entries.iter().any(|e| e.name == "subfolder" && e.is_dir));

        // 2. List Subfolder
        let sub_listing = ArchiveHandler::list_archive_contents(&path_str, "subfolder").unwrap();
        assert!(sub_listing.entries.iter().any(|e| e.name == "nested.txt" && !e.is_dir));

        // 3. Read File
        let file_content = ArchiveHandler::read_archive_entry(&path_str, "hello.txt", 0).unwrap();
        assert_eq!(file_content.content, "Hello Brum SquashFS!");
        assert_eq!(file_content.size, 20);
        assert!(!file_content.is_binary);

        let nested_content = ArchiveHandler::read_archive_entry(&path_str, "subfolder/nested.txt", 0).unwrap();
        assert_eq!(nested_content.content, "Nested file content in SquashFS");

        // 4. Extract
        let extract_dir = temp_dir.path().join("extracted");
        ArchiveHandler::extract_archive(&path_str, &extract_dir.to_string_lossy()).unwrap();

        let extracted_hello = extract_dir.join("hello.txt");
        assert!(extracted_hello.is_file());
        assert_eq!(fs::read_to_string(extracted_hello).unwrap(), "Hello Brum SquashFS!");
    }

    #[test]
    fn test_real_iso_reading() {
        let test_isos = [
            "/tmp/test_iso_pure.iso",
            "/tmp/test_iso_joliet.iso",
            "/tmp/test_iso_udf.iso",
            "/tmp/test_joliet_rr.iso",
            "/tmp/test_joliet_only.iso",
            "/tmp/test_rr_only.iso",
            "/tmp/test_udf_hybrid.iso",
        ];

        for name in &test_isos {
            if Path::new(name).exists() {
                println!("--- Testing ISO: {} ---", name);
                let root_listing = ArchiveHandler::list_archive_contents(name, "").unwrap();
                println!("Entries count for {}: {}", name, root_listing.entries.len());
                for e in &root_listing.entries {
                    println!("  Entry: {} (is_dir: {}, size: {})", e.name, e.is_dir, e.size);
                }
                assert!(!root_listing.entries.is_empty(), "ISO {} root listing must not be empty", name);

                // If sub_folder exists, test subpath navigation
                if let Some(sub) = root_listing.entries.iter().find(|e| e.is_dir) {
                    println!("  Testing subpath listing for: {}", sub.name);
                    let sub_listing = ArchiveHandler::list_archive_contents(name, &sub.name);
                    println!("  Sub listing result: {:?}", sub_listing);
                    if let Ok(sl) = sub_listing {
                        assert!(!sl.entries.is_empty(), "Subfolder {} must have entries", sub.name);
                    }
                }

                // If file exists, test reading file
                if let Some(file_entry) = root_listing.entries.iter().find(|e| !e.is_dir) {
                    println!("  Testing read entry for: {}", file_entry.name);
                    let content_res = ArchiveHandler::read_archive_entry(name, &file_entry.name, 0);
                    println!("  Read result: {:?}", content_res);
                    assert!(content_res.is_ok(), "Must be able to read file entry {}", file_entry.name);
                }

                // Test extraction
                let temp_dir = tempfile::tempdir().unwrap();
                let extract_res = ArchiveHandler::extract_archive(name, &temp_dir.path().to_string_lossy());
                println!("  Extract result: {:?}", extract_res);
                assert!(extract_res.is_ok(), "Must be able to extract ISO {}", name);
            }
        }
    }

    #[test]
    fn test_normalize_archive_path_and_fog_navigation() {
        let (p1, s1) = ArchiveHandler::normalize_archive_path("/home/bolt/images/w10fog/");
        assert_eq!(s1, "/home/bolt/images/w10fog");
        assert_eq!(p1.to_string_lossy(), "/home/bolt/images/w10fog");

        let (p2, s2) = ArchiveHandler::normalize_archive_path("/C:/Users/name/images/w10fog%20test/");
        assert_eq!(s2, "C:/Users/name/images/w10fog test");
        assert_eq!(p2.to_string_lossy(), "C:/Users/name/images/w10fog test");

        // Test with real FOG directory if exists
        let fog_path = "/home/bolt/projects/Test/test-images/w10fog";
        if Path::new(fog_path).exists() {
            let listing = ArchiveHandler::list_archive_contents(&format!("{}/", fog_path), "").unwrap();
            assert!(listing.entries.iter().any(|e| e.name == "[FOG Image Summary.txt]"));
            assert!(listing.entries.iter().any(|e| e.name.contains("fat32")));

            let report = ArchiveHandler::read_archive_entry(fog_path, "[FOG Image Summary.txt]", 0).unwrap();
            assert!(report.content.contains("FOG PROJECT IMAGE MANIFEST"));

            // Test partition 4 (NTFS partition listing)
            let p4_entry = listing.entries.iter().find(|e| e.name.contains("p4")).unwrap();
            let p4_listing = ArchiveHandler::list_archive_contents(fog_path, &p4_entry.name).unwrap();
            assert_eq!(p4_listing.current_path, format!("archive://{}#{}", fog_path, p4_entry.name));
            assert!(p4_listing.entries.iter().any(|e| e.name.eq_ignore_ascii_case("Recovery")));

            let rec_listing = ArchiveHandler::list_archive_contents(fog_path, &format!("{}/Recovery", p4_entry.name)).unwrap();
            eprintln!("Recovery entries: {:?}", rec_listing.entries.iter().map(|e| (&e.name, e.is_dir, e.size)).collect::<Vec<_>>());
            assert_eq!(rec_listing.current_path, format!("archive://{}#{}/Recovery", fog_path, p4_entry.name));

            let p4_info = ArchiveHandler::read_archive_entry(fog_path, &format!("{}/[Partition Info.txt]", p4_entry.name), 0).unwrap();
            assert!(p4_info.content.contains("PARTITION INFORMATION"));
            assert!(p4_info.content.contains("d1p4.img"));

            // Test partition 1 (FAT32 EFI partition)
            let p1_entry = listing.entries.iter().find(|e| e.name.contains("p1")).unwrap();
            let p1_listing = ArchiveHandler::list_archive_contents(fog_path, &p1_entry.name).unwrap();
            assert_eq!(p1_listing.current_path, format!("archive://{}#{}", fog_path, p1_entry.name));
        }
    }

    #[test]
    fn test_extract_archive_smart_root_stripping() {
        let temp_dir = tempfile::tempdir().unwrap();
        let src_folder = temp_dir.path().join("myfolder");
        fs::create_dir_all(&src_folder).unwrap();
        fs::write(src_folder.join("file1.txt"), "hello from file 1").unwrap();
        fs::write(src_folder.join("file2.txt"), "hello from file 2").unwrap();

        // 1. Create ZIP of folder
        let zip_path = temp_dir.path().join("myfolder.zip");
        ArchiveHandler::create_zip(&[src_folder.to_string_lossy().to_string()], &zip_path.to_string_lossy()).unwrap();
        assert!(zip_path.exists());

        // 2. Test "Extract to <myfolder>" (target directory has name "myfolder")
        // Should strip redundant "myfolder/" and place file1.txt directly into target
        let extract_subfolder = temp_dir.path().join("extracted_sub").join("myfolder");
        ArchiveHandler::extract_archive(&zip_path.to_string_lossy(), &extract_subfolder.to_string_lossy()).unwrap();
        assert!(extract_subfolder.join("file1.txt").exists(), "file1.txt should be in target directly");
        assert!(extract_subfolder.join("file2.txt").exists(), "file2.txt should be in target directly");
        assert!(!extract_subfolder.join("myfolder").join("file1.txt").exists(), "Must not create duplicate nested myfolder/myfolder");

        // 3. Test "Extract Here" (target directory is parent "extracted_here")
        let extract_here = temp_dir.path().join("extracted_here");
        ArchiveHandler::extract_archive(&zip_path.to_string_lossy(), &extract_here.to_string_lossy()).unwrap();
        assert!(extract_here.join("myfolder").join("file1.txt").exists(), "Extract here should create myfolder/file1.txt");

        // 4. Test Tar.gz
        let targz_path = temp_dir.path().join("myfolder.tar.gz");
        ArchiveHandler::create_targz(&[src_folder.to_string_lossy().to_string()], &targz_path.to_string_lossy()).unwrap();
        assert!(targz_path.exists());

        let extract_tar_sub = temp_dir.path().join("extracted_tar_sub").join("myfolder");
        ArchiveHandler::extract_archive(&targz_path.to_string_lossy(), &extract_tar_sub.to_string_lossy()).unwrap();
        assert!(extract_tar_sub.join("file1.txt").exists(), "Tar extract to subfolder should strip duplicate root");
        assert!(!extract_tar_sub.join("myfolder").join("file1.txt").exists(), "Tar must not create duplicate nested folder");
    }
}
