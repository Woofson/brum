//! Compressed disk and CD images (#159): `.iso.zst`, `.img.gz`, `.raw.xz`, `.dd.bz2`, …
//!
//! Brum unpacks the image once into its cache folder (with progress and cancel in Tasks), then
//! looks at what came out: a partclone/FOG backup, an ISO 9660 / UDF disc, a SquashFS image, or a
//! disk (partition table or bare filesystem). The unpacked copy is reused until the original
//! changes, and copies nobody opened for a while are removed (the admin sets how long).

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

/// Days an unpacked copy may go unused before it is removed. 0 = only when the admin clears it.
static RETENTION_DAYS: AtomicU32 = AtomicU32::new(DEFAULT_RETENTION_DAYS);
pub const DEFAULT_RETENTION_DAYS: u32 = 7;
/// Database key for the admin setting
pub const RETENTION_SETTING_KEY: &str = "image_cache_retention_days";

pub fn retention_days() -> u32 {
    RETENTION_DAYS.load(Ordering::Relaxed)
}

pub fn set_retention_days(days: u32) {
    RETENTION_DAYS.store(days.min(3650), Ordering::Relaxed);
}

/// Reads the admin's setting, cleans up once, and keeps cleaning up every few hours
pub fn start_cache_upkeep(auth: &crate::auth::AuthManager) {
    if let Some(days) = auth.get_server_setting(RETENTION_SETTING_KEY).and_then(|v| v.parse::<u32>().ok()) {
        set_retention_days(days);
    }
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        std::thread::Builder::new()
            .name("image-cache-upkeep".into())
            .spawn(|| loop {
                let _ = cleanup_expired();
                std::thread::sleep(std::time::Duration::from_secs(6 * 3600));
            })
            .ok();
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    Zstd,
    Gzip,
    Xz,
    Bzip2,
}

/// What an unpacked image turned out to be
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InnerImage {
    Partclone,
    Disc,
    Squashfs,
    Disk,
}

/// The folder unpacked images are kept in (the name predates CD and disk images)
pub fn cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("brum")
        .join("partclone_cache")
}

/// Cache file names for one version of one source file (path, size and change time)
pub fn cache_paths(source_path: &Path, file_len: u64, mtime: u64) -> (PathBuf, PathBuf) {
    use sha2::{Digest, Sha256};
    let dir = cache_dir();
    let _ = std::fs::create_dir_all(&dir);
    let mut hasher = Sha256::new();
    hasher.update(source_path.to_string_lossy().as_bytes());
    hasher.update(file_len.to_le_bytes());
    hasher.update(mtime.to_le_bytes());
    let hash_hex = hex::encode(hasher.finalize());
    let prefix = &hash_hex[..24];
    (
        dir.join(format!("partclone_{}.raw", prefix)),
        dir.join(format!("partclone_{}.partial", prefix)),
    )
}

/// Names that may hold a compressed disk or CD image. A plain `.gz` is left alone (it is
/// usually a single compressed file, and `.tar.gz` is an archive).
pub fn is_compressed_image_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    if lower.ends_with(".zst") || lower.ends_with(".zstd") {
        return true;
    }
    for comp in [".gz", ".xz", ".bz2"] {
        if let Some(stem) = lower.strip_suffix(comp) {
            if [".iso", ".img", ".raw", ".dd", ".bin"].iter().any(|ext| stem.ends_with(ext)) {
                return true;
            }
        }
    }
    false
}

/// The compression a file starts with, if any
pub fn detect_compression(file: &mut File) -> std::io::Result<Option<Compression>> {
    let mut magic = [0u8; 6];
    file.seek(SeekFrom::Start(0))?;
    let n = file.read(&mut magic)?;
    file.seek(SeekFrom::Start(0))?;
    Ok(if n >= 4 && magic[..4] == [0x28, 0xB5, 0x2F, 0xFD] {
        Some(Compression::Zstd)
    } else if n >= 2 && magic[..2] == [0x1F, 0x8B] {
        Some(Compression::Gzip)
    } else if n >= 6 && magic == [0xFD, b'7', b'z', b'X', b'Z', 0x00] {
        Some(Compression::Xz)
    } else if n >= 3 && &magic[..3] == b"BZh" {
        Some(Compression::Bzip2)
    } else {
        None
    })
}

/// Looks at the start of an unpacked image to see what it holds
pub fn probe_inner(raw: &Path) -> std::io::Result<InnerImage> {
    let mut f = File::open(raw)?;
    let mut head = [0u8; 16];
    let n = f.read(&mut head)?;
    let head = &head[..n];
    if head.starts_with(b"partclone-image") || head.starts_with(b"PARTCLONE") {
        return Ok(InnerImage::Partclone);
    }
    if head.starts_with(b"hsqs") {
        return Ok(InnerImage::Squashfs);
    }
    // ISO 9660 ("CD001") or UDF ("BEA01" / "NSR0x") volume descriptors start at sector 16
    let mut vd = [0u8; 6];
    if f.seek(SeekFrom::Start(32768)).is_ok() && f.read_exact(&mut vd).is_ok() {
        let id = &vd[1..6];
        if id == b"CD001" || id == b"BEA01" || id == b"NSR02" || id == b"NSR03" {
            return Ok(InnerImage::Disc);
        }
    }
    Ok(InnerImage::Disk)
}

struct CountingReader<R> {
    inner: R,
    bytes_read: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl<R: Read> Read for CountingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.bytes_read.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
}

/// Marks a cached copy as just used, so the cleanup keeps it
fn touch(path: &Path) {
    if let Ok(f) = std::fs::OpenOptions::new().write(true).open(path) {
        let _ = f.set_modified(std::time::SystemTime::now());
    }
}

/// Every unpack the tests started, so they can count them
#[cfg(test)]
static UNPACKED: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());

/// The lock for one cache entry, shared by everyone unpacking that image
fn unpack_gate(raw_path: &Path) -> std::sync::Arc<std::sync::Mutex<()>> {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, OnceLock};
    static GATES: OnceLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> = OnceLock::new();
    let mut gates = GATES.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    // Forget gates nobody holds any more, so the list doesn't grow
    gates.retain(|_, g| Arc::strong_count(g) > 1);
    gates.entry(raw_path.to_path_buf()).or_default().clone()
}

/// Room kept free on the cache disk after unpacking
const SPACE_MARGIN: u64 = 1024 * 1024 * 1024;

/// How big the unpacked copy will be, as far as the start of it tells. Partclone/FOG backups
/// can be measured from their block map; for anything else the compressed size is the least it needs.
fn expected_size(file: &mut File, compression: Compression, file_len: u64) -> u64 {
    let size = (|| -> std::io::Result<Option<u64>> {
        file.seek(SeekFrom::Start(0))?;
        let mut decoder: Box<dyn Read + '_> = match compression {
            Compression::Zstd => Box::new(zstd::stream::read::Decoder::new(&mut *file)?),
            Compression::Gzip => Box::new(flate2::read::MultiGzDecoder::new(&mut *file)),
            Compression::Xz => Box::new(liblzma::read::XzDecoder::new_multi_decoder(&mut *file)),
            Compression::Bzip2 => Box::new(bzip2::read::MultiBzDecoder::new(&mut *file)),
        };
        partclone_size(&mut decoder)
    })();
    let _ = file.seek(SeekFrom::Start(0));
    match size {
        Ok(Some(size)) => size.max(file_len),
        _ => file_len,
    }
}

/// Largest block map read to measure a partclone image (enough for a 2 TB volume in 4 KB blocks)
const MAX_BITMAP: u64 = 64 * 1024 * 1024;

/// The size of an unpacked partclone image, from its header and block map. `None` if it isn't one.
fn partclone_size(r: &mut dyn Read) -> std::io::Result<Option<u64>> {
    let mut head = [0u8; 0x5c];
    if r.read_exact(&mut head).is_err() {
        return Ok(None);
    }
    if !head.starts_with(b"partclone-image") && !head.starts_with(b"PARTCLONE") {
        return Ok(None);
    }
    let u64_at = |at: usize| u64::from_le_bytes(head[at..at + 8].try_into().unwrap());
    let u32_at = |at: usize| u32::from_le_bytes(head[at..at + 4].try_into().unwrap()) as u64;
    if !(head[16..18].starts_with(b"0.") || head[16..18].starts_with(b"1.")) {
        // Older header (partclone 0.2): used blocks, each followed by a 4-byte checksum
        return Ok(u64_at(0x30).checked_mul(u32_at(0x38) + 4));
    }
    // Newer header (partclone 0.3 and later). Its "used blocks" field is often 0 in FOG images,
    // so the used blocks are counted in the block map, one bit per block.
    let (total, block, options) = (u64_at(0x3c), u32_at(0x54), u32_at(0x58));
    let map_len = total.div_ceil(8);
    if map_len > MAX_BITMAP || options > 4096 {
        return Ok(None);
    }
    let mut skip = vec![0u8; options as usize]; // options and the header's checksum
    r.read_exact(&mut skip)?;
    let mut map = vec![0u8; map_len as usize];
    r.read_exact(&mut map)?;
    let used: u64 = map.iter().map(|b| b.count_ones() as u64).sum();
    // header + map + the map's checksum + the data
    Ok(used.checked_mul(block).map(|data| 0x5c + options + map_len + 4 + data))
}

/// Refuses to start an unpack that would fill the disk the cache is on
fn check_space(file: &mut File, compression: Compression, file_len: u64, dir: &Path) -> std::io::Result<()> {
    let _ = std::fs::create_dir_all(dir);
    let Some(free) = free_space(dir) else { return Ok(()) };
    let needed = expected_size(file, compression, file_len);
    if needed.saturating_add(SPACE_MARGIN) > free {
        let size = crate::tools::disk_usage::format_size;
        return Err(std::io::Error::new(
            std::io::ErrorKind::StorageFull,
            format!(
                "Not enough free space to open this image: it unpacks to about {}, and the disk with Brum's cache ({}) has {} free. Free up space there, or clear unpacked images in Settings.",
                size(needed), dir.display(), size(free)
            ),
        ));
    }
    Ok(())
}

fn free_space(dir: &Path) -> Option<u64> {
    let path = dir.to_str()?;
    #[cfg(unix)]
    { crate::tools::disk_usage::query_statvfs(path).map(|(_, _, available, _)| available) }
    #[cfg(windows)]
    { crate::tools::disk_usage::query_windows_disk(path).map(|(_, _, available)| available) }
}

/// Unpacks a compressed file into the cache (once) and returns the unpacked copy's path.
/// `source_path` names the cache entry and the Tasks entry; without it a one-off name is used.
pub fn unpack_to_cache(mut file: File, source_path: Option<&Path>, compression: Compression) -> std::io::Result<PathBuf> {
    let meta = file.metadata().ok();
    let file_len = meta.as_ref().map(|m| m.len()).unwrap_or(0);
    let mtime = meta.as_ref()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let (raw_path, partial_path) = match source_path {
        Some(p) => cache_paths(p, file_len, mtime),
        None => {
            let dir = cache_dir();
            let _ = std::fs::create_dir_all(&dir);
            let id = uuid::Uuid::new_v4();
            (dir.join(format!("partclone_{}.raw", id)), dir.join(format!("partclone_{}.partial", id)))
        }
    };

    if raw_path.metadata().map(|m| m.len() > 0).unwrap_or(false) {
        touch(&raw_path);
        return Ok(raw_path);
    }

    // One unpack per image at a time: a second request waits for the first and then uses its copy
    // (two unpacks writing the same file at once would spoil it)
    let gate = source_path.map(|_| unpack_gate(&raw_path));
    let _held = gate.as_ref().map(|g| g.lock().unwrap_or_else(|e| e.into_inner()));
    if raw_path.metadata().map(|m| m.len() > 0).unwrap_or(false) {
        touch(&raw_path);
        return Ok(raw_path);
    }

    check_space(&mut file, compression, file_len, raw_path.parent().unwrap_or(Path::new(".")))?;

    let task_mgr = crate::tools::tasks::get_global_task_manager();
    let display_name = source_path
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("compressed image")
        .to_string();
    let source_label = source_path.map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|| display_name.clone());
    let task_id = task_mgr.as_ref().map(|tm| {
        tm.sync_create_task(&format!("Decompressing {}", display_name), "decompress", &source_label, &raw_path.to_string_lossy(), file_len)
    });

    #[cfg(test)]
    UNPACKED.lock().unwrap().push(raw_path.clone());

    let result = (|| -> std::io::Result<()> {
        let mut out_file = std::fs::OpenOptions::new().create(true).write(true).truncate(true).open(&partial_path)?;
        file.seek(SeekFrom::Start(0))?;
        let counter = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let counting = CountingReader { inner: &mut file, bytes_read: counter.clone() };
        let mut decoder: Box<dyn Read + '_> = match compression {
            Compression::Zstd => Box::new(zstd::stream::read::Decoder::new(counting)?),
            Compression::Gzip => Box::new(flate2::read::MultiGzDecoder::new(counting)),
            Compression::Xz => Box::new(liblzma::read::XzDecoder::new_multi_decoder(counting)),
            Compression::Bzip2 => Box::new(bzip2::read::MultiBzDecoder::new(counting)),
        };

        let mut chunk = vec![0u8; 4 * 1024 * 1024];
        let start = std::time::Instant::now();
        let mut last_update = std::time::Instant::now();
        loop {
            let n = decoder.read(&mut chunk)?;
            if n == 0 {
                break;
            }
            std::io::Write::write_all(&mut out_file, &chunk[..n])?;
            if last_update.elapsed().as_millis() >= 250 {
                last_update = std::time::Instant::now();
                let done = counter.load(Ordering::Relaxed).min(file_len);
                let secs = start.elapsed().as_secs_f64();
                let speed = if secs > 0.0 { (done as f64 / secs) as u64 } else { 0 };
                if let (Some(tm), Some(tid)) = (&task_mgr, &task_id) {
                    tm.sync_update_stream_progress(tid, Some(&display_name), done, file_len, 0, 1, done, speed);
                    if tm.sync_is_cancelled(tid) {
                        return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "Decompression cancelled by user"));
                    }
                }
            }
        }
        std::io::Write::flush(&mut out_file)?;
        drop(out_file);
        std::fs::rename(&partial_path, &raw_path)?;
        Ok(())
    })();

    match result {
        Ok(()) => {
            if let (Some(tm), Some(tid)) = (&task_mgr, &task_id) {
                tm.sync_complete_task(tid);
            }
            // A good moment to drop copies nobody uses any more
            std::thread::spawn(|| { let _ = cleanup_expired(); });
            Ok(raw_path)
        }
        Err(err) => {
            let _ = std::fs::remove_file(&partial_path);
            if let (Some(tm), Some(tid)) = (&task_mgr, &task_id) {
                tm.sync_fail_task(tid, &err.to_string());
            }
            Err(err)
        }
    }
}

/// An unpacked compressed image that Brum should open in place of the original file
pub struct UnpackedImage {
    pub raw_path: String,
    pub inner: InnerImage,
}

/// For a compressed disk or CD image: unpacks it (once) and says what is inside.
/// `None` for files that are not compressed images, and for partclone/FOG backups named
/// `.zst`/`.zstd`, which the disk image code already opens itself.
pub fn resolve(archive_path: &Path) -> std::io::Result<Option<UnpackedImage>> {
    let name = archive_path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if !archive_path.is_file() || !is_compressed_image_name(name) {
        return Ok(None);
    }
    let mut file = File::open(archive_path)?;
    let Some(compression) = detect_compression(&mut file)? else {
        return Ok(None);
    };
    let raw = unpack_to_cache(file, Some(archive_path), compression)?;
    let inner = probe_inner(&raw)?;
    let lower = name.to_lowercase();
    if inner == InnerImage::Partclone && (lower.ends_with(".zst") || lower.ends_with(".zstd")) {
        return Ok(None);
    }
    Ok(Some(UnpackedImage { raw_path: raw.to_string_lossy().to_string(), inner }))
}

/// Shows the original file's name in addresses that were built from the unpacked copy
pub fn rewrite_listing(mut listing: super::DirectoryListing, raw: &str, original: &str) -> super::DirectoryListing {
    let from = format!("archive://{raw}#");
    let to = format!("archive://{original}#");
    let fix = |s: &str| -> String { s.replace(&from, &to).replace(raw, original) };
    listing.current_path = fix(&listing.current_path);
    let raw_parent = Path::new(raw).parent().map(|p| p.to_string_lossy().to_string());
    let original_parent = Path::new(original).parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|| "/".to_string());
    listing.parent_path = listing.parent_path.map(|p| {
        if Some(&p) == raw_parent.as_ref() { original_parent.clone() } else { fix(&p) }
    });
    for e in &mut listing.entries {
        e.path = fix(&e.path);
    }
    listing
}

pub fn rewrite_content(mut resp: super::FileContentResponse, raw: &str, original: &str) -> super::FileContentResponse {
    resp.path = resp.path.replace(raw, original);
    resp
}

/// Size and number of unpacked copies in the cache
pub fn cache_stats() -> (u64, u64) {
    let mut bytes = 0;
    let mut count = 0;
    if let Ok(rd) = std::fs::read_dir(cache_dir()) {
        for e in rd.flatten() {
            if let Ok(m) = e.metadata() {
                if m.is_file() {
                    bytes += m.len();
                    count += 1;
                }
            }
        }
    }
    (bytes, count)
}

fn is_cache_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with("partclone_") && (n.ends_with(".raw") || n.ends_with(".partial")))
}

/// Removes unpacked copies nobody used for longer than the retention time.
/// Unfinished unpacks are only removed when a day old (one may still be running).
pub fn cleanup_expired() -> std::io::Result<u64> {
    let days = retention_days();
    let now = std::time::SystemTime::now();
    let mut removed = 0;
    let Ok(rd) = std::fs::read_dir(cache_dir()) else { return Ok(0) };
    for e in rd.flatten() {
        let path = e.path();
        if !is_cache_file(&path) {
            continue;
        }
        let Ok(meta) = e.metadata() else { continue };
        let age = meta.modified().ok().and_then(|m| now.duration_since(m).ok()).unwrap_or_default();
        let is_partial = path.extension().is_some_and(|x| x == "partial");
        let expired = if is_partial {
            age.as_secs() > 24 * 3600
        } else {
            days > 0 && age.as_secs() > u64::from(days) * 24 * 3600
        };
        if expired && std::fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    if removed > 0 {
        tracing::info!(removed, days, "Removed unused unpacked images from the cache");
    }
    Ok(removed)
}

/// Removes every finished unpacked copy now (unfinished unpacks are left to finish)
pub fn clear_cache() -> std::io::Result<u64> {
    let mut removed = 0;
    let Ok(rd) = std::fs::read_dir(cache_dir()) else { return Ok(0) };
    for e in rd.flatten() {
        let path = e.path();
        if is_cache_file(&path) && path.extension().is_some_and(|x| x == "raw") && std::fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A partclone 0.3 header saying `used` blocks of `block` bytes
    /// with a block map of `total` blocks, of which the first `used` are in use
    fn partclone_head(total: u64, used: u64, block: u32) -> Vec<u8> {
        let mut h = vec![0u8; 0x6e];
        h[..15].copy_from_slice(b"partclone-image");
        h[16..22].copy_from_slice(b"0.3.33");
        h[0x3c..0x44].copy_from_slice(&total.to_le_bytes());
        // the "used blocks" field stays 0, as in real FOG images
        h[0x54..0x58].copy_from_slice(&block.to_le_bytes());
        h[0x58..0x5c].copy_from_slice(&18u32.to_le_bytes());
        let mut map = vec![0u8; total.div_ceil(8) as usize];
        for i in 0..used as usize { map[i / 8] |= 1 << (i % 8); }
        h.extend(map);
        h.extend([0u8; 4]);
        h
    }

    #[test]
    fn size_of_a_compressed_partclone_backup_is_counted_from_its_block_map() {
        let mut head = partclone_head(1_000_000, 300_001, 4096);
        head.extend(vec![0u8; 4096]);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d1p3.img");
        std::fs::write(&path, zstd::bulk::compress(&head, 3).unwrap()).unwrap();
        let mut f = File::open(&path).unwrap();
        let len = f.metadata().unwrap().len();
        assert_eq!(expected_size(&mut f, Compression::Zstd, len), 0x6e + 125_000 + 4 + 300_001 * 4096);
        // Anything else needs at least its compressed size
        std::fs::write(&path, zstd::bulk::compress(b"plain data", 3).unwrap()).unwrap();
        let mut f = File::open(&path).unwrap();
        let len = f.metadata().unwrap().len();
        assert_eq!(expected_size(&mut f, Compression::Zstd, len), len);
    }

    #[test]
    fn refuses_to_unpack_what_would_fill_the_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("huge.img");
        // 10 million blocks of 1 MB (10 TB): far more than any test disk holds
        std::fs::write(&path, zstd::bulk::compress(&partclone_head(10_000_000, 10_000_000, 1 << 20), 3).unwrap()).unwrap();
        let mut f = File::open(&path).unwrap();
        let len = f.metadata().unwrap().len();
        let err = check_space(&mut f, Compression::Zstd, len, dir.path()).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::StorageFull);
        assert!(err.to_string().contains("Not enough free space"));
        // A small image is fine
        std::fs::write(&path, zstd::bulk::compress(&partclone_head(10, 10, 4096), 3).unwrap()).unwrap();
        let mut f = File::open(&path).unwrap();
        let len = f.metadata().unwrap().len();
        assert!(check_space(&mut f, Compression::Zstd, len, dir.path()).is_ok());
    }

    #[test]
    fn opening_the_same_image_twice_at_once_unpacks_it_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("disk.img.zst");
        let data: Vec<u8> = (0..64 * 1024 * 1024u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&path, zstd::bulk::compress(&data, 1).unwrap()).unwrap();
        let start = std::sync::Arc::new(std::sync::Barrier::new(4));
        let threads: Vec<_> = (0..4).map(|_| {
            let (p, start) = (path.clone(), start.clone());
            std::thread::spawn(move || {
                start.wait();
                unpack_to_cache(File::open(&p).unwrap(), Some(&p), Compression::Zstd).unwrap()
            })
        }).collect();
        let raws: Vec<PathBuf> = threads.into_iter().map(|t| t.join().unwrap()).collect();
        assert!(raws.iter().all(|r| r == &raws[0]));
        assert_eq!(UNPACKED.lock().unwrap().iter().filter(|r| *r == &raws[0]).count(), 1, "unpacked more than once");
        assert!(std::fs::read(&raws[0]).unwrap() == data);
        let _ = std::fs::remove_file(&raws[0]);
    }

    #[test]
    fn names_that_may_hold_compressed_images() {
        for yes in ["a.iso.zst", "B.IMG.GZ", "disk.raw.xz", "sd.dd.bz2", "d1p1.zst", "x.zstd", "game.bin.gz"] {
            assert!(is_compressed_image_name(yes), "{yes}");
        }
        for no in ["notes.txt.gz", "src.tar.gz", "src.tar.xz", "a.iso", "logs.gz", "x.bz2"] {
            assert!(!is_compressed_image_name(no), "{no}");
        }
    }

    #[test]
    fn listing_addresses_use_the_original_name() {
        let raw = "/cache/partclone_abc.raw";
        let original = "/home/bolt/old/cd.iso.zst";
        let listing = super::super::DirectoryListing {
            current_path: format!("archive://{raw}#docs"),
            parent_path: Some(format!("archive://{raw}#")),
            entries: vec![],
            total_files: 0,
            total_dirs: 0,
            total_size: 0,
            protocol: "archive".into(),
            is_truncated: None,
            max_limit: None,
        };
        let l = rewrite_listing(listing, raw, original);
        assert_eq!(l.current_path, format!("archive://{original}#docs"));
        assert_eq!(l.parent_path.unwrap(), format!("archive://{original}#"));
        let root = super::super::DirectoryListing {
            current_path: format!("archive://{raw}#"),
            parent_path: Some("/cache".into()),
            entries: vec![],
            total_files: 0,
            total_dirs: 0,
            total_size: 0,
            protocol: "archive".into(),
            is_truncated: None,
            max_limit: None,
        };
        assert_eq!(rewrite_listing(root, raw, original).parent_path.unwrap(), "/home/bolt/old");
    }
}
