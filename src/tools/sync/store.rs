//! Where backups read from and write to.
//!
//! The engine only talks to the `Store` trait, so local folders (including mounted NAS
//! shares) and remote locations share the same backup, restore and logging logic.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::UNIX_EPOCH;
use walkdir::WalkDir;

/// Size and last-modified time (seconds) of one file
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileMeta {
    pub size: u64,
    pub mtime: i64,
}

impl FileMeta {
    /// Same size and a modified time within 2 seconds (FAT and some network shares round times)
    pub fn same_as(&self, other: &FileMeta) -> bool {
        self.size == other.size && (self.mtime - other.mtime).abs() <= 2
    }
}

/// Every file below a folder, keyed by its path relative to that folder (`/` between parts)
pub type Listing = BTreeMap<String, FileMeta>;

/// One entry when browsing a folder
#[derive(Debug, Clone, Serialize)]
pub struct DirItem {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub mtime: i64,
}

/// Unfinished copies end in this; they are never treated as real files
pub const PART_SUFFIX: &str = ".brum-part";

/// The temporary name a file is written under before it is swapped in: `dir/.name.brum-part`
pub fn part_name(rel: &str) -> String {
    match rel.rsplit_once('/') {
        Some((dir, name)) => format!("{dir}/.{name}{PART_SUFFIX}"),
        None => format!(".{rel}{PART_SUFFIX}"),
    }
}

/// A place in rclone's words: the "fs" (`remote:` or a local root) and the path inside it
pub type RcloneLocation = (String, String);

pub trait Store: Send + Sync {
    /// Human readable location, without passwords
    fn label(&self) -> String;
    fn root_exists(&self) -> bool;
    fn ensure_root(&self) -> Result<(), String>;
    /// All files below `base` ("" for the top), keyed relative to `base`.
    /// `skip(rel, is_dir)` returning true leaves a file out, or a whole folder.
    fn list(&self, base: &str, skip: &dyn Fn(&str, bool) -> bool, cancel: &AtomicBool, warnings: &mut Vec<String>) -> Result<Listing, String>;
    /// One folder level, for browsing
    fn list_dir(&self, rel: &str) -> Result<Vec<DirItem>, String>;
    fn stat(&self, rel: &str) -> Option<FileMeta>;
    fn is_dir(&self, rel: &str) -> bool;
    fn open_read(&self, rel: &str) -> Result<Box<dyn Read + Send>, String>;
    /// Writes `data` to a temporary file beside `rel`, sets its modified time, returns its path.
    /// `on_bytes` is told how much was written so far; returning false stops the copy.
    fn write_part(&self, rel: &str, data: &mut dyn Read, mtime: i64, on_bytes: &mut dyn FnMut(u64) -> bool) -> Result<String, String>;
    /// Moves a file, creating folders above `to`; replaces an existing `to`
    fn rename(&self, from: &str, to: &str) -> Result<(), String>;
    fn remove_file(&self, rel: &str) -> Result<(), String>;
    fn remove_dir_all(&self, rel: &str) -> Result<(), String>;
    /// Removes folders above `rel` that became empty, up to (not including) the top
    fn remove_empty_parents(&self, rel: &str);
    /// The real path for local stores, for fast local-only work (partial updates, permissions)
    fn local_path(&self, rel: &str) -> Option<PathBuf>;
    /// This place in rclone's words, so rclone can copy straight to or from it
    fn rclone_location(&self, _rel: &str) -> Option<RcloneLocation> {
        None
    }
    /// Copies one file with rclone (keeping its time). None when this store has no rclone.
    fn rclone_copy(&self, _src: &RcloneLocation, _dst: &RcloneLocation) -> Option<Result<(), String>> {
        None
    }
    /// For several sources shown as one: the name of each folder at the top
    fn part_names(&self) -> Vec<String> {
        Vec::new()
    }
    /// Proton Drive, which the engine feeds in batches
    fn proton(&self) -> Option<&super::proton_store::ProtonStore> {
        None
    }
}

/// Rejects paths that could leave the backup (`..`, absolute paths, drive letters)
pub fn safe_rel(rel: &str) -> Result<String, String> {
    let norm = rel.replace('\\', "/");
    let mut parts = Vec::new();
    for part in norm.split('/') {
        match part {
            "" | "." => continue,
            ".." => return Err(format!("Invalid path: {rel}")),
            p if p.contains(':') && cfg!(windows) => return Err(format!("Invalid path: {rel}")),
            p => parts.push(p),
        }
    }
    Ok(parts.join("/"))
}

/// `a/b` + `c` = `a/b/c`
pub fn join_rel(base: &str, rel: &str) -> String {
    match (base.trim_matches('/'), rel.trim_matches('/')) {
        ("", r) => r.to_string(),
        (b, "") => b.to_string(),
        (b, r) => format!("{b}/{r}"),
    }
}

fn mtime_secs(meta: &fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// A folder on this machine; also mounted Samba/CIFS and NFS shares
pub struct LocalStore {
    root: PathBuf,
}

impl LocalStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn path(&self, rel: &str) -> PathBuf {
        let mut p = self.root.clone();
        for part in rel.split('/').filter(|p| !p.is_empty()) {
            p.push(part);
        }
        p
    }

    fn rel_of(&self, path: &Path) -> Option<String> {
        let rel = path.strip_prefix(&self.root).ok()?;
        let mut parts = Vec::new();
        for c in rel.components() {
            parts.push(c.as_os_str().to_str()?.to_string());
        }
        Some(parts.join("/"))
    }
}

impl Store for LocalStore {
    fn label(&self) -> String {
        self.root.display().to_string()
    }

    fn root_exists(&self) -> bool {
        self.root.is_dir()
    }

    fn ensure_root(&self) -> Result<(), String> {
        fs::create_dir_all(&self.root).map_err(|e| format!("Cannot create {}: {e}", self.root.display()))
    }

    fn list(&self, base: &str, skip: &dyn Fn(&str, bool) -> bool, cancel: &AtomicBool, warnings: &mut Vec<String>) -> Result<Listing, String> {
        let start = self.path(base);
        if !start.exists() {
            return Ok(Listing::new());
        }
        if start.is_file() {
            let meta = fs::metadata(&start).map_err(|e| e.to_string())?;
            let name = start.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
            let mut one = Listing::new();
            one.insert(name, FileMeta { size: meta.len(), mtime: mtime_secs(&meta) });
            return Ok(one);
        }
        let base_store = LocalStore::new(start.clone());
        let mut out = Listing::new();
        let walker = WalkDir::new(&start).follow_links(false).into_iter().filter_entry(|e| {
            if e.depth() == 0 {
                return true;
            }
            match base_store.rel_of(e.path()) {
                Some(rel) => !skip(&rel, e.file_type().is_dir()),
                None => true, // reported below
            }
        });
        for entry in walker {
            if cancel.load(Ordering::Relaxed) {
                return Err("Cancelled".to_string());
            }
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    let what = e.path().and_then(|p| base_store.rel_of(p)).unwrap_or_default();
                    warnings.push(format!("Could not read {what}: {}", e.io_error().map(|io| io.to_string()).unwrap_or_else(|| e.to_string())));
                    continue;
                }
            };
            if entry.depth() == 0 || entry.file_type().is_dir() {
                continue;
            }
            let Some(rel) = base_store.rel_of(entry.path()) else {
                warnings.push(format!("Skipped a file whose name is not valid text: {}", entry.path().display()));
                continue;
            };
            if entry.file_type().is_symlink() {
                warnings.push(format!("Skipped link: {rel}"));
                continue;
            }
            if !entry.file_type().is_file() {
                continue; // sockets, devices, pipes
            }
            match entry.metadata() {
                Ok(meta) => {
                    out.insert(rel, FileMeta { size: meta.len(), mtime: mtime_secs(&meta) });
                }
                Err(e) => warnings.push(format!("Could not read {rel}: {e}")),
            }
        }
        Ok(out)
    }

    fn list_dir(&self, rel: &str) -> Result<Vec<DirItem>, String> {
        let dir = self.path(rel);
        let rd = fs::read_dir(&dir).map_err(|e| format!("Cannot open {}: {e}", dir.display()))?;
        let mut items = Vec::new();
        for ent in rd.flatten() {
            let Some(name) = ent.file_name().to_str().map(|s| s.to_string()) else { continue };
            let Ok(ft) = ent.file_type() else { continue };
            if ft.is_symlink() {
                continue;
            }
            let meta = ent.metadata().ok();
            items.push(DirItem {
                name,
                is_dir: ft.is_dir(),
                size: if ft.is_dir() { 0 } else { meta.as_ref().map(|m| m.len()).unwrap_or(0) },
                mtime: meta.as_ref().map(mtime_secs).unwrap_or(0),
            });
        }
        items.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        Ok(items)
    }

    fn stat(&self, rel: &str) -> Option<FileMeta> {
        let meta = fs::symlink_metadata(self.path(rel)).ok()?;
        meta.is_file().then(|| FileMeta { size: meta.len(), mtime: mtime_secs(&meta) })
    }

    fn is_dir(&self, rel: &str) -> bool {
        self.path(rel).is_dir()
    }

    fn open_read(&self, rel: &str) -> Result<Box<dyn Read + Send>, String> {
        File::open(self.path(rel)).map(|f| Box::new(f) as Box<dyn Read + Send>).map_err(|e| e.to_string())
    }

    fn write_part(&self, rel: &str, data: &mut dyn Read, mtime: i64, on_bytes: &mut dyn FnMut(u64) -> bool) -> Result<String, String> {
        let target = self.path(rel);
        let parent = target.parent().ok_or("Invalid path")?.to_path_buf();
        fs::create_dir_all(&parent).map_err(|e| format!("Cannot create folder: {e}"))?;
        let part_rel = part_name(rel);
        let part_path = self.path(&part_rel);
        let result = (|| -> Result<(), String> {
            let mut out = File::create(&part_path).map_err(|e| e.to_string())?;
            let mut buf = vec![0u8; 1024 * 1024];
            let mut written = 0u64;
            loop {
                let n = data.read(&mut buf).map_err(|e| format!("Read failed: {e}"))?;
                if n == 0 {
                    break;
                }
                out.write_all(&buf[..n]).map_err(|e| format!("Write failed: {e}"))?;
                written += n as u64;
                if !on_bytes(written) {
                    return Err("Cancelled".to_string());
                }
            }
            out.flush().map_err(|e| e.to_string())?;
            drop(out);
            filetime::set_file_mtime(&part_path, filetime::FileTime::from_unix_time(mtime, 0)).map_err(|e| e.to_string())
        })();
        match result {
            Ok(()) => Ok(part_rel),
            Err(e) => {
                let _ = fs::remove_file(&part_path);
                Err(e)
            }
        }
    }

    fn rename(&self, from: &str, to: &str) -> Result<(), String> {
        let (src, dst) = (self.path(from), self.path(to));
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("Cannot create folder: {e}"))?;
        }
        fs::rename(&src, &dst).map_err(|e| format!("Cannot move {from}: {e}"))
    }

    fn remove_file(&self, rel: &str) -> Result<(), String> {
        fs::remove_file(self.path(rel)).map_err(|e| e.to_string())
    }

    fn remove_dir_all(&self, rel: &str) -> Result<(), String> {
        fs::remove_dir_all(self.path(rel)).map_err(|e| e.to_string())
    }

    fn remove_empty_parents(&self, rel: &str) {
        let mut cur = rel;
        while let Some((dir, _)) = cur.rsplit_once('/') {
            if fs::remove_dir(self.path(dir)).is_err() {
                break; // not empty (or already gone)
            }
            cur = dir;
        }
    }

    fn local_path(&self, rel: &str) -> Option<PathBuf> {
        Some(self.path(rel))
    }

    #[cfg(unix)]
    fn rclone_location(&self, rel: &str) -> Option<RcloneLocation> {
        let full = self.path(rel);
        let s = full.to_str()?;
        Some(("/".to_string(), s.trim_start_matches('/').to_string()))
    }
}

/// Updates only the parts of `dst` that differ from `src` (both local). Returns bytes written.
pub fn update_changed_blocks(src: &Path, dst: &Path, mtime: i64, cancel: &AtomicBool, on_bytes: &mut dyn FnMut(u64) -> bool) -> Result<u64, String> {
    const BLOCK: usize = 1024 * 1024;
    let mut s = File::open(src).map_err(|e| e.to_string())?;
    let mut d = OpenOptions::new().read(true).write(true).open(dst).map_err(|e| e.to_string())?;
    let src_len = s.metadata().map_err(|e| e.to_string())?.len();
    let (mut sb, mut db) = (vec![0u8; BLOCK], vec![0u8; BLOCK]);
    let (mut offset, mut written) = (0u64, 0u64);
    while offset < src_len {
        if cancel.load(Ordering::Relaxed) {
            return Err("Cancelled".to_string());
        }
        let n = read_full(&mut s, &mut sb).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        d.seek(SeekFrom::Start(offset)).map_err(|e| e.to_string())?;
        let m = read_full(&mut d, &mut db[..n]).map_err(|e| e.to_string())?;
        if m != n || sb[..n] != db[..n] {
            d.seek(SeekFrom::Start(offset)).map_err(|e| e.to_string())?;
            d.write_all(&sb[..n]).map_err(|e| e.to_string())?;
            written += n as u64;
        }
        offset += n as u64;
        if !on_bytes(offset) {
            return Err("Cancelled".to_string());
        }
    }
    d.set_len(src_len).map_err(|e| e.to_string())?;
    d.flush().map_err(|e| e.to_string())?;
    drop(d);
    filetime::set_file_mtime(dst, filetime::FileTime::from_unix_time(mtime, 0)).map_err(|e| e.to_string())?;
    Ok(written)
}

fn read_full(f: &mut File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut total = 0;
    while total < buf.len() {
        let n = f.read(&mut buf[total..])?;
        if n == 0 {
            break;
        }
        total += n;
    }
    Ok(total)
}

/// CRC-32 of a stored file
pub fn checksum(store: &dyn Store, rel: &str) -> Result<u32, String> {
    let mut r = store.open_read(rel)?;
    let mut h = crc32fast::Hasher::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let n = r.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize())
}

/// Passes data through while computing its CRC-32
pub struct CrcReader<R: Read> {
    inner: R,
    hasher: crc32fast::Hasher,
    pub bytes: u64,
}

impl<R: Read> CrcReader<R> {
    pub fn new(inner: R) -> Self {
        Self { inner, hasher: crc32fast::Hasher::new(), bytes: 0 }
    }
    pub fn crc(&self) -> u32 {
        self.hasher.clone().finalize()
    }
}

impl<R: Read> Read for CrcReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.hasher.update(&buf[..n]);
        self.bytes += n as u64;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_rel_rejects_escapes() {
        assert_eq!(safe_rel("a/./b//c").unwrap(), "a/b/c");
        assert_eq!(safe_rel("/a/b").unwrap(), "a/b");
        assert!(safe_rel("a/../../etc").is_err());
        assert!(safe_rel("..").is_err());
    }

    #[test]
    fn write_part_then_rename() {
        let dir = tempfile::tempdir().unwrap();
        let store = LocalStore::new(dir.path());
        let mut data: &[u8] = b"hello";
        let part = store.write_part("x/y.txt", &mut data, 1_700_000_000, &mut |_| true).unwrap();
        assert!(part.ends_with(PART_SUFFIX));
        assert!(store.stat("x/y.txt").is_none());
        store.rename(&part, "x/y.txt").unwrap();
        let meta = store.stat("x/y.txt").unwrap();
        assert_eq!(meta, FileMeta { size: 5, mtime: 1_700_000_000 });
    }

    #[test]
    fn changed_blocks_only() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a"), dir.path().join("b"));
        let mut big = vec![7u8; 3 * 1024 * 1024 + 10];
        fs::write(&b, &big).unwrap();
        big[2 * 1024 * 1024 + 5] = 9;
        big.truncate(3 * 1024 * 1024);
        fs::write(&a, &big).unwrap();
        let written = update_changed_blocks(&a, &b, 1_700_000_000, &AtomicBool::new(false), &mut |_| true).unwrap();
        assert_eq!(written, 1024 * 1024);
        assert_eq!(fs::read(&b).unwrap(), big);
    }
}

/// Several folders shown as one, each under its own name, for backups with more than one source.
/// `Documents/report.pdf` is `report.pdf` in the folder named Documents.
pub struct MultiStore {
    parts: Vec<(String, std::sync::Arc<dyn Store>)>,
}

impl MultiStore {
    pub fn new(parts: Vec<(String, std::sync::Arc<dyn Store>)>) -> Self {
        Self { parts }
    }

    /// The store and the path inside it for a path in the combined view
    fn route<'a>(&'a self, rel: &str) -> Result<(&'a dyn Store, String), String> {
        let rel = rel.trim_matches('/');
        let (top, rest) = rel.split_once('/').unwrap_or((rel, ""));
        self.parts
            .iter()
            .find(|(name, _)| name == top)
            .map(|(_, s)| (s.as_ref(), rest.to_string()))
            .ok_or_else(|| format!("{rel} is not in any of the backed-up folders"))
    }
}

/// Folder names for several sources: each folder's own name, with " (2)", " (3)" for repeats
pub fn source_names(paths: &[String]) -> Vec<String> {
    let mut used: Vec<String> = Vec::new();
    paths
        .iter()
        .map(|p| {
            let base = p.trim_end_matches('/').rsplit('/').next().filter(|b| !b.is_empty() && !b.contains("://")).unwrap_or("root");
            let base = base.trim_start_matches('.');
            let base = if base.is_empty() { "folder" } else { base };
            let mut name = base.to_string();
            let mut n = 2;
            while used.iter().any(|u| u.eq_ignore_ascii_case(&name)) {
                name = format!("{base} ({n})");
                n += 1;
            }
            used.push(name.clone());
            name
        })
        .collect()
}

impl Store for MultiStore {
    fn label(&self) -> String {
        self.parts.iter().map(|(_, s)| s.label()).collect::<Vec<_>>().join(", ")
    }

    fn root_exists(&self) -> bool {
        self.parts.iter().all(|(_, s)| s.root_exists())
    }

    fn ensure_root(&self) -> Result<(), String> {
        self.parts.iter().try_for_each(|(_, s)| s.ensure_root())
    }

    fn list(&self, base: &str, skip: &dyn Fn(&str, bool) -> bool, cancel: &AtomicBool, warnings: &mut Vec<String>) -> Result<Listing, String> {
        if !base.trim_matches('/').is_empty() {
            let (store, inner) = self.route(base)?;
            return store.list(&inner, skip, cancel, warnings);
        }
        let mut out = Listing::new();
        for (name, store) in &self.parts {
            if skip(name, true) {
                continue;
            }
            let prefixed_skip = |rel: &str, is_dir: bool| skip(&join_rel(name, rel), is_dir);
            for (rel, meta) in store.list("", &prefixed_skip, cancel, warnings)? {
                out.insert(join_rel(name, &rel), meta);
            }
        }
        Ok(out)
    }

    fn list_dir(&self, rel: &str) -> Result<Vec<DirItem>, String> {
        if rel.trim_matches('/').is_empty() {
            return Ok(self.parts.iter().map(|(name, _)| DirItem { name: name.clone(), is_dir: true, size: 0, mtime: 0 }).collect());
        }
        let (store, inner) = self.route(rel)?;
        store.list_dir(&inner)
    }

    fn stat(&self, rel: &str) -> Option<FileMeta> {
        let (store, inner) = self.route(rel).ok()?;
        if inner.is_empty() {
            return None;
        }
        store.stat(&inner)
    }

    fn is_dir(&self, rel: &str) -> bool {
        if rel.trim_matches('/').is_empty() {
            return true;
        }
        self.route(rel).map(|(store, inner)| inner.is_empty() || store.is_dir(&inner)).unwrap_or(false)
    }

    fn open_read(&self, rel: &str) -> Result<Box<dyn Read + Send>, String> {
        let (store, inner) = self.route(rel)?;
        store.open_read(&inner)
    }

    fn write_part(&self, rel: &str, data: &mut dyn Read, mtime: i64, on_bytes: &mut dyn FnMut(u64) -> bool) -> Result<String, String> {
        let (store, inner) = self.route(rel)?;
        let top = rel.trim_matches('/').split('/').next().unwrap_or_default().to_string();
        store.write_part(&inner, data, mtime, on_bytes).map(|p| join_rel(&top, &p))
    }

    fn rename(&self, from: &str, to: &str) -> Result<(), String> {
        let (a, from_inner) = self.route(from)?;
        let (b, to_inner) = self.route(to)?;
        if !std::ptr::eq(a as *const dyn Store as *const u8, b as *const dyn Store as *const u8) {
            return Err("Cannot move a file between two backed-up folders".into());
        }
        a.rename(&from_inner, &to_inner)
    }

    fn remove_file(&self, rel: &str) -> Result<(), String> {
        let (store, inner) = self.route(rel)?;
        store.remove_file(&inner)
    }

    fn remove_dir_all(&self, rel: &str) -> Result<(), String> {
        let (store, inner) = self.route(rel)?;
        if inner.is_empty() {
            return Err("A backed-up folder itself is never removed".into());
        }
        store.remove_dir_all(&inner)
    }

    fn remove_empty_parents(&self, rel: &str) {
        if let Ok((store, inner)) = self.route(rel) {
            store.remove_empty_parents(&inner);
        }
    }

    fn local_path(&self, rel: &str) -> Option<PathBuf> {
        let (store, inner) = self.route(rel).ok()?;
        store.local_path(&inner)
    }

    fn rclone_location(&self, rel: &str) -> Option<RcloneLocation> {
        let (store, inner) = self.route(rel).ok()?;
        store.rclone_location(&inner)
    }

    fn rclone_copy(&self, src: &RcloneLocation, dst: &RcloneLocation) -> Option<Result<(), String>> {
        self.parts.iter().find_map(|(_, s)| s.rclone_copy(src, dst))
    }

    fn part_names(&self) -> Vec<String> {
        self.parts.iter().map(|(n, _)| n.clone()).collect()
    }
}

/// Where a file or folder appears in a git collection: under your home `home/...`, otherwise
/// `root/...` with its full path. `/etc/hosts` is `root/etc/hosts`, `~/.bashrc` is `home/.bashrc`.
pub fn mirror_name(path: &str, home: Option<&str>) -> String {
    let p = path.trim_end_matches('/');
    if let Some(h) = home.map(|h| h.trim_end_matches('/')).filter(|h| !h.is_empty() && *h != "/") {
        if p == h {
            return "home".to_string();
        }
        if let Some(rest) = p.strip_prefix(&format!("{h}/")) {
            return join_rel("home", rest);
        }
    }
    join_rel("root", p.trim_start_matches('/'))
}

/// The real path on this machine for a path in a git collection, the reverse of `mirror_name`:
/// `home/...` lands in the current home (whatever the username was), `root/...` at the same full
/// path. `None` for anything else, or a path that would climb out with `..`.
pub fn unmirror(name: &str, home: Option<&str>) -> Option<String> {
    let name = name.trim_matches('/');
    if name.split('/').any(|part| part.is_empty() || part == "." || part == ".." || part.contains('\\')) {
        return None;
    }
    if name == "home" || name.starts_with("home/") {
        let h = home.map(|h| h.trim_end_matches('/')).filter(|h| !h.is_empty())?;
        return Some(match name.strip_prefix("home/") {
            Some(rest) => format!("{h}/{rest}"),
            None => h.to_string(),
        });
    }
    name.strip_prefix("root/").map(|rest| format!("/{rest}"))
}

/// Tracked files and folders shown at their mirrored paths (see `mirror_name`), for git collections
pub struct MirrorStore {
    /// (mirrored path, real path, is a single file)
    items: Vec<(String, PathBuf, bool)>,
}

impl MirrorStore {
    pub fn new(paths: &[String], home: Option<&str>) -> Self {
        let items = paths
            .iter()
            .map(|p| {
                let real = PathBuf::from(p);
                let is_file = real.is_file();
                (mirror_name(p, home), real, is_file)
            })
            .collect();
        Self { items }
    }

    /// The real path for a path in the mirrored view
    fn real(&self, rel: &str) -> Result<PathBuf, String> {
        let rel = rel.trim_matches('/');
        for (mirror, real, is_file) in &self.items {
            if rel == mirror {
                return Ok(real.clone());
            }
            if !is_file {
                if let Some(rest) = rel.strip_prefix(&format!("{mirror}/")) {
                    let mut p = real.clone();
                    for part in rest.split('/').filter(|x| !x.is_empty()) {
                        p.push(part);
                    }
                    return Ok(p);
                }
            }
            // A temporary copy beside a tracked single file (or one that does not exist yet, as on
            // a new machine before restoring, when it cannot be told from a folder)
            if *is_file || !real.exists() {
                if let (Some((dir, name)), Some(parent)) = (rel.rsplit_once('/'), real.parent()) {
                    let (mdir, _) = mirror.rsplit_once('/').unwrap_or(("", mirror.as_str()));
                    if dir == mdir && name.ends_with(PART_SUFFIX) {
                        return Ok(parent.join(name));
                    }
                }
            }
        }
        Err(format!("{rel} is not one of the tracked files or folders"))
    }

    fn with_parent<T>(&self, rel: &str, f: impl FnOnce(&LocalStore, &str) -> T) -> Result<T, String> {
        let real = self.real(rel)?;
        let parent = real.parent().ok_or("Invalid path")?.to_path_buf();
        let name = real.file_name().and_then(|n| n.to_str()).ok_or("Invalid file name")?.to_string();
        Ok(f(&LocalStore::new(parent), &name))
    }
}

impl Store for MirrorStore {
    fn label(&self) -> String {
        self.items.iter().map(|(_, r, _)| r.display().to_string()).collect::<Vec<_>>().join(", ")
    }

    fn root_exists(&self) -> bool {
        self.items.iter().all(|(_, r, _)| r.exists())
    }

    fn ensure_root(&self) -> Result<(), String> {
        Ok(())
    }

    fn list(&self, base: &str, skip: &dyn Fn(&str, bool) -> bool, cancel: &AtomicBool, warnings: &mut Vec<String>) -> Result<Listing, String> {
        let base = base.trim_matches('/');
        let mut out = Listing::new();
        for (mirror, real, is_file) in &self.items {
            // Only the items inside `base`, keyed relative to it
            let under = if base.is_empty() { Some(mirror.clone()) } else if mirror == base { Some(String::new()) } else { mirror.strip_prefix(&format!("{base}/")).map(String::from) };
            let Some(prefix) = under else {
                if mirror.len() < base.len() && base.starts_with(&format!("{mirror}/")) && !is_file {
                    let inner = &base[mirror.len() + 1..];
                    return LocalStore::new(real).list(inner, skip, cancel, warnings);
                }
                continue;
            };
            if skip(mirror, !is_file) {
                continue;
            }
            if *is_file {
                if let Some(m) = LocalStore::new(real.parent().unwrap_or(Path::new("/"))).stat(real.file_name().and_then(|n| n.to_str()).unwrap_or_default()) {
                    let key = if prefix.is_empty() { real.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string() } else { prefix };
                    out.insert(key, m);
                }
            } else {
                let prefixed_skip = |rel: &str, d: bool| skip(&join_rel(mirror, rel), d);
                for (rel, m) in LocalStore::new(real).list("", &prefixed_skip, cancel, warnings)? {
                    out.insert(join_rel(&prefix, &rel), m);
                }
            }
        }
        Ok(out)
    }

    fn list_dir(&self, rel: &str) -> Result<Vec<DirItem>, String> {
        let rel = rel.trim_matches('/');
        // Folders on the way to the tracked items ("home", "root/etc") and the items themselves
        let mut items: Vec<DirItem> = Vec::new();
        for (mirror, real, is_file) in &self.items {
            if !rel.is_empty() && (mirror == rel || rel.starts_with(&format!("{mirror}/"))) && !is_file {
                let inner = if mirror == rel { "" } else { &rel[mirror.len() + 1..] };
                return LocalStore::new(real).list_dir(inner);
            }
            let rest = if rel.is_empty() { Some(mirror.as_str()) } else { mirror.strip_prefix(&format!("{rel}/")) };
            if let Some(rest) = rest {
                let (first, more) = rest.split_once('/').map(|(a, _)| (a, true)).unwrap_or((rest, false));
                if !items.iter().any(|i| i.name == first) {
                    let is_dir = more || !is_file;
                    let meta = if is_dir { None } else { std::fs::metadata(real).ok() };
                    items.push(DirItem { name: first.to_string(), is_dir, size: meta.as_ref().map(|m| m.len()).unwrap_or(0), mtime: meta.as_ref().map(mtime_secs).unwrap_or(0) });
                }
            }
        }
        items.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        Ok(items)
    }

    fn stat(&self, rel: &str) -> Option<FileMeta> {
        self.with_parent(rel, |s, name| s.stat(name)).ok().flatten()
    }

    fn is_dir(&self, rel: &str) -> bool {
        let rel = rel.trim_matches('/');
        if rel.is_empty() || self.items.iter().any(|(m, _, _)| m.starts_with(&format!("{rel}/"))) {
            return true;
        }
        self.real(rel).map(|p| p.is_dir()).unwrap_or(false)
    }

    fn open_read(&self, rel: &str) -> Result<Box<dyn Read + Send>, String> {
        File::open(self.real(rel)?).map(|f| Box::new(f) as Box<dyn Read + Send>).map_err(|e| e.to_string())
    }

    fn write_part(&self, rel: &str, data: &mut dyn Read, mtime: i64, on_bytes: &mut dyn FnMut(u64) -> bool) -> Result<String, String> {
        let part_in_parent = self.with_parent(rel, |s, name| s.write_part(name, data, mtime, on_bytes))??;
        let dir = rel.trim_matches('/').rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        Ok(join_rel(dir, &part_in_parent))
    }

    fn rename(&self, from: &str, to: &str) -> Result<(), String> {
        let (a, b) = (self.real(from)?, self.real(to)?);
        if let Some(parent) = b.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fs::rename(&a, &b).map_err(|e| format!("Cannot move {from}: {e}"))
    }

    fn remove_file(&self, rel: &str) -> Result<(), String> {
        fs::remove_file(self.real(rel)?).map_err(|e| e.to_string())
    }

    fn remove_dir_all(&self, _rel: &str) -> Result<(), String> {
        Err("Tracked folders are never removed".into())
    }

    fn remove_empty_parents(&self, _rel: &str) {}

    fn local_path(&self, rel: &str) -> Option<PathBuf> {
        self.real(rel).ok()
    }

    #[cfg(unix)]
    fn rclone_location(&self, rel: &str) -> Option<RcloneLocation> {
        let p = self.real(rel).ok()?;
        Some(("/".to_string(), p.to_str()?.trim_start_matches('/').to_string()))
    }

    fn part_names(&self) -> Vec<String> {
        self.items.iter().map(|(m, _, _)| m.clone()).collect()
    }
}
