use super::{DirectoryListing, FileContentResponse, FileEntry};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use tracing::info;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiskFsType {
    Fat12,
    Fat16,
    Fat32,
    Ext4,
    Iso9660,
    Udf,
    Squashfs,
    Unknown,
}

impl DiskFsType {
    pub fn display_name(&self) -> &'static str {
        match self {
            DiskFsType::Fat12 => "FAT12",
            DiskFsType::Fat16 => "FAT16",
            DiskFsType::Fat32 => "FAT32",
            DiskFsType::Ext4 => "Ext2/3/4",
            DiskFsType::Iso9660 => "ISO 9660",
            DiskFsType::Udf => "UDF",
            DiskFsType::Squashfs => "SquashFS",
            DiskFsType::Unknown => "Raw/Unknown",
        }
    }
}

#[derive(Debug, Clone)]
pub struct DiskPartition {
    pub index: usize,
    pub name: String,
    pub start_byte: u64,
    pub size_bytes: u64,
    pub fs_type: DiskFsType,
    pub table_type: String, // "MBR", "GPT", "Raw"
    pub type_str: String,
    pub bootable: bool,
}

impl DiskPartition {
    pub fn slug(&self) -> String {
        let clean_name = self.name.to_lowercase()
            .replace(' ', "-")
            .replace('>', "")
            .replace('<', "")
            .replace('(', "")
            .replace(')', "")
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '-')
            .collect::<String>()
            .trim_matches('-')
            .to_string();

        if self.fs_type != DiskFsType::Unknown {
            let fs_slug = match self.fs_type {
                DiskFsType::Fat12 => "fat12",
                DiskFsType::Fat16 => "fat16",
                DiskFsType::Fat32 => "fat32",
                DiskFsType::Ext4 => "ext4",
                DiskFsType::Iso9660 => "iso",
                DiskFsType::Udf => "udf",
                DiskFsType::Squashfs => "squashfs",
                DiskFsType::Unknown => "raw",
            };
            if clean_name.is_empty() || clean_name == fs_slug {
                format!("p{}-{}", self.index, fs_slug)
            } else {
                format!("p{}-{}-{}", self.index, clean_name, fs_slug)
            }
        } else if !clean_name.is_empty() {
            format!("p{}-{}", self.index, clean_name)
        } else {
            format!("p{}-raw", self.index)
        }
    }
}

/// A slice over a Read + Seek stream bounded to [start, start + size]
pub struct StreamSlice<R> {
    inner: R,
    start: u64,
    size: u64,
    pos: u64,
}

impl<R> StreamSlice<R> {
    pub fn new(inner: R, start: u64, size: u64) -> Self {
        Self {
            inner,
            start,
            size,
            pos: 0,
        }
    }
}

impl<R: Read + Seek> Read for StreamSlice<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.pos >= self.size {
            return Ok(0);
        }
        let max_to_read = (self.size - self.pos).min(buf.len() as u64) as usize;
        self.inner.seek(SeekFrom::Start(self.start + self.pos))?;
        let bytes_read = self.inner.read(&mut buf[..max_to_read])?;
        self.pos += bytes_read as u64;
        Ok(bytes_read)
    }
}

impl<R: Seek> Seek for StreamSlice<R> {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let new_pos = match pos {
            SeekFrom::Start(offset) => offset as i64,
            SeekFrom::Current(offset) => (self.pos as i64) + offset,
            SeekFrom::End(offset) => (self.size as i64) + offset,
        };
        if new_pos < 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Negative seek offset in partition slice",
            ));
        }
        self.pos = (new_pos as u64).min(self.size);
        Ok(self.pos)
    }
}

impl<R: Read + Seek> std::io::Write for StreamSlice<R> {
    fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "Read-only partition slice",
        ))
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub struct DiskImageHandler;

impl DiskImageHandler {
    /// Probes partition table (GPT, MBR, or unpartitioned) and filesystem types of a disk image
    pub fn probe_image(archive_path_str: &str) -> Result<Vec<DiskPartition>, std::io::Error> {
        let mut file = File::open(archive_path_str)?;
        let file_len = file.metadata()?.len();
        if file_len < 512 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Image file is smaller than 512 bytes",
            ));
        }

        let mut sector_0 = [0u8; 512];
        file.seek(SeekFrom::Start(0))?;
        file.read_exact(&mut sector_0)?;

        let mut partitions = Vec::new();

        // 1. Try GPT (GUID Partition Table)
        if file_len >= 1024 {
            let mut sector_1 = [0u8; 512];
            file.seek(SeekFrom::Start(512))?;
            if file.read_exact(&mut sector_1).is_ok() && &sector_1[0..8] == b"EFI PART" {
                let part_entry_lba = u64::from_le_bytes(sector_1[72..80].try_into().unwrap_or([0; 8]));
                let num_entries = u32::from_le_bytes(sector_1[80..84].try_into().unwrap_or([0; 4]));
                let entry_size = u32::from_le_bytes(sector_1[84..88].try_into().unwrap_or([0; 4]));

                if part_entry_lba >= 2 && num_entries > 0 && entry_size >= 128 && (num_entries as u64 * entry_size as u64) < file_len {
                    let mut entry_buf = vec![0u8; entry_size as usize];
                    let mut part_idx = 1;
                    for i in 0..num_entries.min(128) {
                        let offset = part_entry_lba * 512 + (i as u64 * entry_size as u64);
                        if offset + (entry_size as u64) > file_len {
                            break;
                        }
                        file.seek(SeekFrom::Start(offset))?;
                        if file.read_exact(&mut entry_buf).is_err() {
                            break;
                        }

                        let type_guid = &entry_buf[0..16];
                        if type_guid.iter().all(|&b| b == 0) {
                            continue; // Unused partition entry
                        }

                        let first_lba = u64::from_le_bytes(entry_buf[32..40].try_into().unwrap_or([0; 8]));
                        let last_lba = u64::from_le_bytes(entry_buf[40..48].try_into().unwrap_or([0; 8]));
                        if last_lba < first_lba || first_lba == 0 {
                            continue;
                        }

                        let start_byte = first_lba * 512;
                        let size_bytes = (last_lba - first_lba + 1) * 512;
                        if start_byte + size_bytes > file_len {
                            continue;
                        }

                        // Decode UTF-16LE partition name (72 bytes = 36 chars)
                        let mut name_chars = Vec::new();
                        for chunk in entry_buf[56..128].chunks_exact(2) {
                            let code = u16::from_le_bytes([chunk[0], chunk[1]]);
                            if code == 0 {
                                break;
                            }
                            if let Some(c) = char::from_u32(code as u32) {
                                name_chars.push(c);
                            }
                        }
                        let mut part_name: String = name_chars.into_iter().collect();
                        if part_name.trim().is_empty() {
                            part_name = Self::describe_gpt_guid(type_guid);
                        }

                        let fs_type = Self::probe_filesystem(&mut file, start_byte, size_bytes)?;

                        partitions.push(DiskPartition {
                            index: part_idx,
                            name: part_name,
                            start_byte,
                            size_bytes,
                            fs_type,
                            table_type: "GPT".to_string(),
                            type_str: format!("{:02X}{:02X}{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
                                type_guid[3], type_guid[2], type_guid[1], type_guid[0],
                                type_guid[5], type_guid[4],
                                type_guid[7], type_guid[6],
                                type_guid[8], type_guid[9],
                                type_guid[10], type_guid[11], type_guid[12], type_guid[13], type_guid[14], type_guid[15]
                            ),
                            bootable: false,
                        });
                        part_idx += 1;
                    }
                }
            }
        }

        // 2. If no GPT partitions, try MBR (Master Boot Record)
        if partitions.is_empty() && sector_0[510..512] == [0x55, 0xAA] {
            let mut part_idx = 1;
            for slot in 0..4 {
                let off = 446 + slot * 16;
                let status = sector_0[off];
                let part_type = sector_0[off + 4];
                let start_lba = u32::from_le_bytes(sector_0[off + 8..off + 12].try_into().unwrap_or([0; 4])) as u64;
                let num_sectors = u32::from_le_bytes(sector_0[off + 12..off + 16].try_into().unwrap_or([0; 4])) as u64;

                if part_type != 0 && part_type != 0xEE && num_sectors > 0 && start_lba >= 1 {
                    let start_byte = start_lba * 512;
                    let size_bytes = num_sectors * 512;
                    if start_byte + size_bytes <= file_len {
                        let fs_type = Self::probe_filesystem(&mut file, start_byte, size_bytes)?;
                        let name = Self::describe_mbr_type(part_type);

                        partitions.push(DiskPartition {
                            index: part_idx,
                            name,
                            start_byte,
                            size_bytes,
                            fs_type,
                            table_type: "MBR".to_string(),
                            type_str: format!("0x{:02X}", part_type),
                            bootable: status == 0x80,
                        });
                        part_idx += 1;
                    }
                }
            }
        }

        // 3. If still no partitions, probe raw unpartitioned filesystem at offset 0
        if partitions.is_empty() {
            let raw_fs = Self::probe_filesystem(&mut file, 0, file_len)?;
            if raw_fs != DiskFsType::Unknown {
                partitions.push(DiskPartition {
                    index: 1,
                    name: format!("Raw {}", raw_fs.display_name()),
                    start_byte: 0,
                    size_bytes: file_len,
                    fs_type: raw_fs,
                    table_type: "Raw".to_string(),
                    type_str: "Unpartitioned".to_string(),
                    bootable: false,
                });
            }
        }

        Ok(partitions)
    }

    /// Probes filesystem header signatures at a specific partition byte range
    pub fn probe_filesystem(file: &mut File, start_byte: u64, size_bytes: u64) -> Result<DiskFsType, std::io::Error> {
        let file_len = file.metadata()?.len();
        if start_byte >= file_len {
            return Ok(DiskFsType::Unknown);
        }

        // Check FAT (FAT12, FAT16, FAT32)
        let mut boot_sector = [0u8; 512];
        file.seek(SeekFrom::Start(start_byte))?;
        if file.read_exact(&mut boot_sector).is_ok() && boot_sector[510..512] == [0x55, 0xAA] {
            let bytes_per_sec = u16::from_le_bytes(boot_sector[11..13].try_into().unwrap_or([0; 2]));
            let sec_per_clus = boot_sector[13];
            let reserved_sec = u16::from_le_bytes(boot_sector[14..16].try_into().unwrap_or([0; 2]));
            let num_fats = boot_sector[16];

            if (bytes_per_sec == 512 || bytes_per_sec == 1024 || bytes_per_sec == 2048 || bytes_per_sec == 4096)
                && sec_per_clus > 0
                && (sec_per_clus & (sec_per_clus - 1)) == 0
                && reserved_sec > 0
                && num_fats > 0
            {
                let fat12_16_type = String::from_utf8_lossy(&boot_sector[54..62]);
                let fat32_type = String::from_utf8_lossy(&boot_sector[82..90]);

                if fat32_type.starts_with("FAT32") {
                    return Ok(DiskFsType::Fat32);
                } else if fat12_16_type.starts_with("FAT16") {
                    return Ok(DiskFsType::Fat16);
                } else if fat12_16_type.starts_with("FAT12") {
                    return Ok(DiskFsType::Fat12);
                } else {
                    // Fallback to cluster calculation
                    let total_sec_16 = u16::from_le_bytes(boot_sector[19..21].try_into().unwrap_or([0; 2])) as u32;
                    let total_sec_32 = u32::from_le_bytes(boot_sector[32..36].try_into().unwrap_or([0; 4]));
                    let total_sec = if total_sec_16 != 0 { total_sec_16 } else { total_sec_32 };
                    let root_entries = u16::from_le_bytes(boot_sector[17..19].try_into().unwrap_or([0; 2])) as u32;
                    let fat_size_16 = u16::from_le_bytes(boot_sector[22..24].try_into().unwrap_or([0; 2])) as u32;
                    let fat_size_32 = u32::from_le_bytes(boot_sector[36..40].try_into().unwrap_or([0; 4]));
                    let fat_size = if fat_size_16 != 0 { fat_size_16 } else { fat_size_32 };

                    let root_dir_sec = ((root_entries * 32) + (bytes_per_sec as u32 - 1)) / bytes_per_sec as u32;
                    let data_sec = total_sec.saturating_sub(reserved_sec as u32 + (num_fats as u32 * fat_size) + root_dir_sec);
                    let count_of_clusters = data_sec / sec_per_clus as u32;

                    if count_of_clusters < 4085 {
                        return Ok(DiskFsType::Fat12);
                    } else if count_of_clusters < 65525 {
                        return Ok(DiskFsType::Fat16);
                    } else {
                        return Ok(DiskFsType::Fat32);
                    }
                }
            }
        }

        // Check Ext4 / Ext3 / Ext2 (Superblock at offset 1024)
        if size_bytes >= 2048 {
            let mut super_block = [0u8; 1024];
            file.seek(SeekFrom::Start(start_byte + 1024))?;
            if file.read_exact(&mut super_block).is_ok() {
                let magic = u16::from_le_bytes(super_block[0x38..0x3A].try_into().unwrap_or([0; 2]));
                if magic == 0xEF53 {
                    return Ok(DiskFsType::Ext4);
                }
            }
        }

        // Check SquashFS (Magic "hsqs" at start)
        let mut sqsh_header = [0u8; 4];
        file.seek(SeekFrom::Start(start_byte))?;
        if file.read_exact(&mut sqsh_header).is_ok() && &sqsh_header == b"hsqs" {
            return Ok(DiskFsType::Squashfs);
        }

        // Check ISO 9660 ("CD001" at offset 0x8000)
        if size_bytes >= 0x8000 + 2048 {
            let mut iso_pvd = [0u8; 6];
            file.seek(SeekFrom::Start(start_byte + 0x8000))?;
            if file.read_exact(&mut iso_pvd).is_ok() && &iso_pvd[1..6] == b"CD001" {
                return Ok(DiskFsType::Iso9660);
            }
        }

        // Check UDF ("BEA01" / "NSR02" / "NSR03" at 0x8000)
        if size_bytes >= 0x8000 + 2048 {
            let mut udf_sig = [0u8; 6];
            file.seek(SeekFrom::Start(start_byte + 0x8000))?;
            if file.read_exact(&mut udf_sig).is_ok() && (&udf_sig[1..6] == b"BEA01" || &udf_sig[1..6] == b"NSR02" || &udf_sig[1..6] == b"NSR03") {
                return Ok(DiskFsType::Udf);
            }
        }

        Ok(DiskFsType::Unknown)
    }

    /// List directory contents of a disk image partition or root
    pub fn list_image_contents(archive_path_str: &str, subpath: &str) -> Result<DirectoryListing, std::io::Error> {
        let partitions = Self::probe_image(archive_path_str)?;
        let clean_sub = subpath.trim_matches('/');

        // If no partitions or unreadable disk image
        if partitions.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Unable to find recognized partition table or filesystem in disk image",
            ));
        }

        // Multi-partition image: if clean_sub is empty, display partitions list as virtual folders
        if partitions.len() > 1 && clean_sub.is_empty() {
            let mut entries = Vec::new();
            let mut total_size = 0u64;

            for p in &partitions {
                let slug = p.slug();
                total_size += p.size_bytes;
                entries.push(FileEntry {
                    name: slug.clone(),
                    path: format!("archive://{}#{}", archive_path_str, slug),
                    is_dir: true,
                    is_symlink: false,
                    is_empty: Some(false),
                    size: p.size_bytes,
                    modified: None,
                    permissions: "drwxr-xr-x".to_string(),
                    mode_octal: "0755".to_string(),
                    owner: "partition".to_string(),
                    group: p.fs_type.display_name().to_string(),
                    uid: p.index as u32,
                    gid: p.index as u32,
                    mime_type: None,
                    is_archive: false,
                });
            }

            // Add virtual [Disk Layout.txt] summary entry
            entries.push(FileEntry {
                name: "[Disk Layout.txt]".to_string(),
                path: format!("archive://{}#[Disk Layout.txt]", archive_path_str),
                is_dir: false,
                is_symlink: false,
                is_empty: Some(false),
                size: 512,
                modified: None,
                permissions: "-rw-r--r--".to_string(),
                mode_octal: "0644".to_string(),
                owner: "disk".to_string(),
                group: "layout".to_string(),
                uid: 0,
                gid: 0,
                mime_type: Some("text/plain".to_string()),
                is_archive: false,
            });

            return Ok(DirectoryListing {
                current_path: format!("archive://{}#", archive_path_str),
                parent_path: None,
                total_files: 1,
                total_dirs: partitions.len(),
                total_size,
                protocol: "archive".to_string(),
                entries,
                is_truncated: None,
                max_limit: None,
            });
        }

        // Determine target partition and sub-inner path
        let (target_part, inner_fs_path) = if partitions.len() == 1 {
            (&partitions[0], clean_sub)
        } else {
            // Find which partition was selected (e.g. "p1-efi-fat32/EFI/BOOT" -> partition p1-efi-fat32, subpath "EFI/BOOT")
            let first_segment = clean_sub.split('/').next().unwrap_or("");
            if let Some(p) = partitions.iter().find(|p| p.slug() == first_segment) {
                let rest = clean_sub.strip_prefix(first_segment).unwrap_or("").trim_matches('/');
                (p, rest)
            } else if clean_sub == "[Disk Layout.txt]" {
                // Layout file handled in read_image_entry
                return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "Not a directory"));
            } else {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("Partition '{}' not found in disk image", first_segment),
                ));
            }
        };

        // Open filesystem on selected partition
        match target_part.fs_type {
            DiskFsType::Fat12 | DiskFsType::Fat16 | DiskFsType::Fat32 => {
                Self::list_fat_partition(archive_path_str, target_part, inner_fs_path)
            }
            DiskFsType::Ext4 => {
                Self::list_ext4_partition(archive_path_str, target_part, inner_fs_path)
            }
            DiskFsType::Iso9660 | DiskFsType::Udf => {
                super::archive::ArchiveHandler::list_iso_contents(archive_path_str, inner_fs_path)
            }
            DiskFsType::Squashfs => {
                super::archive::ArchiveHandler::list_squashfs_contents(archive_path_str, inner_fs_path)
            }
            DiskFsType::Unknown => Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                format!("Unsupported filesystem format on partition {}", target_part.name),
            )),
        }
    }

    /// List FAT12/16/32 directory contents
    fn list_fat_partition(
        archive_path_str: &str,
        part: &DiskPartition,
        inner_path: &str,
    ) -> Result<DirectoryListing, std::io::Error> {
        let file = File::open(archive_path_str)?;
        let slice = StreamSlice::new(file, part.start_byte, part.size_bytes);
        let fs = fatfs::FileSystem::new(slice, fatfs::FsOptions::new())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("FAT error: {}", e)))?;

        let root_dir = fs.root_dir();
        let target_dir = if inner_path.is_empty() {
            root_dir
        } else {
            root_dir.open_dir(inner_path).map_err(|e| {
                std::io::Error::new(std::io::ErrorKind::NotFound, format!("Directory not found: {}", e))
            })?
        };

        let mut entries = Vec::new();
        let mut total_files = 0;
        let mut total_dirs = 0;
        let mut total_size = 0u64;

        let part_prefix = if part.table_type == "Raw" {
            String::new()
        } else {
            format!("{}/", part.slug())
        };

        for entry in target_dir.iter() {
            let entry = entry.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
            let name = entry.file_name();
            if name == "." || name == ".." {
                continue;
            }

            let is_dir = entry.is_dir();
            let size = entry.len();
            let modified = entry.modified();
            let unix_timestamp = match modified {
                fatfs::DateTime { date, time } => {
                    let dt = chrono::NaiveDate::from_ymd_opt(date.year as i32, date.month as u32, date.day as u32)
                        .and_then(|d| d.and_hms_opt(time.hour as u32, time.min as u32, time.sec as u32));
                    dt.map(|d| d.and_utc().timestamp() as u64)
                }
            };

            let entry_inner_path = if inner_path.is_empty() {
                format!("{}{}", part_prefix, name)
            } else {
                format!("{}{}/{}", part_prefix, inner_path, name)
            };

            if is_dir {
                total_dirs += 1;
            } else {
                total_files += 1;
                total_size += size;
            }

            entries.push(FileEntry {
                name: name.clone(),
                path: format!("archive://{}#{}", archive_path_str, entry_inner_path),
                is_dir,
                is_symlink: false,
                is_empty: None,
                size,
                modified: unix_timestamp,
                permissions: if is_dir { "drwxr-xr-x".to_string() } else { "-rw-r--r--".to_string() },
                mode_octal: if is_dir { "0755".to_string() } else { "0644".to_string() },
                owner: "fat".to_string(),
                group: part.fs_type.display_name().to_string(),
                uid: 1000,
                gid: 1000,
                mime_type: if is_dir { None } else { Some(mime_guess::from_path(&name).first_or_octet_stream().to_string()) },
                is_archive: false,
            });
        }

        let cur_sub = if inner_path.is_empty() {
            if part.table_type == "Raw" { "".to_string() } else { part.slug() }
        } else {
            if part.table_type == "Raw" { inner_path.to_string() } else { format!("{}/{}", part.slug(), inner_path) }
        };

        Ok(DirectoryListing {
            current_path: format!("archive://{}#{}", archive_path_str, cur_sub),
            parent_path: None,
            total_files,
            total_dirs,
            total_size,
            protocol: "archive".to_string(),
            entries,
            is_truncated: None,
            max_limit: None,
        })
    }

    /// List Ext2/3/4 directory contents
    fn list_ext4_partition(
        archive_path_str: &str,
        part: &DiskPartition,
        inner_path: &str,
    ) -> Result<DirectoryListing, std::io::Error> {
        let file = File::open(archive_path_str)?;
        let slice = positioned_io::Slice::new(file, part.start_byte, Some(part.size_bytes));
        let superblock = ext4::SuperBlock::new(slice)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("Ext4 error: {}", e)))?;

        let clean_path = if inner_path.is_empty() {
            "/".to_string()
        } else if !inner_path.starts_with('/') {
            format!("/{}", inner_path)
        } else {
            inner_path.to_string()
        };

        let target_dir_entry = superblock.resolve_path(&clean_path)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::NotFound, format!("Ext4 path resolve error: {}", e)))?;
        let inode = superblock.load_inode(target_dir_entry.inode)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("Ext4 load inode error: {}", e)))?;

        let enhanced = superblock.enhance(&inode)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("Ext4 enhance error: {}", e)))?;

        let dir_entries = match enhanced {
            ext4::Enhanced::Directory(entries) => entries,
            _ => return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "Target Ext4 entry is not a directory")),
        };

        let mut entries = Vec::new();
        let mut total_files = 0;
        let mut total_dirs = 0;
        let mut total_size = 0u64;

        let part_prefix = if part.table_type == "Raw" {
            String::new()
        } else {
            format!("{}/", part.slug())
        };

        for item in dir_entries {
            let name = item.name;
            if name == "." || name == ".." || name == "lost+found" && inner_path.is_empty() {
                continue;
            }

            let item_inode = match superblock.load_inode(item.inode) {
                Ok(inod) => inod,
                Err(_) => continue,
            };

            let is_dir = item_inode.stat.extracted_type == ext4::FileType::Directory;
            let is_symlink = item_inode.stat.extracted_type == ext4::FileType::SymbolicLink;
            let size = item_inode.stat.size;
            let modified = Some(item_inode.stat.mtime.epoch_secs as u64);
            let mode = item_inode.stat.file_mode;

            let entry_inner_path = if inner_path.is_empty() {
                format!("{}{}", part_prefix, name)
            } else {
                format!("{}{}/{}", part_prefix, inner_path, name)
            };

            if is_dir {
                total_dirs += 1;
            } else {
                total_files += 1;
                total_size += size;
            }

            entries.push(FileEntry {
                name: name.clone(),
                path: format!("archive://{}#{}", archive_path_str, entry_inner_path),
                is_dir,
                is_symlink,
                is_empty: None,
                size,
                modified,
                permissions: format_mode_octal_permissions(mode, is_dir, is_symlink),
                mode_octal: format!("{:04o}", mode & 0o7777),
                owner: format!("{}", item_inode.stat.uid),
                group: format!("{}", item_inode.stat.gid),
                uid: item_inode.stat.uid,
                gid: item_inode.stat.gid,
                mime_type: if is_dir { None } else { Some(mime_guess::from_path(&name).first_or_octet_stream().to_string()) },
                is_archive: false,
            });
        }

        let cur_sub = if inner_path.is_empty() {
            if part.table_type == "Raw" { "".to_string() } else { part.slug() }
        } else {
            if part.table_type == "Raw" { inner_path.to_string() } else { format!("{}/{}", part.slug(), inner_path) }
        };

        Ok(DirectoryListing {
            current_path: format!("archive://{}#{}", archive_path_str, cur_sub),
            parent_path: None,
            total_files,
            total_dirs,
            total_size,
            protocol: "archive".to_string(),
            entries,
            is_truncated: None,
            max_limit: None,
        })
    }

    /// Read a specific file entry from a disk image partition
    pub fn read_image_entry(
        archive_path_str: &str,
        inner_path: &str,
        max_bytes: Option<usize>,
    ) -> Result<FileContentResponse, std::io::Error> {
        let clean_path = inner_path.trim_matches('/');

        // Special virtual file: [Disk Layout.txt]
        if clean_path == "[Disk Layout.txt]" {
            let partitions = Self::probe_image(archive_path_str)?;
            let mut report = format!("=== BRUM DISK IMAGE INSPECTION REPORT ===\nImage File: {}\nTotal Partitions: {}\n\n", archive_path_str, partitions.len());
            for p in &partitions {
                report += &format!(
                    "Partition #{}: {}\n  Table Scheme : {}\n  Type / GUID  : {}\n  Filesystem   : {}\n  Start Offset : {} bytes (LBA {})\n  Total Size   : {} ({})\n  Bootable     : {}\n\n",
                    p.index,
                    p.name,
                    p.table_type,
                    p.type_str,
                    p.fs_type.display_name(),
                    p.start_byte,
                    p.start_byte / 512,
                    format_bytes(p.size_bytes),
                    p.size_bytes,
                    if p.bootable { "YES [Active MBR]" } else { "No" }
                );
            }
            let bytes = report.as_bytes();
            return Ok(FileContentResponse {
                path: format!("archive://{}#[Disk Layout.txt]", archive_path_str),
                name: "[Disk Layout.txt]".to_string(),
                content: String::from_utf8_lossy(bytes).to_string(),
                is_binary: false,
                size: bytes.len() as u64,
                mime_type: "text/plain".to_string(),
            });
        }

        let partitions = Self::probe_image(archive_path_str)?;
        let (target_part, file_subpath) = if partitions.len() == 1 {
            (&partitions[0], clean_path)
        } else {
            let first_segment = clean_path.split('/').next().unwrap_or("");
            if let Some(p) = partitions.iter().find(|p| p.slug() == first_segment) {
                let rest = clean_path.strip_prefix(first_segment).unwrap_or("").trim_matches('/');
                (p, rest)
            } else {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("Partition not found for path '{}'", clean_path),
                ));
            }
        };

        match target_part.fs_type {
            DiskFsType::Fat12 | DiskFsType::Fat16 | DiskFsType::Fat32 => {
                let file = File::open(archive_path_str)?;
                let slice = StreamSlice::new(file, target_part.start_byte, target_part.size_bytes);
                let fs = fatfs::FileSystem::new(slice, fatfs::FsOptions::new())
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("FAT error: {}", e)))?;
                let mut fat_file = fs.root_dir().open_file(file_subpath)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::NotFound, format!("FAT file open error: {}", e)))?;

                let limit = max_bytes.unwrap_or(10 * 1024 * 1024);
                let mut buf = Vec::new();
                let mut chunk = vec![0u8; 64 * 1024];
                while buf.len() < limit {
                    let to_read = chunk.len().min(limit - buf.len());
                    let n = fat_file.read(&mut chunk[..to_read])?;
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                }

                let is_binary = buf.iter().take(4096).any(|&b| b == 0);
                let file_name = file_subpath.split('/').filter(|s| !s.is_empty()).last().unwrap_or("file");
                let mime = mime_guess::from_path(file_name).first_or_octet_stream().to_string();

                let content = if is_binary {
                    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &buf)
                } else {
                    String::from_utf8_lossy(&buf).to_string()
                };

                Ok(FileContentResponse {
                    path: format!("archive://{}#{}", archive_path_str, clean_path),
                    name: file_name.to_string(),
                    content,
                    is_binary,
                    size: buf.len() as u64,
                    mime_type: mime,
                })
            }
            DiskFsType::Ext4 => {
                let file = File::open(archive_path_str)?;
                let slice = positioned_io::Slice::new(file, target_part.start_byte, Some(target_part.size_bytes));
                let superblock = ext4::SuperBlock::new(slice)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("Ext4 error: {}", e)))?;

                let clean_ext_path = if !file_subpath.starts_with('/') {
                    format!("/{}", file_subpath)
                } else {
                    file_subpath.to_string()
                };

                let entry = superblock.resolve_path(&clean_ext_path)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::NotFound, format!("Ext4 resolve error: {}", e)))?;
                let inode = superblock.load_inode(entry.inode)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("Ext4 inode error: {}", e)))?;
                let mut reader = superblock.open(&inode)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("Ext4 open error: {}", e)))?;

                let limit = max_bytes.unwrap_or(10 * 1024 * 1024);
                let mut buf = Vec::new();
                let mut chunk = vec![0u8; 64 * 1024];
                while buf.len() < limit {
                    let to_read = chunk.len().min(limit - buf.len());
                    let n = reader.read(&mut chunk[..to_read])?;
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                }

                let is_binary = buf.iter().take(4096).any(|&b| b == 0);
                let file_name = file_subpath.split('/').filter(|s| !s.is_empty()).last().unwrap_or("file");
                let mime = mime_guess::from_path(file_name).first_or_octet_stream().to_string();

                let content = if is_binary {
                    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &buf)
                } else {
                    String::from_utf8_lossy(&buf).to_string()
                };

                Ok(FileContentResponse {
                    path: format!("archive://{}#{}", archive_path_str, clean_path),
                    name: file_name.to_string(),
                    content,
                    is_binary,
                    size: buf.len() as u64,
                    mime_type: mime,
                })
            }
            DiskFsType::Iso9660 | DiskFsType::Udf => {
                super::archive::ArchiveHandler::read_iso_entry(archive_path_str, file_subpath, max_bytes.unwrap_or(10 * 1024 * 1024))
            }
            DiskFsType::Squashfs => {
                super::archive::ArchiveHandler::read_squashfs_entry(archive_path_str, file_subpath, max_bytes.unwrap_or(10 * 1024 * 1024))
            }
            DiskFsType::Unknown => Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                format!("Unsupported filesystem format on partition {}", target_part.name),
            )),
        }
    }

    /// Extract all partitions and files from a disk image to a target directory
    pub fn extract_image(archive_path_str: &str, target_dir_str: &str) -> Result<(), std::io::Error> {
        let partitions = Self::probe_image(archive_path_str)?;
        let target_dir = Path::new(target_dir_str);
        std::fs::create_dir_all(target_dir)?;

        for part in &partitions {
            let part_out_dir = if partitions.len() == 1 {
                target_dir.to_path_buf()
            } else {
                target_dir.join(part.slug())
            };
            std::fs::create_dir_all(&part_out_dir)?;

            match part.fs_type {
                DiskFsType::Fat12 | DiskFsType::Fat16 | DiskFsType::Fat32 => {
                    let file = File::open(archive_path_str)?;
                    let slice = StreamSlice::new(file, part.start_byte, part.size_bytes);
                    if let Ok(fs) = fatfs::FileSystem::new(slice, fatfs::FsOptions::new()) {
                        Self::extract_fat_dir(&fs.root_dir(), &part_out_dir)?;
                    }
                }
                DiskFsType::Ext4 => {
                    let file = File::open(archive_path_str)?;
                    let slice = positioned_io::Slice::new(file, part.start_byte, Some(part.size_bytes));
                    if let Ok(superblock) = ext4::SuperBlock::new(slice) {
                        if let Ok(root_entry) = superblock.resolve_path("/") {
                            if let Ok(root_inode) = superblock.load_inode(root_entry.inode) {
                                Self::extract_ext4_dir(&superblock, &root_inode, &part_out_dir)?;
                            }
                        }
                    }
                }
                DiskFsType::Iso9660 | DiskFsType::Udf => {
                    let _ = super::archive::ArchiveHandler::extract_archive(archive_path_str, part_out_dir.to_str().unwrap_or(""));
                }
                DiskFsType::Squashfs => {
                    let _ = super::archive::ArchiveHandler::extract_archive(archive_path_str, part_out_dir.to_str().unwrap_or(""));
                }
                _ => {}
            }
        }

        info!("Extracted disk image {} to {}", archive_path_str, target_dir_str);
        Ok(())
    }

    fn extract_fat_dir<IO: Read + Seek + std::io::Write>(
        dir: &fatfs::Dir<IO>,
        out_dir: &Path,
    ) -> Result<(), std::io::Error> {
        for entry_res in dir.iter() {
            let entry = entry_res.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
            let name = entry.file_name();
            if name == "." || name == ".." {
                continue;
            }
            let child_out = out_dir.join(&name);
            if entry.is_dir() {
                std::fs::create_dir_all(&child_out)?;
                let sub_dir = entry.to_dir();
                Self::extract_fat_dir(&sub_dir, &child_out)?;
            } else if entry.is_file() {
                if let Some(parent) = child_out.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let mut in_file = entry.to_file();
                let mut out_file = File::create(&child_out)?;
                std::io::copy(&mut in_file, &mut out_file)?;
            }
        }
        Ok(())
    }

    fn extract_ext4_dir<R: positioned_io::ReadAt>(
        sb: &ext4::SuperBlock<R>,
        inode: &ext4::Inode,
        out_dir: &Path,
    ) -> Result<(), std::io::Error> {
        let enhanced = sb.enhance(inode)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("Ext4 enhance: {}", e)))?;
        if let ext4::Enhanced::Directory(entries) = enhanced {
            for entry in entries {
                let name = entry.name;
                if name == "." || name == ".." {
                    continue;
                }
                let child_out = out_dir.join(&name);
                if let Ok(child_inode) = sb.load_inode(entry.inode) {
                    if child_inode.stat.extracted_type == ext4::FileType::Directory {
                        std::fs::create_dir_all(&child_out)?;
                        Self::extract_ext4_dir(sb, &child_inode, &child_out)?;
                    } else if child_inode.stat.extracted_type == ext4::FileType::RegularFile {
                        if let Some(parent) = child_out.parent() {
                            std::fs::create_dir_all(parent)?;
                        }
                        if let Ok(mut in_file) = sb.open(&child_inode) {
                            let mut out_file = File::create(&child_out)?;
                            std::io::copy(&mut in_file, &mut out_file)?;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn describe_mbr_type(t: u8) -> String {
        match t {
            0x01 => "FAT12".to_string(),
            0x04 => "FAT16 (<32MB)".to_string(),
            0x05 => "Extended (CHS)".to_string(),
            0x06 => "FAT16 (>32MB)".to_string(),
            0x07 => "NTFS / exFAT".to_string(),
            0x0B => "FAT32 (CHS)".to_string(),
            0x0C => "FAT32 (LBA)".to_string(),
            0x0E => "FAT16 (LBA)".to_string(),
            0x0F => "Extended (LBA)".to_string(),
            0x82 => "Linux swap".to_string(),
            0x83 => "Linux filesystem".to_string(),
            0xEF => "EFI System Partition".to_string(),
            _ => format!("MBR Partition (0x{:02X})", t),
        }
    }

    fn describe_gpt_guid(guid: &[u8]) -> String {
        let hex = format!("{:02X}{:02X}{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
            guid[3], guid[2], guid[1], guid[0],
            guid[5], guid[4],
            guid[7], guid[6],
            guid[8], guid[9],
            guid[10], guid[11], guid[12], guid[13], guid[14], guid[15]
        );
        match hex.as_str() {
            "C12A7328-F81F-11D2-BA4B-00A0C93EC93B" => "EFI System".to_string(),
            "0FC63DAF-8483-4772-8E79-3D69D8477DE4" => "Linux filesystem".to_string(),
            "EBD0A0A2-B9E5-4433-87C0-68B6B72699C7" => "Microsoft Basic Data".to_string(),
            "4F68BCE3-E8CD-4DB1-96E7-FBCAF984B709" => "Linux Root (x86_64)".to_string(),
            "933AC7E1-2EB4-4F13-B844-0E14E2AEF0F5" => "Linux Home".to_string(),
            "0657FD6D-A4AB-43C4-84E5-0933C84B4F4F" => "Linux Swap".to_string(),
            _ => "Data Partition".to_string(),
        }
    }
}

fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

fn format_mode_octal_permissions(mode: u16, is_dir: bool, is_symlink: bool) -> String {
    let prefix = if is_dir {
        'd'
    } else if is_symlink {
        'l'
    } else {
        '-'
    };
    let r1 = if mode & 0o400 != 0 { 'r' } else { '-' };
    let w1 = if mode & 0o200 != 0 { 'w' } else { '-' };
    let x1 = if mode & 0o100 != 0 { 'x' } else { '-' };
    let r2 = if mode & 0o040 != 0 { 'r' } else { '-' };
    let w2 = if mode & 0o020 != 0 { 'w' } else { '-' };
    let x2 = if mode & 0o010 != 0 { 'x' } else { '-' };
    let r3 = if mode & 0o004 != 0 { 'r' } else { '-' };
    let w3 = if mode & 0o002 != 0 { 'w' } else { '-' };
    let x3 = if mode & 0o001 != 0 { 'x' } else { '-' };
    format!("{}{}{}{}{}{}{}{}{}{}", prefix, r1, w1, x1, r2, w2, x2, r3, w3, x3)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_mbr_fat16_probing_and_reading() {
        let mut tmp = NamedTempFile::new().unwrap();

        // Construct 2MB disk image with MBR partition table
        let mut disk = vec![0u8; 2 * 1024 * 1024];

        // MBR Signature
        disk[510] = 0x55;
        disk[511] = 0xAA;

        // Partition 1 at LBA 1 (offset 512), 2000 sectors (~1MB), Type 0x06 (FAT16)
        let off1 = 446;
        disk[off1] = 0x80; // bootable
        disk[off1 + 4] = 0x06; // FAT16
        disk[off1 + 8..off1 + 12].copy_from_slice(&1u32.to_le_bytes()); // start LBA 1
        disk[off1 + 12..off1 + 16].copy_from_slice(&2000u32.to_le_bytes()); // 2000 sectors

        // Partition 2 at LBA 2001 (offset 2001*512), 1500 sectors, Type 0x83 (Linux)
        let off2 = 446 + 16;
        disk[off2] = 0x00;
        disk[off2 + 4] = 0x83; // Linux filesystem
        disk[off2 + 8..off2 + 12].copy_from_slice(&2001u32.to_le_bytes());
        disk[off2 + 12..off2 + 16].copy_from_slice(&1500u32.to_le_bytes());

        tmp.write_all(&disk).unwrap();
        tmp.flush().unwrap();

        let path = tmp.path().to_str().unwrap();
        let partitions = DiskImageHandler::probe_image(path).unwrap();
        assert_eq!(partitions.len(), 2);
        assert_eq!(partitions[0].index, 1);
        assert_eq!(partitions[0].start_byte, 512);
        assert_eq!(partitions[0].size_bytes, 2000 * 512);
        assert_eq!(partitions[0].table_type, "MBR");
        assert!(partitions[0].bootable);

        assert_eq!(partitions[1].index, 2);
        assert_eq!(partitions[1].start_byte, 2001 * 512);
        assert_eq!(partitions[1].size_bytes, 1500 * 512);
        assert_eq!(partitions[1].table_type, "MBR");
        assert!(!partitions[1].bootable);

        // Test multi-partition container root listing
        let listing = DiskImageHandler::list_image_contents(path, "").unwrap();
        assert_eq!(listing.entries.len(), 3); // 2 partitions + [Disk Layout.txt]
        assert!(listing.entries.iter().any(|e| e.name == "p1-fat16-32mb"));
        assert!(listing.entries.iter().any(|e| e.name == "p2-linux-filesystem"));
        assert!(listing.entries.iter().any(|e| e.name == "[Disk Layout.txt]"));

        // Test reading [Disk Layout.txt]
        let layout_resp = DiskImageHandler::read_image_entry(path, "[Disk Layout.txt]", None).unwrap();
        assert!(!layout_resp.is_binary);
        assert!(layout_resp.content.contains("BRUM DISK IMAGE INSPECTION REPORT"));
        assert!(layout_resp.content.contains("Total Partitions: 2"));
        assert!(layout_resp.content.contains("YES [Active MBR]"));
    }

    #[test]
    fn test_gpt_probing() {
        let mut tmp = NamedTempFile::new().unwrap();
        let mut disk = vec![0u8; 2 * 1024 * 1024];

        // Sector 1: GPT Header
        let s1 = 512;
        disk[s1..s1 + 8].copy_from_slice(b"EFI PART");
        disk[s1 + 80..s1 + 84].copy_from_slice(&2u32.to_le_bytes()); // num_entries = 2
        disk[s1 + 84..s1 + 88].copy_from_slice(&128u32.to_le_bytes()); // entry_size = 128
        disk[s1 + 72..s1 + 80].copy_from_slice(&2u64.to_le_bytes()); // part_entry_lba = 2 (offset 1024)

        // Sector 2 (LBA 2, offset 1024): Partition Entry 1 (EFI System)
        let e1 = 1024;
        // EFI System GUID: C12A7328-F81F-11D2-BA4B-00A0C93EC93B
        disk[e1..e1 + 16].copy_from_slice(&[
            0x28, 0x73, 0x2A, 0xC1, 0x1F, 0xF8, 0xD2, 0x11,
            0xBA, 0x4B, 0x00, 0xA0, 0xC9, 0x3E, 0xC9, 0x3B
        ]);
        disk[e1 + 32..e1 + 40].copy_from_slice(&2048u64.to_le_bytes()); // First LBA 2048
        disk[e1 + 40..e1 + 48].copy_from_slice(&2047u64.wrapping_add(100).to_le_bytes()); // Last LBA 2147

        // Name: "EFI" (UTF-16LE: 'E', 0, 'F', 0, 'I', 0)
        disk[e1 + 56..e1 + 62].copy_from_slice(&[b'E', 0, b'F', 0, b'I', 0]);

        tmp.write_all(&disk).unwrap();
        tmp.flush().unwrap();

        let path = tmp.path().to_str().unwrap();
        let partitions = DiskImageHandler::probe_image(path).unwrap();
        assert_eq!(partitions.len(), 1);
        assert_eq!(partitions[0].name, "EFI");
        assert_eq!(partitions[0].table_type, "GPT");
        assert_eq!(partitions[0].start_byte, 2048 * 512);
        assert_eq!(partitions[0].size_bytes, 100 * 512);
    }

    #[test]
    fn test_fat_filesystem_read_lifecycle() {
        let mut tmp = NamedTempFile::new().unwrap();
        // Format 1.44MB floppy FAT12 image
        let mut cursor = std::io::Cursor::new(vec![0u8; 1440 * 1024]);
        fatfs::format_volume(&mut cursor, fatfs::FormatVolumeOptions::new()).unwrap();
        let buf = cursor.into_inner();
        tmp.write_all(&buf).unwrap();
        tmp.flush().unwrap();

        // Write a test file into the FAT image
        {
            let file = std::fs::OpenOptions::new().read(true).write(true).open(tmp.path()).unwrap();
            let fs = fatfs::FileSystem::new(file, fatfs::FsOptions::new()).unwrap();
            let root = fs.root_dir();
            root.create_dir("DOCS").unwrap();
            let mut f = root.create_file("HELLO.TXT").unwrap();
            f.write_all(b"Hello from Brum Raw FAT Image!").unwrap();
        }

        let path = tmp.path().to_str().unwrap();
        let partitions = DiskImageHandler::probe_image(path).unwrap();
        assert_eq!(partitions.len(), 1);
        assert_eq!(partitions[0].table_type, "Raw");

        // List root
        let listing = DiskImageHandler::list_image_contents(path, "").unwrap();
        assert!(listing.entries.iter().any(|e| e.name == "HELLO.TXT" && !e.is_dir));
        assert!(listing.entries.iter().any(|e| e.name == "DOCS" && e.is_dir));

        // Read file
        let resp = DiskImageHandler::read_image_entry(path, "HELLO.TXT", None).unwrap();
        assert_eq!(resp.content, "Hello from Brum Raw FAT Image!");
    }
}
