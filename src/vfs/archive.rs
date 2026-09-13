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

pub struct ArchiveHandler;

impl ArchiveHandler {
    pub fn list_archive_contents(
        archive_path_str: &str,
        subpath_filter: &str,
    ) -> Result<DirectoryListing, std::io::Error> {
        let path = Path::new(archive_path_str);
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

    pub fn read_archive_entry(
        archive_path_str: &str,
        inner_path: &str,
        max_bytes: usize,
    ) -> Result<FileContentResponse, std::io::Error> {
        let path = Path::new(archive_path_str);
        let lower = archive_path_str.to_lowercase();
        let clean_inner = inner_path.trim_matches('/');

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

    pub fn extract_archive(archive_path_str: &str, target_dir_str: &str) -> Result<(), std::io::Error> {
        let path = Path::new(archive_path_str);
        let target = Path::new(target_dir_str);
        fs::create_dir_all(target)?;

        let lower = archive_path_str.to_lowercase();

        if lower.ends_with(".zip") || lower.ends_with(".cbz") || lower.ends_with(".epub") {
            let file = File::open(path)?;
            let mut zip = ZipArchive::new(BufReader::new(file))
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
            zip.extract(target)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
            info!("Extracted zip {} to {}", archive_path_str, target_dir_str);
            Ok(())
        } else if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
            let file = File::open(path)?;
            let tar = GzDecoder::new(file);
            let mut archive = TarArchive::new(tar);
            archive.unpack(target)?;
            info!("Extracted tar.gz {} to {}", archive_path_str, target_dir_str);
            Ok(())
        } else if lower.ends_with(".tar") {
            let file = File::open(path)?;
            let mut archive = TarArchive::new(file);
            archive.unpack(target)?;
            info!("Extracted tar {} to {}", archive_path_str, target_dir_str);
            Ok(())
        } else if lower.ends_with(".tar.bz2") || lower.ends_with(".tbz2") {
            let file = File::open(path)?;
            let bz = bzip2::read::BzDecoder::new(file);
            let mut archive = TarArchive::new(bz);
            archive.unpack(target)?;
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
}
