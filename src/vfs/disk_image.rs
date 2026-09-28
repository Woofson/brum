use super::{DirectoryListing, FileContentResponse, FileEntry};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
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
    Ntfs,
    Partclone,
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
            DiskFsType::Ntfs => "NTFS",
            DiskFsType::Partclone => "Partclone Stream",
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
    pub table_type: String, // "MBR", "GPT", "Raw", "FOG", "Container"
    pub type_str: String,
    pub bootable: bool,
    pub container_subpath: Option<String>,
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

        let stripped_name = clean_name
            .strip_prefix("dev-")
            .or_else(|| clean_name.strip_prefix("dev"))
            .unwrap_or(&clean_name);

        if self.fs_type != DiskFsType::Unknown {
            let fs_slug = match self.fs_type {
                DiskFsType::Fat12 => "fat12",
                DiskFsType::Fat16 => "fat16",
                DiskFsType::Fat32 => "fat32",
                DiskFsType::Ext4 => "ext4",
                DiskFsType::Iso9660 => "iso",
                DiskFsType::Udf => "udf",
                DiskFsType::Squashfs => "squashfs",
                DiskFsType::Ntfs => "ntfs",
                DiskFsType::Partclone => "partclone",
                DiskFsType::Unknown => "raw",
            };
            if stripped_name.is_empty() || stripped_name == fs_slug {
                format!("p{}-{}", self.index, fs_slug)
            } else {
                format!("p{}-{}-{}", self.index, stripped_name, fs_slug)
            }
        } else if !stripped_name.is_empty() {
            format!("p{}-{}", self.index, stripped_name)
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

// -----------------------------------------------------------------------------
// 🌟 QCOW2 VIRTUAL DISK READER (v2 & v3)
// -----------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Qcow2Header {
    pub version: u32,
    pub backing_file_offset: u64,
    pub backing_file_size: u32,
    pub cluster_bits: u32,
    pub size: u64,
    pub crypt_method: u32,
    pub l1_size: u32,
    pub l1_table_offset: u64,
    pub refcount_table_offset: u64,
}

pub struct Qcow2Reader {
    file: File,
    header: Qcow2Header,
    l1_table: Vec<u64>,
    pos: u64,
}

impl Qcow2Reader {
    pub fn open(mut file: File) -> Result<Self, std::io::Error> {
        let mut header_buf = [0u8; 72];
        file.seek(SeekFrom::Start(0))?;
        file.read_exact(&mut header_buf)?;

        if &header_buf[0..4] != b"QFI\xFB" {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "Not a QCOW2 image"));
        }

        let version = u32::from_be_bytes(header_buf[4..8].try_into().unwrap());
        if version != 2 && version != 3 {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("Unsupported QCOW2 version {}", version)));
        }

        let backing_file_offset = u64::from_be_bytes(header_buf[8..16].try_into().unwrap());
        let backing_file_size = u32::from_be_bytes(header_buf[16..20].try_into().unwrap());
        let cluster_bits = u32::from_be_bytes(header_buf[20..24].try_into().unwrap());
        let size = u64::from_be_bytes(header_buf[24..32].try_into().unwrap());
        let crypt_method = u32::from_be_bytes(header_buf[32..36].try_into().unwrap());
        let l1_size = u32::from_be_bytes(header_buf[36..40].try_into().unwrap());
        let l1_table_offset = u64::from_be_bytes(header_buf[40..48].try_into().unwrap());
        let refcount_table_offset = u64::from_be_bytes(header_buf[48..56].try_into().unwrap());

        if cluster_bits < 9 || cluster_bits > 28 {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid cluster_bits in QCOW2 header"));
        }

        // Read L1 table
        let mut l1_table = Vec::with_capacity(l1_size as usize);
        file.seek(SeekFrom::Start(l1_table_offset))?;
        let mut l1_raw = vec![0u8; (l1_size as usize) * 8];
        file.read_exact(&mut l1_raw)?;
        for chunk in l1_raw.chunks_exact(8) {
            l1_table.push(u64::from_be_bytes(chunk.try_into().unwrap()));
        }

        Ok(Self {
            file,
            header: Qcow2Header {
                version,
                backing_file_offset,
                backing_file_size,
                cluster_bits,
                size,
                crypt_method,
                l1_size,
                l1_table_offset,
                refcount_table_offset,
            },
            l1_table,
            pos: 0,
        })
    }

    pub fn virtual_size(&self) -> u64 {
        self.header.size
    }

    pub fn read_at_pos(&mut self, start_pos: u64, buf: &mut [u8]) -> std::io::Result<usize> {
        if start_pos >= self.header.size {
            return Ok(0);
        }

        let cluster_size = 1u64 << self.header.cluster_bits;
        let l2_entries = cluster_size / 8;
        let to_read = (self.header.size - start_pos).min(buf.len() as u64) as usize;

        let mut read_bytes = 0;
        while read_bytes < to_read {
            let cur_vpos = start_pos + (read_bytes as u64);
            let cluster_idx = cur_vpos / cluster_size;
            let l1_idx = (cluster_idx / l2_entries) as usize;
            let l2_idx = (cluster_idx % l2_entries) as usize;
            let offset_in_cluster = cur_vpos % cluster_size;
            let chunk_len = ((cluster_size - offset_in_cluster) as usize).min(to_read - read_bytes);

            if l1_idx >= self.l1_table.len() || self.l1_table[l1_idx] == 0 {
                // Sparse unallocated cluster -> zero filled
                buf[read_bytes..read_bytes + chunk_len].fill(0);
            } else {
                let l2_offset = self.l1_table[l1_idx] & 0x00fffffffffffe00;
                let mut l2_entry_buf = [0u8; 8];
                self.file.seek(SeekFrom::Start(l2_offset + (l2_idx as u64 * 8)))?;
                self.file.read_exact(&mut l2_entry_buf)?;
                let l2_entry = u64::from_be_bytes(l2_entry_buf);

                let host_offset = l2_entry & 0x00fffffffffffe00;
                let is_zero = (l2_entry & 1) != 0; // Standard v3 zero flag

                if host_offset == 0 || is_zero {
                    buf[read_bytes..read_bytes + chunk_len].fill(0);
                } else {
                    self.file.seek(SeekFrom::Start(host_offset + offset_in_cluster))?;
                    self.file.read_exact(&mut buf[read_bytes..read_bytes + chunk_len])?;
                }
            }

            read_bytes += chunk_len;
        }

        Ok(read_bytes)
    }
}

impl Read for Qcow2Reader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.read_at_pos(self.pos, buf)?;
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for Qcow2Reader {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let new_pos = match pos {
            SeekFrom::Start(off) => off as i64,
            SeekFrom::Current(off) => (self.pos as i64) + off,
            SeekFrom::End(off) => (self.header.size as i64) + off,
        };
        if new_pos < 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "Negative seek in QCOW2"));
        }
        self.pos = (new_pos as u64).min(self.header.size);
        Ok(self.pos)
    }
}

impl std::io::Write for Qcow2Reader {
    fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "Read-only QCOW2"))
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// 🌟 VMWARE VMDK VIRTUAL DISK READER (Sparse & Descriptor)
// -----------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct VmdkHeader {
    pub version: u32,
    pub flags: u32,
    pub capacity: u64, // In 512-byte sectors
    pub grain_size: u64, // In 512-byte sectors (typically 128 = 64KB)
    pub descriptor_offset: u64,
    pub descriptor_size: u64,
    pub num_gtes_per_gt: u32,
    pub rgd_offset: u64,
    pub gd_offset: u64,
    pub over_head: u64,
}

pub struct VmdkReader {
    file: File,
    header: VmdkHeader,
    gd_table: Vec<u32>,
    pos: u64,
}

impl VmdkReader {
    pub fn open(mut file: File) -> Result<Self, std::io::Error> {
        let mut header_buf = [0u8; 512];
        file.seek(SeekFrom::Start(0))?;
        file.read_exact(&mut header_buf)?;

        if &header_buf[0..4] != b"KDMV" {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "Not a monolithic sparse VMDK image"));
        }

        let version = u32::from_le_bytes(header_buf[4..8].try_into().unwrap());
        let flags = u32::from_le_bytes(header_buf[8..12].try_into().unwrap());
        let capacity = u64::from_le_bytes(header_buf[12..20].try_into().unwrap());
        let grain_size = u64::from_le_bytes(header_buf[20..28].try_into().unwrap());
        let descriptor_offset = u64::from_le_bytes(header_buf[28..36].try_into().unwrap());
        let descriptor_size = u64::from_le_bytes(header_buf[36..44].try_into().unwrap());
        let num_gtes_per_gt = u32::from_le_bytes(header_buf[44..48].try_into().unwrap());
        let rgd_offset = u64::from_le_bytes(header_buf[48..56].try_into().unwrap());
        let gd_offset = u64::from_le_bytes(header_buf[56..64].try_into().unwrap());
        let over_head = u64::from_le_bytes(header_buf[64..72].try_into().unwrap());

        if grain_size == 0 || num_gtes_per_gt == 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid VMDK grain geometry"));
        }

        let total_grains = (capacity + grain_size - 1) / grain_size;
        let gd_entries = ((total_grains + num_gtes_per_gt as u64 - 1) / num_gtes_per_gt as u64) as usize;

        let mut gd_table = Vec::with_capacity(gd_entries);
        file.seek(SeekFrom::Start(gd_offset * 512))?;
        let mut gd_raw = vec![0u8; gd_entries * 4];
        file.read_exact(&mut gd_raw)?;
        for chunk in gd_raw.chunks_exact(4) {
            gd_table.push(u32::from_le_bytes(chunk.try_into().unwrap()));
        }

        Ok(Self {
            file,
            header: VmdkHeader {
                version,
                flags,
                capacity,
                grain_size,
                descriptor_offset,
                descriptor_size,
                num_gtes_per_gt,
                rgd_offset,
                gd_offset,
                over_head,
            },
            gd_table,
            pos: 0,
        })
    }

    pub fn virtual_size(&self) -> u64 {
        self.header.capacity * 512
    }

    pub fn read_at_pos(&mut self, start_pos: u64, buf: &mut [u8]) -> std::io::Result<usize> {
        let total_size = self.virtual_size();
        if start_pos >= total_size {
            return Ok(0);
        }

        let grain_size_bytes = self.header.grain_size * 512;
        let to_read = (total_size - start_pos).min(buf.len() as u64) as usize;

        let mut read_bytes = 0;
        while read_bytes < to_read {
            let cur_vpos = start_pos + (read_bytes as u64);
            let grain_idx = cur_vpos / grain_size_bytes;
            let gd_idx = (grain_idx / self.header.num_gtes_per_gt as u64) as usize;
            let gt_idx = (grain_idx % self.header.num_gtes_per_gt as u64) as usize;
            let offset_in_grain = cur_vpos % grain_size_bytes;
            let chunk_len = ((grain_size_bytes - offset_in_grain) as usize).min(to_read - read_bytes);

            if gd_idx >= self.gd_table.len() || self.gd_table[gd_idx] == 0 {
                buf[read_bytes..read_bytes + chunk_len].fill(0);
            } else {
                let gt_sector = self.gd_table[gd_idx] as u64;
                let mut gt_entry_buf = [0u8; 4];
                self.file.seek(SeekFrom::Start((gt_sector * 512) + (gt_idx as u64 * 4)))?;
                self.file.read_exact(&mut gt_entry_buf)?;
                let grain_sector = u32::from_le_bytes(gt_entry_buf) as u64;

                if grain_sector == 0 {
                    buf[read_bytes..read_bytes + chunk_len].fill(0);
                } else {
                    self.file.seek(SeekFrom::Start((grain_sector * 512) + offset_in_grain))?;
                    self.file.read_exact(&mut buf[read_bytes..read_bytes + chunk_len])?;
                }
            }

            read_bytes += chunk_len;
        }

        Ok(read_bytes)
    }
}

impl Read for VmdkReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.read_at_pos(self.pos, buf)?;
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for VmdkReader {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let total_size = self.virtual_size();
        let new_pos = match pos {
            SeekFrom::Start(off) => off as i64,
            SeekFrom::Current(off) => (self.pos as i64) + off,
            SeekFrom::End(off) => (total_size as i64) + off,
        };
        if new_pos < 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "Negative seek in VMDK"));
        }
        self.pos = (new_pos as u64).min(total_size);
        Ok(self.pos)
    }
}

impl std::io::Write for VmdkReader {
    fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "Read-only VMDK"))
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// 🌟 MICROSOFT VHD VIRTUAL DISK READER (Dynamic & Fixed)
// -----------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct VhdHeader {
    pub is_dynamic: bool,
    pub current_size: u64,
    pub block_size: u32,
    pub max_bat_entries: u32,
    pub bat_offset: u64,
}

pub struct VhdReader {
    file: File,
    header: VhdHeader,
    bat: Vec<u32>,
    pos: u64,
}

impl VhdReader {
    pub fn open(mut file: File) -> Result<Self, std::io::Error> {
        let file_len = file.metadata()?.len();
        if file_len < 512 {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "File too small for VHD"));
        }

        // Check footer at EOF - 512 or offset 0
        let mut footer = [0u8; 512];
        file.seek(SeekFrom::Start(file_len - 512))?;
        file.read_exact(&mut footer)?;

        if &footer[0..8] != b"conectix" {
            file.seek(SeekFrom::Start(0))?;
            file.read_exact(&mut footer)?;
            if &footer[0..8] != b"conectix" {
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "Not a VHD image"));
            }
        }

        let disk_type = u32::from_be_bytes(footer[60..64].try_into().unwrap());
        let current_size = u64::from_be_bytes(footer[48..56].try_into().unwrap());
        let data_offset = u64::from_be_bytes(footer[16..24].try_into().unwrap());

        if disk_type == 2 || data_offset == 0xFFFFFFFFFFFFFFFF {
            // Fixed VHD
            return Ok(Self {
                file,
                header: VhdHeader {
                    is_dynamic: false,
                    current_size,
                    block_size: 512,
                    max_bat_entries: 0,
                    bat_offset: 0,
                },
                bat: Vec::new(),
                pos: 0,
            });
        }

        // Dynamic VHD: read dynamic header at data_offset
        let mut dyn_header = [0u8; 1024];
        file.seek(SeekFrom::Start(data_offset))?;
        file.read_exact(&mut dyn_header)?;

        if &dyn_header[0..8] != b"cxsparse" {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid dynamic VHD header"));
        }

        let bat_offset = u64::from_be_bytes(dyn_header[16..24].try_into().unwrap());
        let max_bat_entries = u32::from_be_bytes(dyn_header[28..32].try_into().unwrap());
        let block_size = u32::from_be_bytes(dyn_header[32..36].try_into().unwrap());

        let mut bat = Vec::with_capacity(max_bat_entries as usize);
        file.seek(SeekFrom::Start(bat_offset))?;
        let mut bat_raw = vec![0u8; (max_bat_entries as usize) * 4];
        file.read_exact(&mut bat_raw)?;
        for chunk in bat_raw.chunks_exact(4) {
            bat.push(u32::from_be_bytes(chunk.try_into().unwrap()));
        }

        Ok(Self {
            file,
            header: VhdHeader {
                is_dynamic: true,
                current_size,
                block_size,
                max_bat_entries,
                bat_offset,
            },
            bat,
            pos: 0,
        })
    }

    pub fn virtual_size(&self) -> u64 {
        self.header.current_size
    }

    pub fn read_at_pos(&mut self, start_pos: u64, buf: &mut [u8]) -> std::io::Result<usize> {
        if !self.header.is_dynamic {
            self.file.seek(SeekFrom::Start(start_pos))?;
            return self.file.read(buf);
        }

        let total_size = self.header.current_size;
        if start_pos >= total_size {
            return Ok(0);
        }

        let block_size = self.header.block_size as u64;
        let sec_per_block = block_size / 512;
        let bitmap_secs = ((sec_per_block / 8) + 511) / 512;
        let to_read = (total_size - start_pos).min(buf.len() as u64) as usize;

        let mut read_bytes = 0;
        while read_bytes < to_read {
            let cur_vpos = start_pos + (read_bytes as u64);
            let block_idx = (cur_vpos / block_size) as usize;
            let offset_in_block = cur_vpos % block_size;
            let chunk_len = ((block_size - offset_in_block) as usize).min(to_read - read_bytes);

            if block_idx >= self.bat.len() || self.bat[block_idx] == 0xFFFFFFFF {
                buf[read_bytes..read_bytes + chunk_len].fill(0);
            } else {
                let block_sec = self.bat[block_idx] as u64;
                let host_offset = ((block_sec + bitmap_secs) * 512) + offset_in_block;
                self.file.seek(SeekFrom::Start(host_offset))?;
                self.file.read_exact(&mut buf[read_bytes..read_bytes + chunk_len])?;
            }

            read_bytes += chunk_len;
        }

        Ok(read_bytes)
    }
}

impl Read for VhdReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.read_at_pos(self.pos, buf)?;
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for VhdReader {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let total_size = self.virtual_size();
        let new_pos = match pos {
            SeekFrom::Start(off) => off as i64,
            SeekFrom::Current(off) => (self.pos as i64) + off,
            SeekFrom::End(off) => (total_size as i64) + off,
        };
        if new_pos < 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "Negative seek in VHD"));
        }
        self.pos = (new_pos as u64).min(total_size);
        Ok(self.pos)
    }
}

impl std::io::Write for VhdReader {
    fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "Read-only VHD"))
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// 🌟 PARTCLONE IMAGE STREAM & BITMAP READER
// -----------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct PartcloneHeader {
    pub magic: String,
    pub fs_name: String,
    pub device_size: u64,
    pub total_blocks: u64,
    pub used_blocks: u64,
    pub block_size: u32,
    pub crc_size: u32,
    pub bitmap: Vec<u8>,
    pub data_start_offset: u64,
}

pub enum PartcloneSource {
    File(File),
    Memory(std::io::Cursor<Vec<u8>>),
}

impl Read for PartcloneSource {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::File(f) => f.read(buf),
            Self::Memory(m) => m.read(buf),
        }
    }
}

impl Seek for PartcloneSource {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        match self {
            Self::File(f) => f.seek(pos),
            Self::Memory(m) => m.seek(pos),
        }
    }
}

struct AtomicCountingReader<R> {
    inner: R,
    bytes_read: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl<R: Read> Read for AtomicCountingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.bytes_read.fetch_add(n as u64, std::sync::atomic::Ordering::Relaxed);
        Ok(n)
    }
}

fn get_partclone_cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("brum")
        .join("partclone_cache")
}

fn get_cache_paths_for_image(source_path: &Path, file_len: u64, mtime: u64) -> (PathBuf, PathBuf) {
    use sha2::{Digest, Sha256};
    let cache_dir = get_partclone_cache_dir();
    let _ = std::fs::create_dir_all(&cache_dir);

    let mut hasher = Sha256::new();
    hasher.update(source_path.to_string_lossy().as_bytes());
    hasher.update(&file_len.to_le_bytes());
    hasher.update(&mtime.to_le_bytes());
    let hash_hex = hex::encode(hasher.finalize());
    let prefix = if hash_hex.len() >= 24 { &hash_hex[..24] } else { &hash_hex };

    let raw_path = cache_dir.join(format!("partclone_{}.raw", prefix));
    let partial_path = cache_dir.join(format!("partclone_{}.partial", prefix));
    (raw_path, partial_path)
}

pub struct PartcloneReader {
    source: PartcloneSource,
    header: PartcloneHeader,
    used_cum_sums: Vec<u64>,
    pos: u64,
}

impl PartcloneReader {
    pub fn open(file: File) -> Result<Self, std::io::Error> {
        Self::open_with_optional_path(file, None)
    }

    pub fn open_path<P: AsRef<Path>>(path: P) -> Result<Self, std::io::Error> {
        let p = path.as_ref();
        let file = File::open(p)?;
        Self::open_with_optional_path(file, Some(p))
    }

    pub fn open_with_optional_path(mut file: File, path_opt: Option<&Path>) -> Result<Self, std::io::Error> {
        let mut magic_4 = [0u8; 4];
        file.seek(SeekFrom::Start(0))?;
        let n = file.read(&mut magic_4)?;
        file.seek(SeekFrom::Start(0))?;

        let is_zstd = n >= 4 && magic_4 == [0x28, 0xB5, 0x2F, 0xFD];
        let is_gzip = n >= 2 && magic_4[0] == 0x1F && magic_4[1] == 0x8B;

        if is_zstd || is_gzip {
            let meta = file.metadata().ok();
            let file_len = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            let mtime = meta.as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);

            let (raw_path, partial_path) = if let Some(source_path) = path_opt {
                get_cache_paths_for_image(source_path, file_len, mtime)
            } else {
                let cache_dir = get_partclone_cache_dir();
                let _ = std::fs::create_dir_all(&cache_dir);
                let id = uuid::Uuid::new_v4();
                (cache_dir.join(format!("partclone_{}.raw", id)), cache_dir.join(format!("partclone_{}.partial", id)))
            };

            // If raw cache already exists and is non-empty, open directly!
            if raw_path.exists() {
                if let Ok(raw_meta) = raw_path.metadata() {
                    if raw_meta.len() > 0 {
                        if let Ok(cached_file) = File::open(&raw_path) {
                            if let Ok(reader) = Self::open_from_source(PartcloneSource::File(cached_file)) {
                                return Ok(reader);
                            }
                        }
                    }
                }
            }

            // Register task in TaskManager
            let task_mgr = crate::tools::tasks::get_global_task_manager();
            let display_name = path_opt
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .unwrap_or("compressed image");

            let task_id_opt = task_mgr.as_ref().map(|tm| {
                tm.sync_create_task(
                    &format!("Decompressing {}", display_name),
                    "decompress",
                    path_opt.map(|p| p.to_string_lossy().to_string()).as_deref().unwrap_or(display_name),
                    &raw_path.to_string_lossy(),
                    file_len,
                )
            });

            let decompress_res = (|| -> Result<File, std::io::Error> {
                let mut out_file = std::fs::OpenOptions::new()
                    .create(true)
                    .write(true)
                    .truncate(true)
                    .open(&partial_path)?;

                file.seek(SeekFrom::Start(0))?;
                let bytes_atomic = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
                let counting_file = AtomicCountingReader {
                    inner: file,
                    bytes_read: bytes_atomic.clone(),
                };

                let mut decoder: Box<dyn Read> = if is_zstd {
                    Box::new(zstd::stream::read::Decoder::new(counting_file)?)
                } else {
                    Box::new(flate2::read::GzDecoder::new(counting_file))
                };

                let mut chunk = vec![0u8; 4 * 1024 * 1024]; // 4 MB chunk
                let start_time = std::time::Instant::now();
                let mut last_update = std::time::Instant::now();

                loop {
                    let bytes_decompressed = decoder.read(&mut chunk)?;
                    if bytes_decompressed == 0 {
                        break;
                    }
                    std::io::Write::write_all(&mut out_file, &chunk[..bytes_decompressed])?;

                    let now = std::time::Instant::now();
                    if now.duration_since(last_update).as_millis() >= 250 {
                        last_update = now;
                        let elapsed_secs = start_time.elapsed().as_secs_f64();
                        let comp_bytes = bytes_atomic.load(std::sync::atomic::Ordering::Relaxed).min(file_len);
                        let speed = if elapsed_secs > 0.0 {
                            (comp_bytes as f64 / elapsed_secs) as u64
                        } else {
                            0
                        };

                        if let (Some(ref tm), Some(ref tid)) = (&task_mgr, &task_id_opt) {
                            tm.sync_update_stream_progress(
                                tid,
                                Some(display_name),
                                comp_bytes,
                                file_len,
                                0,
                                1,
                                comp_bytes,
                                speed,
                            );

                            if tm.sync_is_cancelled(tid) {
                                return Err(std::io::Error::new(
                                    std::io::ErrorKind::Interrupted,
                                    "Decompression cancelled by user",
                                ));
                            }
                        }
                    }
                }

                std::io::Write::flush(&mut out_file)?;
                drop(out_file);
                std::fs::rename(&partial_path, &raw_path)?;
                File::open(&raw_path)
            })();

            match decompress_res {
                Ok(raw_file) => {
                    if let (Some(ref tm), Some(ref tid)) = (&task_mgr, &task_id_opt) {
                        tm.sync_complete_task(tid);
                    }
                    return Self::open_from_source(PartcloneSource::File(raw_file));
                }
                Err(err) => {
                    let _ = std::fs::remove_file(&partial_path);
                    if let (Some(ref tm), Some(ref tid)) = (&task_mgr, &task_id_opt) {
                        tm.sync_fail_task(tid, &err.to_string());
                    }
                    return Err(err);
                }
            }
        }

        Self::open_from_source(PartcloneSource::File(file))
    }

    pub fn open_from_source(mut source: PartcloneSource) -> Result<Self, std::io::Error> {
        let mut magic_buf = [0u8; 16];
        source.seek(SeekFrom::Start(0))?;
        source.read_exact(&mut magic_buf)?;

        let magic_str = String::from_utf8_lossy(&magic_buf).trim_matches('\0').to_string();
        if !magic_str.starts_with("partclone-image") && !magic_str.starts_with("PARTCLONE") {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "Not a Partclone image file"));
        }

        let mut next_16 = [0u8; 16];
        source.read_exact(&mut next_16)?;
        let next_16_str = String::from_utf8_lossy(&next_16).trim_matches('\0').to_string();

        let (fs_name, device_size, total_blocks, used_blocks, block_size, crc_size, bitmap_offset) = if next_16_str.starts_with("0.") || next_16_str.starts_with("1.") {
            // Partclone v0.3.x header:
            let mut fs_raw = [0u8; 16];
            source.seek(SeekFrom::Start(0x24))?;
            source.read_exact(&mut fs_raw)?;
            let clean_fs = String::from_utf8_lossy(&fs_raw)
                .chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .collect::<String>();

            source.seek(SeekFrom::Start(0x34))?;
            let mut num_buf = [0u8; 40];
            source.read_exact(&mut num_buf)?;

            let device_size = u64::from_le_bytes(num_buf[0..8].try_into().unwrap());
            let total_blocks = u64::from_le_bytes(num_buf[8..16].try_into().unwrap());
            let used_blocks = u64::from_le_bytes(num_buf[16..24].try_into().unwrap());
            let block_size = u32::from_le_bytes(num_buf[32..36].try_into().unwrap());
            let feature_size = u32::from_le_bytes(num_buf[36..40].try_into().unwrap());

            let mut crc_size = 0u32;
            if feature_size >= 2 {
                let mut feat_head = [0u8; 2];
                source.seek(SeekFrom::Start(0x5c))?;
                source.read_exact(&mut feat_head)?;
                let crc_type = u16::from_le_bytes(feat_head);
                crc_size = match crc_type {
                    0 => 0,
                    1 => 2,
                    2 => 4,
                    3 => 8,
                    4 => 16,
                    5 => 20,
                    6 => 32,
                    7 => 64,
                    _ => 0,
                };
            }

            let bitmap_offset = 0x5c + feature_size as u64;
            (clean_fs, device_size, total_blocks, used_blocks, block_size, crc_size, bitmap_offset)
        } else {
            // Partclone v0.2.x header:
            let fs_name = next_16_str;
            let mut num_buf = [0u8; 32];
            source.read_exact(&mut num_buf)?;
            let device_size = u64::from_le_bytes(num_buf[0..8].try_into().unwrap());
            let total_blocks = u64::from_le_bytes(num_buf[8..16].try_into().unwrap());
            let used_blocks = u64::from_le_bytes(num_buf[16..24].try_into().unwrap());
            let block_size = u32::from_le_bytes(num_buf[24..28].try_into().unwrap());
            let bitmap_offset = source.stream_position()?;

            (fs_name, device_size, total_blocks, used_blocks, block_size, 0u32, bitmap_offset)
        };

        if block_size == 0 || total_blocks == 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid Partclone block size or count"));
        }

        let bitmap_len = ((total_blocks + 7) / 8) as usize;
        let mut bitmap = vec![0u8; bitmap_len];
        source.seek(SeekFrom::Start(bitmap_offset))?;
        source.read_exact(&mut bitmap)?;

        let data_start_offset = source.stream_position()? + (crc_size as u64);

        let mut used_cum_sums = Vec::with_capacity((total_blocks as usize / 64) + 2);
        let mut running_sum = 0u64;
        used_cum_sums.push(0);

        for chunk in bitmap.chunks(8) {
            let mut val = 0u64;
            for (idx, &byte) in chunk.iter().enumerate() {
                val |= (byte as u64) << (idx * 8);
            }
            running_sum += val.count_ones() as u64;
            used_cum_sums.push(running_sum);
        }

        Ok(Self {
            source,
            header: PartcloneHeader {
                magic: magic_str,
                fs_name,
                device_size,
                total_blocks,
                used_blocks,
                block_size,
                crc_size,
                bitmap,
                data_start_offset,
            },
            used_cum_sums,
            pos: 0,
        })
    }

    pub fn virtual_size(&self) -> u64 {
        self.header.device_size
    }

    pub fn get_fs_type(&self) -> DiskFsType {
        let upper = self.header.fs_name.to_uppercase();
        if upper.contains("EXT") {
            DiskFsType::Ext4
        } else if upper.contains("FAT") {
            DiskFsType::Fat32
        } else if upper.contains("NTFS") {
            DiskFsType::Ntfs
        } else {
            DiskFsType::Partclone
        }
    }

    fn get_used_block_ordinal(&self, block_idx: u64) -> Option<u64> {
        let byte_idx = (block_idx / 8) as usize;
        let bit_idx = (block_idx % 8) as u8;
        if byte_idx >= self.header.bitmap.len() {
            return None;
        }

        // Check if block bit is set
        if (self.header.bitmap[byte_idx] & (1 << bit_idx)) == 0 {
            return None; // Sparse block
        }

        let chunk64_idx = (block_idx / 64) as usize;
        let base_count = self.used_cum_sums.get(chunk64_idx).copied().unwrap_or(0);

        // Count bits from chunk64_idx * 64 up to block_idx
        let start_byte = chunk64_idx * 8;
        let mut sub_count = 0u64;
        for i in start_byte..byte_idx {
            sub_count += self.header.bitmap[i].count_ones() as u64;
        }

        let mask = (1u8 << bit_idx) - 1;
        sub_count += (self.header.bitmap[byte_idx] & mask).count_ones() as u64;

        Some(base_count + sub_count)
    }

    pub fn read_at_pos(&mut self, start_pos: u64, buf: &mut [u8]) -> std::io::Result<usize> {
        let total_size = self.header.device_size;
        if start_pos >= total_size {
            return Ok(0);
        }

        let block_size = self.header.block_size as u64;
        let to_read = (total_size - start_pos).min(buf.len() as u64) as usize;

        let mut read_bytes = 0;
        while read_bytes < to_read {
            let cur_vpos = start_pos + (read_bytes as u64);
            let block_idx = cur_vpos / block_size;
            let offset_in_block = cur_vpos % block_size;
            let chunk_len = ((block_size - offset_in_block) as usize).min(to_read - read_bytes);

            match self.get_used_block_ordinal(block_idx) {
                Some(ordinal) => {
                    let host_offset = self.header.data_start_offset + (ordinal * block_size) + offset_in_block;
                    self.source.seek(SeekFrom::Start(host_offset))?;
                    self.source.read_exact(&mut buf[read_bytes..read_bytes + chunk_len])?;
                }
                None => {
                    buf[read_bytes..read_bytes + chunk_len].fill(0);
                }
            }

            read_bytes += chunk_len;
        }

        Ok(read_bytes)
    }
}

impl Read for PartcloneReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.read_at_pos(self.pos, buf)?;
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for PartcloneReader {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let total_size = self.virtual_size();
        let new_pos = match pos {
            SeekFrom::Start(off) => off as i64,
            SeekFrom::Current(off) => (self.pos as i64) + off,
            SeekFrom::End(off) => (total_size as i64) + off,
        };
        if new_pos < 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "Negative seek in Partclone"));
        }
        self.pos = (new_pos as u64).min(total_size);
        Ok(self.pos)
    }
}

impl std::io::Write for PartcloneReader {
    fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "Read-only Partclone"))
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// 🌟 UNIFIED VIRTUAL DISK STREAM
// -----------------------------------------------------------------------------

pub enum VirtualDiskStream {
    Raw(StreamSlice<File>),
    Qcow2(Qcow2Reader),
    Vmdk(VmdkReader),
    Vhd(VhdReader),
    Partclone(PartcloneReader),
}

impl VirtualDiskStream {
    pub fn open(path_str: &str) -> Result<Self, std::io::Error> {
        let file = File::open(path_str)?;
        let file_len = file.metadata()?.len();

        if file_len < 512 {
            return Ok(Self::Raw(StreamSlice::new(file, 0, file_len)));
        }

        let lower = path_str.to_lowercase();

        // 1. Try QCOW2
        if lower.ends_with(".qcow2") || lower.ends_with(".qcow") {
            if let Ok(qcow) = Qcow2Reader::open(File::open(path_str)?) {
                return Ok(Self::Qcow2(qcow));
            }
        }

        // 2. Try VMDK
        if lower.ends_with(".vmdk") {
            if let Ok(vmdk) = VmdkReader::open(File::open(path_str)?) {
                return Ok(Self::Vmdk(vmdk));
            }
        }

        // 3. Try VHD
        if lower.ends_with(".vhd") || lower.ends_with(".vhdx") {
            if let Ok(vhd) = VhdReader::open(File::open(path_str)?) {
                return Ok(Self::Vhd(vhd));
            }
        }

        // 4. Try Partclone
        if lower.ends_with(".img") || lower.ends_with(".partclone") {
            if let Ok(partclone) = PartcloneReader::open_path(Path::new(path_str)) {
                return Ok(Self::Partclone(partclone));
            }
        }

        // 5. Header-based probing for non-standard file extensions
        if let Ok(qcow) = Qcow2Reader::open(File::open(path_str)?) {
            return Ok(Self::Qcow2(qcow));
        }
        if let Ok(vmdk) = VmdkReader::open(File::open(path_str)?) {
            return Ok(Self::Vmdk(vmdk));
        }
        if let Ok(vhd) = VhdReader::open(File::open(path_str)?) {
            return Ok(Self::Vhd(vhd));
        }
        if let Ok(partclone) = PartcloneReader::open_path(Path::new(path_str)) {
            return Ok(Self::Partclone(partclone));
        }

        Ok(Self::Raw(StreamSlice::new(file, 0, file_len)))
    }

    pub fn total_virtual_size(&self) -> u64 {
        match self {
            Self::Raw(r) => r.size,
            Self::Qcow2(q) => q.virtual_size(),
            Self::Vmdk(v) => v.virtual_size(),
            Self::Vhd(v) => v.virtual_size(),
            Self::Partclone(p) => p.virtual_size(),
        }
    }

    pub fn container_type_name(&self) -> &'static str {
        match self {
            Self::Raw(_) => "Raw Sector Image",
            Self::Qcow2(_) => "QEMU QCOW2 Virtual Disk",
            Self::Vmdk(_) => "VMware VMDK Sparse Disk",
            Self::Vhd(_) => "Microsoft VHD Virtual Disk",
            Self::Partclone(_) => "Partclone Partition Image",
        }
    }

    pub fn read_virtual_at(&mut self, pos: u64, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Raw(r) => {
                r.seek(SeekFrom::Start(pos))?;
                r.read(buf)
            }
            Self::Qcow2(q) => q.read_at_pos(pos, buf),
            Self::Vmdk(v) => v.read_at_pos(pos, buf),
            Self::Vhd(v) => v.read_at_pos(pos, buf),
            Self::Partclone(p) => p.read_at_pos(pos, buf),
        }
    }
}

impl Read for VirtualDiskStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Raw(r) => r.read(buf),
            Self::Qcow2(q) => q.read(buf),
            Self::Vmdk(v) => v.read(buf),
            Self::Vhd(v) => v.read(buf),
            Self::Partclone(p) => p.read(buf),
        }
    }
}

impl Seek for VirtualDiskStream {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        match self {
            Self::Raw(r) => r.seek(pos),
            Self::Qcow2(q) => q.seek(pos),
            Self::Vmdk(v) => v.seek(pos),
            Self::Vhd(v) => v.seek(pos),
            Self::Partclone(p) => p.seek(pos),
        }
    }
}

impl std::io::Write for VirtualDiskStream {
    fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "Read-only virtual disk stream"))
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// 🌟 FOG PROJECT IMAGE FOLDER MANIFEST & PARSER
// -----------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct FogPartitionEntry {
    pub dev_name: String,
    pub start_lba: u64,
    pub size_lba: u64,
    pub size_bytes: u64,
    pub type_guid_or_id: String,
    pub fs_name: String,
    pub is_fixed: bool,
    pub image_filename: Option<String>,
}

#[derive(Debug, Clone)]
pub struct FogProjectManifest {
    pub dir_path: PathBuf,
    pub label_type: String, // "gpt" or "dos"
    pub disk_id: String,
    pub partitions: Vec<FogPartitionEntry>,
}

impl FogProjectManifest {
    pub fn probe_directory(dir: &Path) -> Result<Option<Self>, std::io::Error> {
        if !dir.is_dir() {
            return Ok(None);
        }

        let partitions_file = dir.join("d1.partitions");
        let min_partitions_file = dir.join("d1.minimum.partitions");
        let mbr_file = dir.join("d1.mbr");

        if !partitions_file.exists() && !min_partitions_file.exists() && !mbr_file.exists() {
            return Ok(None);
        }

        let mut label_type = "dos".to_string();
        let mut disk_id = String::new();
        let mut partitions = Vec::new();

        // Parse sfdisk layout file (d1.partitions or d1.minimum.partitions)
        let sfdisk_path = if partitions_file.exists() { partitions_file } else { min_partitions_file };
        if let Ok(content) = std::fs::read_to_string(&sfdisk_path) {
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("label:") {
                    label_type = trimmed.strip_prefix("label:").unwrap_or("").trim().to_string();
                } else if trimmed.starts_with("label-id:") {
                    disk_id = trimmed.strip_prefix("label-id:").unwrap_or("").trim().to_string();
                } else if trimmed.starts_with("/dev/") {
                    // /dev/sda1 : start= 2048, size= 1048576, type=c12a7328-f81f-11d2-ba4b-00a0c93ec93b, uuid=...
                    if let Some((dev, params)) = trimmed.split_once(':') {
                        let dev_name = dev.trim().to_string();
                        let mut start_lba = 0u64;
                        let mut size_lba = 0u64;
                        let mut type_str = String::new();

                        for param in params.split(',') {
                            let param = param.trim();
                            if let Some((k, v)) = param.split_once('=') {
                                let k = k.trim();
                                let v = v.trim();
                                match k {
                                    "start" => start_lba = v.parse().unwrap_or(0),
                                    "size" => size_lba = v.parse().unwrap_or(0),
                                    "type" => type_str = v.to_string(),
                                    _ => {}
                                }
                            }
                        }

                        // Determine associated FOG partclone filename
                        let part_num = if let Some(pos) = dev_name.rfind('p') {
                            let suffix = &dev_name[pos + 1..];
                            if suffix.chars().all(|c| c.is_ascii_digit()) && !suffix.is_empty() {
                                suffix.to_string()
                            } else {
                                dev_name.chars().rev().take_while(|c| c.is_ascii_digit()).collect::<String>().chars().rev().collect()
                            }
                        } else {
                            dev_name.chars().rev().take_while(|c| c.is_ascii_digit()).collect::<String>().chars().rev().collect()
                        };
                        let base_img = format!("d1p{}.img", part_num);
                        let zst_img = format!("d1p{}.zst", part_num);
                        let gz_img = format!("d1p{}.img.gz", part_num);

                        let matched_file = if dir.join(&base_img).exists() {
                            Some(base_img)
                        } else if dir.join(&zst_img).exists() {
                            Some(zst_img)
                        } else if dir.join(&gz_img).exists() {
                            Some(gz_img)
                        } else {
                            None
                        };

                        partitions.push(FogPartitionEntry {
                            dev_name,
                            start_lba,
                            size_lba,
                            size_bytes: size_lba * 512,
                            type_guid_or_id: type_str,
                            fs_name: "auto".to_string(),
                            is_fixed: false,
                            image_filename: matched_file,
                        });
                    }
                }
            }
        }

        // Cross-reference d1.original.fstypes
        let fstypes_file = dir.join("d1.original.fstypes");
        if let Ok(content) = std::fs::read_to_string(&fstypes_file) {
            for line in content.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    let dev = parts[0];
                    let fstype = parts[1];
                    if let Some(p) = partitions.iter_mut().find(|p| p.dev_name.contains(dev) || dev.contains(&p.dev_name)) {
                        p.fs_name = fstype.to_string();
                    }
                }
            }
        }

        // Cross-reference d1.fixed_size_partitions (e.g. ":1:2")
        let fixed_file = dir.join("d1.fixed_size_partitions");
        if let Ok(content) = std::fs::read_to_string(&fixed_file) {
            for num in content.split(':') {
                if let Ok(n) = num.trim().parse::<usize>() {
                    if n > 0 && n <= partitions.len() {
                        partitions[n - 1].is_fixed = true;
                    }
                }
            }
        }

        Ok(Some(Self {
            dir_path: dir.to_path_buf(),
            label_type,
            disk_id,
            partitions,
        }))
    }

    pub fn generate_summary_report(&self) -> String {
        let mut report = format!(
            "=== FOG PROJECT IMAGE MANIFEST & DEPLOYMENT SUMMARY ===\nDirectory: {}\nPartition Scheme: {}\nDisk ID / UUID: {}\nTotal Partitions: {}\n\n",
            self.dir_path.display(),
            self.label_type.to_uppercase(),
            if self.disk_id.is_empty() { "N/A" } else { &self.disk_id },
            self.partitions.len()
        );

        for (idx, p) in self.partitions.iter().enumerate() {
            report += &format!(
                "Partition #{}: {}\n  Start Sector   : LBA {} (offset {} bytes)\n  Sector Count   : {} sectors\n  Partition Size : {} ({})\n  Type / GUID    : {}\n  Filesystem     : {}\n  Fixed Resizing : {}\n  Image Stream   : {}\n\n",
                idx + 1,
                p.dev_name,
                p.start_lba,
                p.start_lba * 512,
                p.size_lba,
                format_bytes(p.size_bytes),
                p.size_bytes,
                if p.type_guid_or_id.is_empty() { "Auto" } else { &p.type_guid_or_id },
                p.fs_name,
                if p.is_fixed { "YES (Non-resizable)" } else { "No (Auto-expandable)" },
                p.image_filename.as_deref().unwrap_or("Raw/Missing")
            );
        }

        report
    }
}

// -----------------------------------------------------------------------------
// 🌟 MAIN DISK IMAGE & CONTAINER HANDLER
// -----------------------------------------------------------------------------

pub struct DiskImageHandler;

impl DiskImageHandler {
    pub fn is_supported_image(path_or_name: &str) -> bool {
        let lower = path_or_name.to_lowercase();
        lower.ends_with(".img")
            || lower.ends_with(".raw")
            || lower.ends_with(".dd")
            || lower.ends_with(".vhd")
            || lower.ends_with(".vhdx")
            || lower.ends_with(".qcow2")
            || lower.ends_with(".qcow")
            || lower.ends_with(".vmdk")
            || lower.ends_with(".zst")
            || lower.ends_with(".zstd")
    }

    /// Probes partition table (GPT, MBR, FOG, or unpartitioned) and filesystem types of a disk image
    pub fn probe_image(archive_path_str: &str) -> Result<Vec<DiskPartition>, std::io::Error> {
        let path = Path::new(archive_path_str);

        // 1. Check if path is a FOG Project directory or manifest file inside it
        let fog_dir_opt = if path.is_dir() {
            Some(path)
        } else if path.is_file() {
            let fname = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if fname.starts_with("d1") || fname.ends_with(".partitions") {
                path.parent()
            } else {
                None
            }
        } else {
            None
        };

        if let Some(fog_dir) = fog_dir_opt {
            if let Ok(Some(fog)) = FogProjectManifest::probe_directory(fog_dir) {
                let mut partitions = Vec::new();
                for (idx, fp) in fog.partitions.iter().enumerate() {
                    let upper_guid = fp.type_guid_or_id.to_uppercase();
                    let fs_type = if fp.fs_name.to_lowercase().contains("ext") {
                        DiskFsType::Ext4
                    } else if fp.fs_name.to_lowercase().contains("fat") || upper_guid == "C12A7328-F81F-11D2-BA4B-00A0C93EC93B" {
                        DiskFsType::Fat32
                    } else if fp.fs_name.to_lowercase().contains("ntfs") || upper_guid == "DE94BBA4-06D1-4D40-A16A-BFD50179D6AC" {
                        DiskFsType::Ntfs
                    } else {
                        DiskFsType::Partclone
                    };

                    partitions.push(DiskPartition {
                        index: idx + 1,
                        name: fp.dev_name.clone(),
                        start_byte: fp.start_lba * 512,
                        size_bytes: fp.size_bytes,
                        fs_type,
                        table_type: "FOG".to_string(),
                        type_str: fp.type_guid_or_id.clone(),
                        bootable: false,
                        container_subpath: fp.image_filename.clone(),
                    });
                }
                return Ok(partitions);
            }
        }

        let mut vdisk = VirtualDiskStream::open(archive_path_str)?;
        let file_len = vdisk.total_virtual_size();
        if file_len < 512 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Image file is smaller than 512 bytes",
            ));
        }

        // If direct Partclone stream, probe filesystem directly
        if let VirtualDiskStream::Partclone(ref p) = vdisk {
            let fs_type = p.get_fs_type();
            return Ok(vec![DiskPartition {
                index: 1,
                name: format!("Partclone {}", fs_type.display_name()),
                start_byte: 0,
                size_bytes: file_len,
                fs_type,
                table_type: "Partclone".to_string(),
                type_str: p.header.fs_name.clone(),
                bootable: false,
                container_subpath: None,
            }]);
        }

        let mut sector_0 = [0u8; 512];
        vdisk.read_virtual_at(0, &mut sector_0)?;

        let mut partitions = Vec::new();

        // 2. Try GPT (GUID Partition Table)
        if file_len >= 1024 {
            let mut sector_1 = [0u8; 512];
            if vdisk.read_virtual_at(512, &mut sector_1).is_ok() && &sector_1[0..8] == b"EFI PART" {
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
                        if vdisk.read_virtual_at(offset, &mut entry_buf).is_err() {
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

                        let fs_type = Self::probe_virtual_filesystem(&mut vdisk, start_byte, size_bytes)?;

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
                            container_subpath: None,
                        });
                        part_idx += 1;
                    }
                }
            }
        }

        // 3. If no GPT partitions, try MBR (Master Boot Record)
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
                        let fs_type = Self::probe_virtual_filesystem(&mut vdisk, start_byte, size_bytes)?;
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
                            container_subpath: None,
                        });
                        part_idx += 1;
                    }
                }
            }
        }

        // 4. If still no partitions, probe raw unpartitioned filesystem at offset 0
        if partitions.is_empty() {
            let raw_fs = Self::probe_virtual_filesystem(&mut vdisk, 0, file_len)?;
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
                    container_subpath: None,
                });
            }
        }

        Ok(partitions)
    }

    /// Probes filesystem header signatures on a virtual disk stream
    pub fn probe_virtual_filesystem(vdisk: &mut VirtualDiskStream, start_byte: u64, size_bytes: u64) -> Result<DiskFsType, std::io::Error> {
        let total_size = vdisk.total_virtual_size();
        if start_byte >= total_size {
            return Ok(DiskFsType::Unknown);
        }

        // Check FAT (FAT12, FAT16, FAT32)
        let mut boot_sector = [0u8; 512];
        if vdisk.read_virtual_at(start_byte, &mut boot_sector).is_ok() && boot_sector[510..512] == [0x55, 0xAA] {
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
            if vdisk.read_virtual_at(start_byte + 1024, &mut super_block).is_ok() {
                let magic = u16::from_le_bytes(super_block[0x38..0x3A].try_into().unwrap_or([0; 2]));
                if magic == 0xEF53 {
                    return Ok(DiskFsType::Ext4);
                }
            }
        }

        // Check SquashFS (Magic "hsqs" at start)
        let mut sqsh_header = [0u8; 4];
        if vdisk.read_virtual_at(start_byte, &mut sqsh_header).is_ok() && &sqsh_header == b"hsqs" {
            return Ok(DiskFsType::Squashfs);
        }

        // Check ISO 9660 ("CD001" at offset 0x8000)
        if size_bytes >= 0x8000 + 2048 {
            let mut iso_pvd = [0u8; 6];
            if vdisk.read_virtual_at(start_byte + 0x8000, &mut iso_pvd).is_ok() && &iso_pvd[1..6] == b"CD001" {
                return Ok(DiskFsType::Iso9660);
            }
        }

        // Check UDF ("BEA01" / "NSR02" / "NSR03" at 0x8000)
        if size_bytes >= 0x8000 + 2048 {
            let mut udf_sig = [0u8; 6];
            if vdisk.read_virtual_at(start_byte + 0x8000, &mut udf_sig).is_ok() && (&udf_sig[1..6] == b"BEA01" || &udf_sig[1..6] == b"NSR02" || &udf_sig[1..6] == b"NSR03") {
                return Ok(DiskFsType::Udf);
            }
        }

        Ok(DiskFsType::Unknown)
    }

    /// List directory contents of a disk image partition, container, or root
    pub fn list_image_contents(archive_path_str: &str, subpath: &str) -> Result<DirectoryListing, std::io::Error> {
        let partitions = Self::probe_image(archive_path_str)?;
        let clean_sub = subpath.trim_matches('/');

        if partitions.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Unable to find recognized partition table or filesystem in disk image/container",
            ));
        }

        let path = Path::new(archive_path_str);
        let is_dir_or_fog = path.is_dir()
            || (path.is_file()
                && path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with("d1") || n.ends_with(".partitions"))
                    .unwrap_or(false));

        // Check if root of multi-partition image / FOG directory
        if (partitions.len() > 1 || is_dir_or_fog) && clean_sub.is_empty() {
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

            // Add virtual layout / manifest summary entry
            let summary_filename = if is_dir_or_fog {
                "[FOG Image Summary.txt]"
            } else {
                "[Disk Layout.txt]"
            };

            entries.push(FileEntry {
                name: summary_filename.to_string(),
                path: format!("archive://{}#{}", archive_path_str, summary_filename),
                is_dir: false,
                is_symlink: false,
                is_empty: Some(false),
                size: 1024,
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
        let (target_part, inner_fs_path) = if partitions.len() == 1 && !is_dir_or_fog {
            (&partitions[0], clean_sub)
        } else {
            let first_segment = clean_sub.split('/').next().unwrap_or("");
            if let Some(p) = partitions.iter().find(|p| p.slug() == first_segment) {
                let rest = clean_sub.strip_prefix(first_segment).unwrap_or("").trim_matches('/');
                (p, rest)
            } else if clean_sub == "[Disk Layout.txt]" || clean_sub == "[FOG Image Summary.txt]" {
                return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "Not a directory"));
            } else {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("Partition '{}' not found in disk image", first_segment),
                ));
            }
        };

        // If target is inside a FOG Project folder with a dedicated sub-image
        let fog_base_dir = if path.is_dir() {
            Some(path)
        } else if path.is_file() {
            path.parent()
        } else {
            None
        };

        let fog_part_img = if let Some(dir) = fog_base_dir {
            target_part.container_subpath.as_ref().map(|sub| dir.join(sub))
        } else {
            None
        };

        let effective_stream_path = fog_part_img
            .as_ref()
            .and_then(|p| p.to_str())
            .unwrap_or(archive_path_str);
        let start_offset = if fog_part_img.is_some() {
            0
        } else {
            target_part.start_byte
        };

        // Open filesystem on selected partition
        match target_part.fs_type {
            DiskFsType::Fat12 | DiskFsType::Fat16 | DiskFsType::Fat32 => {
                Self::list_fat_partition(archive_path_str, effective_stream_path, target_part, start_offset, inner_fs_path)
            }
            DiskFsType::Ext4 => {
                Self::list_ext4_partition(archive_path_str, effective_stream_path, target_part, start_offset, inner_fs_path)
            }
            DiskFsType::Iso9660 | DiskFsType::Udf => {
                super::archive::ArchiveHandler::list_iso_contents(effective_stream_path, inner_fs_path)
            }
            DiskFsType::Squashfs => {
                super::archive::ArchiveHandler::list_squashfs_contents(effective_stream_path, inner_fs_path)
            }
            DiskFsType::Ntfs => {
                match Self::list_ntfs_partition(archive_path_str, effective_stream_path, target_part, start_offset, inner_fs_path) {
                    Ok(listing) => Ok(listing),
                    Err(err) => {
                        tracing::debug!("NTFS listing failed for {}: {}, fallback to metadata", effective_stream_path, err);
                        Self::fallback_partition_metadata_listing(archive_path_str, target_part)
                    }
                }
            }
            DiskFsType::Partclone => {
                // Try NTFS, then FAT, then Ext4, then fallback to metadata
                if let Ok(listing) = Self::list_ntfs_partition(archive_path_str, effective_stream_path, target_part, start_offset, inner_fs_path) {
                    Ok(listing)
                } else if let Ok(listing) = Self::list_fat_partition(archive_path_str, effective_stream_path, target_part, start_offset, inner_fs_path) {
                    Ok(listing)
                } else {
                    Self::fallback_partition_metadata_listing(archive_path_str, target_part)
                }
            }
            DiskFsType::Unknown => Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                format!("Unsupported filesystem format on partition {}", target_part.name),
            )),
        }
    }

    /// List FAT12/16/32 directory contents on a virtual disk stream
    fn list_fat_partition(
        archive_path_str: &str,
        effective_image_path: &str,
        part: &DiskPartition,
        start_offset: u64,
        inner_path: &str,
    ) -> Result<DirectoryListing, std::io::Error> {
        let vdisk = VirtualDiskStream::open(effective_image_path)?;
        let slice = StreamSlice::new(vdisk, start_offset, part.size_bytes);
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
        effective_image_path: &str,
        part: &DiskPartition,
        start_offset: u64,
        inner_path: &str,
    ) -> Result<DirectoryListing, std::io::Error> {
        let file = File::open(effective_image_path)?;
        let slice = positioned_io::Slice::new(file, start_offset, Some(part.size_bytes));
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

    /// List NTFS directory contents on a virtual disk stream
    fn list_ntfs_partition(
        archive_path_str: &str,
        effective_image_path: &str,
        part: &DiskPartition,
        start_offset: u64,
        inner_path: &str,
    ) -> Result<DirectoryListing, std::io::Error> {
        let vdisk = VirtualDiskStream::open(effective_image_path)?;
        let mut slice = StreamSlice::new(vdisk, start_offset, part.size_bytes);
        let mut ntfs = ntfs::Ntfs::new(&mut slice)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("NTFS mount error: {}", e)))?;
        let _ = ntfs.read_upcase_table(&mut slice);

        let mut current_dir = ntfs.root_directory(&mut slice)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("NTFS root dir error: {}", e)))?;

        let clean_inner = inner_path.trim_matches('/');
        if !clean_inner.is_empty() {
            for segment in clean_inner.split('/') {
                if segment.is_empty() {
                    continue;
                }
                let index = current_dir.directory_index(&mut slice)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::NotFound, format!("NTFS index error: {}", e)))?;
                let mut iter = index.entries();
                let mut found = None;
                while let Some(entry_res) = iter.next(&mut slice) {
                    let entry = match entry_res {
                        Ok(e) => e,
                        Err(_) => continue,
                    };
                    if let Some(Ok(key)) = entry.key() {
                        let name = key.name().to_string_lossy();
                        if name.eq_ignore_ascii_case(segment) {
                            if let Ok(child) = entry.to_file(&ntfs, &mut slice) {
                                if child.is_directory() {
                                    found = Some(child);
                                    break;
                                }
                            }
                        }
                    }
                }
                match found {
                    Some(next_dir) => current_dir = next_dir,
                    None => return Err(std::io::Error::new(std::io::ErrorKind::NotFound, format!("Directory '{}' not found in NTFS volume", segment))),
                }
            }
        }

        let index = current_dir.directory_index(&mut slice)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("NTFS directory index error: {}", e)))?;
        let mut iter = index.entries();

        let mut entries = Vec::new();
        let mut total_files = 0;
        let mut total_dirs = 0;
        let mut total_size = 0u64;

        let part_prefix = if part.table_type == "Raw" {
            String::new()
        } else {
            format!("{}/", part.slug())
        };

        let mut seen_names = std::collections::HashSet::new();

        while let Some(entry_res) = iter.next(&mut slice) {
            let entry = match entry_res {
                Ok(e) => e,
                Err(_) => continue,
            };
            let key = match entry.key() {
                Some(Ok(k)) => k,
                _ => continue,
            };

            let name = key.name().to_string_lossy();
            if name == "." || name == ".." || name.is_empty() {
                continue;
            }

            let lower_name = name.to_lowercase();
            if seen_names.contains(&lower_name) {
                continue;
            }
            seen_names.insert(lower_name);

            let child_file = match entry.to_file(&ntfs, &mut slice) {
                Ok(f) => f,
                Err(_) => continue,
            };

            let is_dir = child_file.is_directory();
            let size = if is_dir {
                0
            } else if let Some(Ok(data_item)) = child_file.data(&mut slice, "") {
                data_item.to_attribute().map(|a| a.value_length()).unwrap_or(child_file.data_size() as u64)
            } else {
                child_file.data_size() as u64
            };

            let modified_time = if let Ok(info) = child_file.info() {
                let intervals = info.modification_time().nt_timestamp();
                let nt_epoch_diff = 116_444_736_000_000_000u64;
                let secs = (intervals.saturating_sub(nt_epoch_diff)) / 10_000_000;
                Some(secs)
            } else {
                None
            };

            let entry_inner_path = if clean_inner.is_empty() {
                format!("{}{}", part_prefix, name)
            } else {
                format!("{}{}/{}", part_prefix, clean_inner, name)
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
                modified: modified_time,
                permissions: if is_dir { "drwxr-xr-x".to_string() } else { "-rw-r--r--".to_string() },
                mode_octal: if is_dir { "0755".to_string() } else { "0644".to_string() },
                owner: "ntfs".to_string(),
                group: part.fs_type.display_name().to_string(),
                uid: 1000,
                gid: 1000,
                mime_type: if is_dir { None } else { Some(mime_guess::from_path(&name).first_or_octet_stream().to_string()) },
                is_archive: false,
            });
        }

        let cur_sub = if clean_inner.is_empty() {
            if part.table_type == "Raw" { "".to_string() } else { part.slug() }
        } else {
            if part.table_type == "Raw" { clean_inner.to_string() } else { format!("{}/{}", part.slug(), clean_inner) }
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

    fn fallback_partition_metadata_listing(archive_path_str: &str, target_part: &DiskPartition) -> Result<DirectoryListing, std::io::Error> {
        let mut entries = Vec::new();
        entries.push(FileEntry {
            name: "[Partition Info.txt]".to_string(),
            path: format!("archive://{}#{}/[Partition Info.txt]", archive_path_str, target_part.slug()),
            is_dir: false,
            is_symlink: false,
            is_empty: Some(false),
            size: 512,
            modified: None,
            permissions: "-rw-r--r--".to_string(),
            mode_octal: "0644".to_string(),
            owner: "partclone".to_string(),
            group: target_part.fs_type.display_name().to_string(),
            uid: 1000,
            gid: 1000,
            mime_type: Some("text/plain".to_string()),
            is_archive: false,
        });
        Ok(DirectoryListing {
            current_path: format!("archive://{}#{}", archive_path_str, target_part.slug()),
            parent_path: None,
            total_files: 1,
            total_dirs: 0,
            total_size: target_part.size_bytes,
            protocol: "archive".to_string(),
            entries,
            is_truncated: None,
            max_limit: None,
        })
    }

    /// Read a specific file entry from a disk image partition or container
    pub fn read_image_entry(
        archive_path_str: &str,
        inner_path: &str,
        max_bytes: Option<usize>,
    ) -> Result<FileContentResponse, std::io::Error> {
        let clean_path = inner_path.trim_matches('/');

        // Special virtual file: [FOG Image Summary.txt]
        if clean_path == "[FOG Image Summary.txt]" {
            let path = Path::new(archive_path_str);
            let fog_dir_opt = if path.is_dir() {
                Some(path)
            } else if path.is_file() {
                path.parent()
            } else {
                None
            };
            if let Some(fog_dir) = fog_dir_opt {
                if let Ok(Some(fog)) = FogProjectManifest::probe_directory(fog_dir) {
                    let report = fog.generate_summary_report();
                    let size = report.len() as u64;
                    return Ok(FileContentResponse {
                        path: format!("archive://{}#[FOG Image Summary.txt]", archive_path_str),
                        name: "[FOG Image Summary.txt]".to_string(),
                        content: report,
                        is_binary: false,
                        size,
                        mime_type: "text/plain".to_string(),
                    });
                }
            }
        }

        // Special virtual file: [Disk Layout.txt]
        if clean_path == "[Disk Layout.txt]" {
            let partitions = Self::probe_image(archive_path_str)?;
            let vdisk = VirtualDiskStream::open(archive_path_str)?;
            let mut report = format!(
                "=== BRUM DISK IMAGE & CONTAINER INSPECTION REPORT ===\nImage File: {}\nContainer Format: {}\nTotal Virtual Capacity: {} ({} bytes)\nTotal Partitions: {}\n\n",
                archive_path_str,
                vdisk.container_type_name(),
                format_bytes(vdisk.total_virtual_size()),
                vdisk.total_virtual_size(),
                partitions.len()
            );

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

            let size = report.len() as u64;
            return Ok(FileContentResponse {
                path: format!("archive://{}#[Disk Layout.txt]", archive_path_str),
                name: "[Disk Layout.txt]".to_string(),
                content: report,
                is_binary: false,
                size,
                mime_type: "text/plain".to_string(),
            });
        }

        let partitions = Self::probe_image(archive_path_str)?;
        let path = Path::new(archive_path_str);
        let is_dir_or_fog = path.is_dir()
            || (path.is_file()
                && path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with("d1") || n.ends_with(".partitions"))
                    .unwrap_or(false));

        let (target_part, file_subpath) = if partitions.len() == 1 && !is_dir_or_fog {
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

        let fog_base_dir = if path.is_dir() {
            Some(path)
        } else if path.is_file() {
            path.parent()
        } else {
            None
        };

        let fog_part_img = if let Some(dir) = fog_base_dir {
            target_part.container_subpath.as_ref().map(|sub| dir.join(sub))
        } else {
            None
        };

        let effective_stream_path = fog_part_img
            .as_ref()
            .and_then(|p| p.to_str())
            .unwrap_or(archive_path_str);
        let (start_offset, partition_size) = if fog_part_img.is_some() {
            (0, target_part.size_bytes)
        } else {
            (target_part.start_byte, target_part.size_bytes)
        };

        match target_part.fs_type {
            DiskFsType::Fat12 | DiskFsType::Fat16 | DiskFsType::Fat32 => {
                let vdisk = VirtualDiskStream::open(effective_stream_path)?;
                let slice = StreamSlice::new(vdisk, start_offset, partition_size);
                let fs = fatfs::FileSystem::new(slice, fatfs::FsOptions::new())
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("FAT error: {}", e)))?;
                let mut fat_file = fs.root_dir().open_file(file_subpath)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::NotFound, format!("FAT file open error: {}", e)))?;

                let limit = match max_bytes {
                    Some(0) | None => usize::MAX,
                    Some(l) => l,
                };
                let mut buf = Vec::new();
                let mut chunk = vec![0u8; 64 * 1024];
                while buf.len() < limit {
                    let to_read = chunk.len().min(limit.saturating_sub(buf.len()));
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
                let file = File::open(effective_stream_path)?;
                let slice = positioned_io::Slice::new(file, start_offset, Some(partition_size));
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

                let limit = match max_bytes {
                    Some(0) | None => usize::MAX,
                    Some(l) => l,
                };
                let mut buf = Vec::new();
                let mut chunk = vec![0u8; 64 * 1024];
                while buf.len() < limit {
                    let to_read = chunk.len().min(limit.saturating_sub(buf.len()));
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
            DiskFsType::Ntfs => {
                if file_subpath == "[Partition Info.txt]" {
                    Self::fallback_partition_metadata_response(archive_path_str, target_part, clean_path)
                } else {
                    Self::read_ntfs_file(
                        archive_path_str,
                        effective_stream_path,
                        target_part,
                        start_offset,
                        partition_size,
                        clean_path,
                        file_subpath,
                        max_bytes,
                    )
                }
            }
            DiskFsType::Partclone => {
                if file_subpath == "[Partition Info.txt]" || file_subpath.is_empty() {
                    Self::fallback_partition_metadata_response(archive_path_str, target_part, clean_path)
                } else {
                    Self::read_ntfs_file(
                        archive_path_str,
                        effective_stream_path,
                        target_part,
                        start_offset,
                        partition_size,
                        clean_path,
                        file_subpath,
                        max_bytes,
                    )
                }
            }
            DiskFsType::Iso9660 | DiskFsType::Udf => {
                super::archive::ArchiveHandler::read_iso_entry(effective_stream_path, file_subpath, max_bytes.unwrap_or(0))
            }
            DiskFsType::Squashfs => {
                super::archive::ArchiveHandler::read_squashfs_entry(effective_stream_path, file_subpath, max_bytes.unwrap_or(0))
            }
            _ => Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                format!("Unsupported filesystem format on partition {}", target_part.name),
            )),
        }
    }

    fn fallback_partition_metadata_response(archive_path_str: &str, target_part: &DiskPartition, _clean_path: &str) -> Result<FileContentResponse, std::io::Error> {
        let info = format!(
            "=== PARTITION INFORMATION ===\nPartition: {}\nType: {}\nFilesystem: {}\nStart LBA: {}\nOffset: {} bytes\nSize: {} ({})\nFixed Size: {}\nImage File: {}\n",
            target_part.name,
            target_part.type_str,
            target_part.fs_type.display_name(),
            target_part.start_byte / 512,
            target_part.start_byte,
            format_bytes(target_part.size_bytes),
            target_part.size_bytes,
            if target_part.bootable { "YES" } else { "NO" },
            target_part.container_subpath.as_deref().unwrap_or("N/A")
        );
        let size = info.len() as u64;
        Ok(FileContentResponse {
            path: format!("archive://{}#{}/[Partition Info.txt]", archive_path_str, target_part.slug()),
            name: "[Partition Info.txt]".to_string(),
            content: info,
            is_binary: false,
            size,
            mime_type: "text/plain".to_string(),
        })
    }

    fn read_ntfs_file(
        archive_path_str: &str,
        effective_image_path: &str,
        _target_part: &DiskPartition,
        start_offset: u64,
        partition_size: u64,
        clean_path: &str,
        file_subpath: &str,
        max_bytes: Option<usize>,
    ) -> Result<FileContentResponse, std::io::Error> {
        let vdisk = VirtualDiskStream::open(effective_image_path)?;
        let mut slice = StreamSlice::new(vdisk, start_offset, partition_size);
        let mut ntfs = ntfs::Ntfs::new(&mut slice)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("NTFS mount error: {}", e)))?;
        let _ = ntfs.read_upcase_table(&mut slice);

        let clean_file_path = file_subpath.trim_matches('/');
        let segments: Vec<&str> = clean_file_path.split('/').filter(|s| !s.is_empty()).collect();
        if segments.is_empty() {
            return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "Empty file path"));
        }

        let mut current_dir = ntfs.root_directory(&mut slice)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("NTFS root dir error: {}", e)))?;

        for &segment in &segments[..segments.len() - 1] {
            let index = current_dir.directory_index(&mut slice)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::NotFound, format!("NTFS index error: {}", e)))?;
            let mut iter = index.entries();
            let mut found = None;
            while let Some(entry_res) = iter.next(&mut slice) {
                let entry = match entry_res {
                    Ok(e) => e,
                    Err(_) => continue,
                };
                if let Some(Ok(key)) = entry.key() {
                    if key.name().to_string_lossy().eq_ignore_ascii_case(segment) {
                        if let Ok(child) = entry.to_file(&ntfs, &mut slice) {
                            if child.is_directory() {
                                found = Some(child);
                                break;
                            }
                        }
                    }
                }
            }
            match found {
                Some(next_dir) => current_dir = next_dir,
                None => return Err(std::io::Error::new(std::io::ErrorKind::NotFound, format!("Directory '{}' not found in NTFS volume", segment))),
            }
        }

        let target_filename = segments.last().unwrap();
        let index = current_dir.directory_index(&mut slice)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::NotFound, format!("NTFS index error: {}", e)))?;
        let mut iter = index.entries();
        let mut found_file = None;

        while let Some(entry_res) = iter.next(&mut slice) {
            let entry = match entry_res {
                Ok(e) => e,
                Err(_) => continue,
            };
            if let Some(Ok(key)) = entry.key() {
                if key.name().to_string_lossy().eq_ignore_ascii_case(target_filename) {
                    if let Ok(child) = entry.to_file(&ntfs, &mut slice) {
                        if !child.is_directory() {
                            found_file = Some(child);
                            break;
                        }
                    }
                }
            }
        }

        let file = found_file.ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, format!("File '{}' not found in NTFS volume", target_filename))
        })?;

        let data_item = file.data(&mut slice, "")
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "No default data stream found in NTFS file"))?
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("NTFS data attribute error: {}", e)))?;

        let data_attr = data_item.to_attribute()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("NTFS attribute error: {}", e)))?;

        let data_value = data_attr.value(&mut slice)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("NTFS data value error: {}", e)))?;

        let total_len = data_value.len();
        let limit = match max_bytes {
            Some(0) | None => total_len as usize,
            Some(l) => l.min(total_len as usize),
        };
        let mut buf = vec![0u8; limit];

        let mut stream = data_value.attach(&mut slice);
        let n = stream.read(&mut buf)?;
        buf.truncate(n);

        let is_binary = buf.iter().take(4096).any(|&b| b == 0);
        let mime = mime_guess::from_path(target_filename).first_or_octet_stream().to_string();

        let content = if is_binary {
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &buf)
        } else {
            String::from_utf8_lossy(&buf).to_string()
        };

        Ok(FileContentResponse {
            path: format!("archive://{}#{}", archive_path_str, clean_path),
            name: target_filename.to_string(),
            content,
            is_binary,
            size: total_len,
            mime_type: mime,
        })
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
                    let vdisk = VirtualDiskStream::open(archive_path_str)?;
                    let slice = StreamSlice::new(vdisk, part.start_byte, part.size_bytes);
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
        assert!(layout_resp.content.contains("BRUM DISK IMAGE & CONTAINER INSPECTION REPORT"));
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

    #[test]
    fn test_qcow2_reader_lifecycle() {
        let mut tmp = NamedTempFile::new().unwrap();
        let cluster_size = 65536usize; // 64KB
        let virtual_size = 1048576u64; // 1MB
        let l1_table_offset = cluster_size as u64; // offset 65536
        let l2_table_offset = 2 * cluster_size as u64; // offset 131072
        let data_cluster_offset = 3 * cluster_size as u64; // offset 196608

        let mut image = vec![0u8; 4 * cluster_size];

        // 1. QCOW2 Header (v3)
        image[0..4].copy_from_slice(b"QFI\xFB");
        image[4..8].copy_from_slice(&3u32.to_be_bytes()); // version 3
        image[20..24].copy_from_slice(&16u32.to_be_bytes()); // cluster_bits 16
        image[24..32].copy_from_slice(&virtual_size.to_be_bytes()); // virtual size 1MB
        image[36..40].copy_from_slice(&1u32.to_be_bytes()); // l1_size = 1
        image[40..48].copy_from_slice(&l1_table_offset.to_be_bytes()); // l1_table_offset

        // 2. L1 Table Entry 0 pointing to L2 Table
        let l1_pos = l1_table_offset as usize;
        image[l1_pos..l1_pos + 8].copy_from_slice(&l2_table_offset.to_be_bytes());

        // 3. L2 Table Entry 0 pointing to Data Cluster
        let l2_pos = l2_table_offset as usize;
        image[l2_pos..l2_pos + 8].copy_from_slice(&data_cluster_offset.to_be_bytes());

        // 4. Populate Data Cluster with MBR Signature and text
        let data_pos = data_cluster_offset as usize;
        image[data_pos + 510] = 0x55;
        image[data_pos + 511] = 0xAA;

        // Partition 1 at LBA 1
        let off1 = data_pos + 446;
        image[off1] = 0x80;
        image[off1 + 4] = 0x06; // FAT16
        image[off1 + 8..off1 + 12].copy_from_slice(&1u32.to_le_bytes());
        image[off1 + 12..off1 + 16].copy_from_slice(&100u32.to_le_bytes());

        tmp.write_all(&image).unwrap();
        tmp.flush().unwrap();

        let path = tmp.path().to_str().unwrap();
        let partitions = DiskImageHandler::probe_image(path).unwrap();
        assert_eq!(partitions.len(), 1);
        assert_eq!(partitions[0].table_type, "MBR");
        assert_eq!(partitions[0].start_byte, 512);
        assert_eq!(partitions[0].size_bytes, 100 * 512);
    }

    #[test]
    fn test_vmdk_sparse_reader_lifecycle() {
        let mut tmp = NamedTempFile::new().unwrap();
        let grain_size_sectors = 128u64; // 64KB
        let num_gtes = 512u32;
        let capacity_sectors = 2048u64; // 1MB

        let gd_sector = 1u64;
        let gt_sector = 2u64;
        let data_sector = 128u64;

        let mut image = vec![0u8; (data_sector as usize + grain_size_sectors as usize) * 512];

        // 1. VMDK Header
        image[0..4].copy_from_slice(b"KDMV");
        image[4..8].copy_from_slice(&1u32.to_le_bytes()); // version 1
        image[12..20].copy_from_slice(&capacity_sectors.to_le_bytes());
        image[20..28].copy_from_slice(&grain_size_sectors.to_le_bytes());
        image[44..48].copy_from_slice(&num_gtes.to_le_bytes());
        image[56..64].copy_from_slice(&gd_sector.to_le_bytes());

        // 2. Grain Directory Entry 0 -> gt_sector
        let gd_pos = (gd_sector * 512) as usize;
        image[gd_pos..gd_pos + 4].copy_from_slice(&(gt_sector as u32).to_le_bytes());

        // 3. Grain Table Entry 0 -> data_sector
        let gt_pos = (gt_sector * 512) as usize;
        image[gt_pos..gt_pos + 4].copy_from_slice(&(data_sector as u32).to_le_bytes());

        // 4. Data Sector MBR
        let data_pos = (data_sector * 512) as usize;
        image[data_pos + 510] = 0x55;
        image[data_pos + 511] = 0xAA;
        let off1 = data_pos + 446;
        image[off1] = 0x80;
        image[off1 + 4] = 0x83; // Linux Ext4
        image[off1 + 8..off1 + 12].copy_from_slice(&1u32.to_le_bytes());
        image[off1 + 12..off1 + 16].copy_from_slice(&500u32.to_le_bytes());

        tmp.write_all(&image).unwrap();
        tmp.flush().unwrap();

        let path = tmp.path().to_str().unwrap();
        let partitions = DiskImageHandler::probe_image(path).unwrap();
        assert_eq!(partitions.len(), 1);
        assert_eq!(partitions[0].table_type, "MBR");
        assert_eq!(partitions[0].name, "Linux filesystem");
    }

    #[test]
    fn test_partclone_image_reader_lifecycle() {
        let mut tmp = NamedTempFile::new().unwrap();
        let block_size = 4096u32;
        let total_blocks = 256u64; // 1MB total
        let used_blocks = 2u64;

        let mut header = Vec::new();
        // Magic
        let mut magic = [0u8; 16];
        magic[..15].copy_from_slice(b"partclone-image");
        header.extend_from_slice(&magic);

        // FS name
        let mut fs = [0u8; 16];
        fs[..5].copy_from_slice(b"EXTFS");
        header.extend_from_slice(&fs);

        // Sizes
        header.extend_from_slice(&(1048576u64).to_le_bytes()); // device_size 1MB
        header.extend_from_slice(&total_blocks.to_le_bytes());
        header.extend_from_slice(&used_blocks.to_le_bytes());
        header.extend_from_slice(&block_size.to_le_bytes());
        header.extend_from_slice(&0u32.to_le_bytes()); // feature_size

        // Bitmap (32 bytes = 256 bits): set bit 0 and bit 1
        let mut bitmap = vec![0u8; 32];
        bitmap[0] = 0b00000011; // Blocks 0 and 1 are used
        header.extend_from_slice(&bitmap);

        // Data payload (2 blocks of 4096 bytes)
        let mut data = vec![0u8; 2 * block_size as usize];
        // Block 0 Superblock at offset 1024 with 0xEF53 magic
        data[1024 + 0x38] = 0x53;
        data[1024 + 0x39] = 0xEF;
        header.extend_from_slice(&data);

        tmp.write_all(&header).unwrap();
        tmp.flush().unwrap();

        let path = tmp.path().to_str().unwrap();
        let partitions = DiskImageHandler::probe_image(path).unwrap();
        assert_eq!(partitions.len(), 1);
        assert_eq!(partitions[0].table_type, "Partclone");
        assert_eq!(partitions[0].fs_type, DiskFsType::Ext4);
    }

    #[test]
    fn test_fog_project_directory_manifest() {
        let temp_dir = tempfile::tempdir().unwrap();
        let fog_dir = temp_dir.path().join("win11-gold-master");
        std::fs::create_dir_all(&fog_dir).unwrap();

        // 1. Write d1.partitions (sfdisk syntax)
        let sfdisk_content = r#"
label: gpt
label-id: A1B2C3D4-E5F6-7890-ABCD-EF1234567890
device: /dev/sda
unit: sectors
first-lba: 2048
last-lba: 104857600

/dev/sda1 : start= 2048, size= 204800, type=c12a7328-f81f-11d2-ba4b-00a0c93ec93b, uuid=11111111-2222-3333-4444-555555555555, name="EFI"
/dev/sda2 : start= 206848, size= 102400000, type=ebd0a0a2-b9e5-4433-87c0-68b6b72699c7, uuid=66666666-7777-8888-9999-000000000000, name="Basic data partition"
"#;
        std::fs::write(fog_dir.join("d1.partitions"), sfdisk_content).unwrap();

        // 2. Write d1.original.fstypes
        std::fs::write(fog_dir.join("d1.original.fstypes"), "/dev/sda1 vfat\n/dev/sda2 ntfs\n").unwrap();

        // 3. Write d1.fixed_size_partitions
        std::fs::write(fog_dir.join("d1.fixed_size_partitions"), ":1\n").unwrap();

        // 4. Create dummy partition image files
        std::fs::write(fog_dir.join("d1p1.img"), b"dummy efi").unwrap();
        std::fs::write(fog_dir.join("d1p2.img"), b"dummy ntfs").unwrap();

        let fog_path = fog_dir.to_str().unwrap();
        let partitions = DiskImageHandler::probe_image(fog_path).unwrap();
        assert_eq!(partitions.len(), 2);
        assert_eq!(partitions[0].name, "/dev/sda1");
        assert_eq!(partitions[0].fs_type, DiskFsType::Fat32);
        assert_eq!(partitions[1].name, "/dev/sda2");
        assert_eq!(partitions[1].fs_type, DiskFsType::Ntfs);

        // Test listing directory
        let listing = DiskImageHandler::list_image_contents(fog_path, "").unwrap();
        assert_eq!(listing.entries.len(), 3); // 2 partitions + [FOG Image Summary.txt]
        assert!(listing.entries.iter().any(|e| e.name == "[FOG Image Summary.txt]"));

        // Test reading [FOG Image Summary.txt]
        let resp = DiskImageHandler::read_image_entry(fog_path, "[FOG Image Summary.txt]", None).unwrap();
        assert!(!resp.is_binary);
        assert!(resp.content.contains("FOG PROJECT IMAGE MANIFEST"));
        assert!(resp.content.contains("/dev/sda1"));
        assert!(resp.content.contains("/dev/sda2"));
        assert!(resp.content.contains("YES (Non-resizable)"));
    }

    #[test]
    fn test_compressed_partclone_caching() {
        let block_size = 4096u32;
        let total_blocks = 256u64; // 1MB total
        let used_blocks = 2u64;

        let mut header = Vec::new();
        let mut magic = [0u8; 16];
        magic[..15].copy_from_slice(b"partclone-image");
        header.extend_from_slice(&magic);

        let mut fs = [0u8; 16];
        fs[..5].copy_from_slice(b"EXTFS");
        header.extend_from_slice(&fs);

        header.extend_from_slice(&(1048576u64).to_le_bytes());
        header.extend_from_slice(&total_blocks.to_le_bytes());
        header.extend_from_slice(&used_blocks.to_le_bytes());
        header.extend_from_slice(&block_size.to_le_bytes());
        header.extend_from_slice(&0u32.to_le_bytes());

        let mut bitmap = vec![0u8; 32];
        bitmap[0] = 0b00000011;
        header.extend_from_slice(&bitmap);

        let mut data = vec![0u8; 2 * block_size as usize];
        data[1024 + 0x38] = 0x53;
        data[1024 + 0x39] = 0xEF;
        header.extend_from_slice(&data);

        // Compress raw partclone header with zstd
        let compressed = zstd::encode_all(&header[..], 3).unwrap();

        let mut tmp = NamedTempFile::new().unwrap();
        tmp.write_all(&compressed).unwrap();
        tmp.flush().unwrap();

        let path = tmp.path();
        let reader = PartcloneReader::open_path(path).unwrap();
        assert_eq!(reader.virtual_size(), 1048576);
        assert_eq!(reader.get_fs_type(), DiskFsType::Ext4);
    }

    #[test]
    #[ignore]
    fn test_real_fog_w10_images() {
        let fog_path = "/home/bolt/projects/Test/test-images/w10fog";
        if !std::path::Path::new(fog_path).exists() {
            return;
        }

        let partitions = DiskImageHandler::probe_image(fog_path).unwrap();
        assert_eq!(partitions.len(), 4);
        println!("Partitions: {:?}", partitions.iter().map(|p| p.slug()).collect::<Vec<_>>());

        // Test listing partition 1 (FAT32 EFI)
        let p1_listing = DiskImageHandler::list_image_contents(fog_path, &partitions[0].slug()).unwrap();
        println!("P1 entries: {:?}", p1_listing.entries.iter().map(|e| &e.name).collect::<Vec<_>>());
        assert!(p1_listing.entries.iter().any(|e| e.name.to_lowercase() == "efi"));

        // Test listing partition 4 (NTFS Recovery)
        let p4_listing = DiskImageHandler::list_image_contents(fog_path, &partitions[3].slug()).unwrap();
        println!("P4 entries: {:?}", p4_listing.entries.iter().map(|e| &e.name).collect::<Vec<_>>());
        assert!(p4_listing.entries.iter().any(|e| e.name.to_lowercase() == "recovery"));

        // Test listing subfolder in partition 4 (Recovery)
        let p4_sub = format!("{}/Recovery", partitions[3].slug());
        let sub_listing = DiskImageHandler::list_image_contents(fog_path, &p4_sub).unwrap();
        println!("P4 Recovery/ entries: {:?}", sub_listing.entries.iter().map(|e| &e.name).collect::<Vec<_>>());
    }
}
