//! Copies and moves between Fleet machines (#190), done by Brum servers instead of the browser.
//!
//! A job copies items from a folder on one machine into a folder on another. Each file goes to a
//! temporary `.brum-part` file first, is checked (SHA-256 worked out on each machine, so files do
//! not travel twice), and only then replaces the target. A move removes the original only after
//! its copy is checked. An interrupted copy keeps its part file, so Retry continues where it
//! stopped. Jobs run in the task list, so they keep going after the browser is closed, and can be
//! paused, stopped and retried there.

use crate::tools::sync::fleet_store::FleetStore;
use crate::tools::sync::store::{part_name, FileMeta, Store};
use crate::tools::tasks::TaskManager;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// SHA-256 and size of a file on this machine
pub fn sha256_file(path: &Path) -> Result<(String, u64), String> {
    let mut f = File::open(path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    let mut size = 0u64;
    loop {
        let n = f.read(&mut buf).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        size += n as u64;
    }
    Ok((format!("{:x}", h.finalize()), size))
}

/// A folder on this machine or on another Fleet machine. Paths inside it use `/`.
pub enum Place {
    Local(PathBuf),
    Remote(FleetStore),
}

/// What one source item holds
enum ItemKind {
    File(FileMeta),
    /// Files (relative to the item) with size and time, and every folder below it
    Dir(Vec<(String, FileMeta)>, Vec<String>),
}

impl Place {
    fn local(&self, rel: &str) -> Option<PathBuf> {
        match self {
            Place::Local(root) => {
                let mut p = root.clone();
                for part in rel.split('/').filter(|p| !p.is_empty()) {
                    p.push(part);
                }
                Some(p)
            }
            Place::Remote(_) => None,
        }
    }

    fn item(&self, name: &str) -> Result<ItemKind, String> {
        match self {
            Place::Local(_) => {
                let path = self.local(name).unwrap();
                let meta = fs::symlink_metadata(&path).map_err(|e| format!("Cannot read {name}: {e}"))?;
                if meta.is_file() {
                    return Ok(ItemKind::File(FileMeta { size: meta.len(), mtime: mtime_of(&meta) }));
                }
                if !meta.is_dir() {
                    return Err(format!("{name} is a link or special file and is not copied"));
                }
                let (mut files, mut dirs) = (Vec::new(), Vec::new());
                for e in walkdir::WalkDir::new(&path).follow_links(false).into_iter() {
                    let e = e.map_err(|e| format!("Cannot read inside {name}: {e}"))?;
                    if e.depth() == 0 {
                        continue;
                    }
                    let rel: Vec<String> = match e.path().strip_prefix(&path) {
                        Ok(r) => r.components().filter_map(|c| c.as_os_str().to_str().map(String::from)).collect(),
                        Err(_) => continue,
                    };
                    let rel = rel.join("/");
                    if e.file_type().is_dir() {
                        dirs.push(rel);
                    } else if e.file_type().is_file() {
                        if let Ok(m) = e.metadata() {
                            files.push((rel, FileMeta { size: m.len(), mtime: mtime_of(&m) }));
                        }
                    }
                }
                Ok(ItemKind::Dir(files, dirs))
            }
            Place::Remote(s) => {
                if let Some(m) = s.stat(name) {
                    return Ok(ItemKind::File(m));
                }
                if !s.is_dir(name) {
                    return Err(format!("{name} was not found"));
                }
                // Names from another machine must stay inside the item, whatever it sends
                let (files, dirs) = s.tree(name)?;
                let files = files.into_iter().filter_map(|(rel, m)| crate::tools::sync::store::safe_rel(&rel).ok().filter(|r| !r.is_empty()).map(|r| (r, m))).collect();
                let dirs = dirs.into_iter().filter_map(|rel| crate::tools::sync::store::safe_rel(&rel).ok().filter(|r| !r.is_empty())).collect();
                Ok(ItemKind::Dir(files, dirs))
            }
        }
    }

    fn exists(&self, rel: &str) -> bool {
        match self {
            Place::Local(_) => self.local(rel).is_some_and(|p| p.exists()),
            Place::Remote(s) => s.stat(rel).is_some() || s.is_dir(rel),
        }
    }

    fn make_dir(&self, rel: &str) -> Result<(), String> {
        match self {
            Place::Local(_) => fs::create_dir_all(self.local(rel).unwrap()).map_err(|e| format!("Cannot create folder {rel}: {e}")),
            Place::Remote(s) => s.make_dir(rel),
        }
    }

    fn file_size(&self, rel: &str) -> Option<u64> {
        match self {
            Place::Local(_) => fs::metadata(self.local(rel).unwrap()).ok().filter(|m| m.is_file()).map(|m| m.len()),
            Place::Remote(s) => s.file_size(rel),
        }
    }

    fn read_from(&self, rel: &str, offset: u64) -> Result<Box<dyn Read + Send>, String> {
        match self {
            Place::Local(_) => {
                let mut f = File::open(self.local(rel).unwrap()).map_err(|e| format!("Cannot read {rel}: {e}"))?;
                if offset > 0 {
                    f.seek(SeekFrom::Start(offset)).map_err(|e| e.to_string())?;
                }
                Ok(Box::new(f))
            }
            Place::Remote(s) => s.read_from(rel, offset),
        }
    }

    /// Reads a file for showing it (at most `limit` bytes are wanted)
    pub fn read_limited(&self, rel: &str, limit: u64) -> Result<Box<dyn Read + Send>, String> {
        Ok(Box::new(self.read_from(rel, 0)?.take(limit)))
    }

    fn sha256(&self, rel: &str) -> Result<String, String> {
        match self {
            Place::Local(_) => sha256_file(&self.local(rel).unwrap()).map(|(h, _)| h),
            Place::Remote(s) => s.sha256(rel),
        }
    }

    /// Writes into the part file from `offset` on; see `FleetStore::write_part_at`
    fn write_part_at(&self, part: &str, data: &mut dyn Read, mtime: i64, offset: u64, hash: bool, on_bytes: &mut dyn FnMut(u64) -> bool) -> Result<Option<String>, String> {
        match self {
            Place::Local(_) => {
                let path = self.local(part).unwrap();
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent).map_err(|e| format!("Cannot create folder: {e}"))?;
                }
                let mut hasher = hash.then(Sha256::new);
                let mut out = if offset > 0 {
                    if let Some(h) = hasher.as_mut() {
                        let mut f = File::open(&path).map_err(|e| e.to_string())?;
                        std::io::copy(&mut f, h).map_err(|e| e.to_string())?;
                    }
                    let f = OpenOptions::new().append(true).open(&path).map_err(|e| e.to_string())?;
                    if f.metadata().map(|m| m.len()).unwrap_or(0) != offset {
                        return Err("The unfinished copy changed; starting again".into());
                    }
                    f
                } else {
                    File::create(&path).map_err(|e| e.to_string())?
                };
                let mut buf = vec![0u8; 1024 * 1024];
                let mut written = offset;
                loop {
                    let n = data.read(&mut buf).map_err(|e| format!("Read failed: {e}"))?;
                    if n == 0 {
                        break;
                    }
                    out.write_all(&buf[..n]).map_err(|e| format!("Write failed: {e}"))?;
                    if let Some(h) = hasher.as_mut() {
                        h.update(&buf[..n]);
                    }
                    written += n as u64;
                    if !on_bytes(written) {
                        return Err("Cancelled".into());
                    }
                }
                out.flush().map_err(|e| e.to_string())?;
                drop(out);
                filetime::set_file_mtime(&path, filetime::FileTime::from_unix_time(mtime, 0)).map_err(|e| e.to_string())?;
                Ok(hasher.map(|h| format!("{:x}", h.finalize())))
            }
            Place::Remote(s) => s.write_part_at(part, data, mtime, offset, hash, on_bytes),
        }
    }

    fn rename(&self, from: &str, to: &str) -> Result<(), String> {
        match self {
            Place::Local(_) => {
                let dst = self.local(to).unwrap();
                if let Some(parent) = dst.parent() {
                    fs::create_dir_all(parent).map_err(|e| format!("Cannot create folder: {e}"))?;
                }
                fs::rename(self.local(from).unwrap(), &dst).map_err(|e| format!("Cannot put {to} in place: {e}"))
            }
            Place::Remote(s) => s.rename(from, to),
        }
    }

    fn remove_file(&self, rel: &str) -> Result<(), String> {
        match self {
            Place::Local(_) => fs::remove_file(self.local(rel).unwrap()).map_err(|e| format!("Cannot remove {rel}: {e}")),
            Place::Remote(s) => s.remove_file(rel),
        }
    }

    /// Removes the folder `rel` if it is empty
    fn remove_dir_if_empty(&self, rel: &str) {
        match self {
            Place::Local(_) => {
                let _ = fs::remove_dir(self.local(rel).unwrap());
            }
            // Removes the empty folders above a name inside `rel`, which starts with `rel` itself
            Place::Remote(s) => s.remove_empty_parents(&format!("{rel}/.brum-empty-check")),
        }
    }
}

fn mtime_of(meta: &fs::Metadata) -> i64 {
    meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// One item chosen in the source pane and how to treat a same-named item at the target
#[derive(Clone, Debug, serde::Deserialize)]
pub struct ItemSpec {
    pub name: String,
    /// "overwrite" (default) or "rename" (keep both: the copy gets a new name)
    #[serde(default)]
    pub conflict: Option<String>,
}

/// One file to copy
#[derive(Clone, Debug)]
struct FileWork {
    src: String,
    dst: String,
    size: u64,
    mtime: i64,
}

pub struct Job {
    pub id: String,
    pub is_move: bool,
    pub verify: bool,
    src: Place,
    dst: Place,
    items: Vec<ItemSpec>,
    /// Files that did not make it in the last run, for Retry
    failed: Mutex<Vec<FileWork>>,
    /// Source folders a move empties, removed at the end (deepest first)
    move_dirs: Mutex<Vec<String>>,
    running: Mutex<bool>,
    max_jobs: usize,
}

static JOBS: OnceLock<Mutex<HashMap<String, Arc<Job>>>> = OnceLock::new();

fn jobs() -> &'static Mutex<HashMap<String, Arc<Job>>> {
    JOBS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The job behind a task, while Brum runs
pub fn job(id: &str) -> Option<Arc<Job>> {
    jobs().lock().ok()?.get(id).cloned()
}

/// How many jobs copy at the same time; the others wait their turn
static SLOTS: OnceLock<(Mutex<usize>, Condvar)> = OnceLock::new();

fn slots() -> &'static (Mutex<usize>, Condvar) {
    SLOTS.get_or_init(|| (Mutex::new(0), Condvar::new()))
}

struct Slot;

impl Drop for Slot {
    fn drop(&mut self) {
        let (lock, cv) = slots();
        if let Ok(mut n) = lock.lock() {
            *n = n.saturating_sub(1);
        }
        cv.notify_all();
    }
}

/// Waits for a free slot; None when the task was stopped meanwhile
fn take_slot(tasks: &TaskManager, id: &str, max: usize) -> Option<Slot> {
    let (lock, cv) = slots();
    let mut n = lock.lock().ok()?;
    let mut said = false;
    while *n >= max.max(1) {
        if tasks.sync_is_cancelled(id) {
            return None;
        }
        if !said {
            tasks.sync_edit(id, |t| t.current_file = Some("Waiting for another copy to finish".into()));
            said = true;
        }
        n = cv.wait_timeout(n, Duration::from_millis(500)).ok()?.0;
    }
    *n += 1;
    Some(Slot)
}

impl Job {
    pub fn new(id: String, is_move: bool, verify: bool, src: Place, dst: Place, items: Vec<ItemSpec>, max_jobs: usize) -> Arc<Job> {
        let job = Arc::new(Job { id: id.clone(), is_move, verify, src, dst, items, failed: Mutex::new(Vec::new()), move_dirs: Mutex::new(Vec::new()), running: Mutex::new(false), max_jobs });
        if let Ok(mut map) = jobs().lock() {
            map.insert(id, job.clone());
        }
        job
    }

    /// Whether a Retry has something to do
    pub fn has_failed(&self) -> bool {
        self.failed.lock().map(|f| !f.is_empty()).unwrap_or(false)
    }

    /// Lists what to copy: every file, and the folders to create at the target
    fn plan(&self, tasks: &TaskManager) -> Result<(Vec<FileWork>, Vec<String>), String> {
        let (mut work, mut dirs, mut move_dirs) = (Vec::new(), Vec::new(), Vec::new());
        for item in &self.items {
            let name = item.name.as_str();
            let kind = match self.src.item(name) {
                Ok(k) => k,
                Err(e) => {
                    tasks.sync_log(&self.id, &format!("✗ {e}"));
                    continue;
                }
            };
            // Keep both: the copy gets a free name like "photo (1).jpg"
            let target = if item.conflict.as_deref() == Some("rename") && self.dst.exists(name) { self.free_name(name, matches!(kind, ItemKind::Dir(..))) } else { name.to_string() };
            if target != name {
                tasks.sync_log(&self.id, &format!("{name} already exists there; the copy is called {target}"));
            }
            match kind {
                ItemKind::File(m) => work.push(FileWork { src: name.to_string(), dst: target, size: m.size, mtime: m.mtime }),
                ItemKind::Dir(files, subdirs) => {
                    dirs.push(target.clone());
                    move_dirs.push(name.to_string());
                    for d in subdirs {
                        dirs.push(format!("{target}/{d}"));
                        move_dirs.push(format!("{name}/{d}"));
                    }
                    for (rel, m) in files {
                        work.push(FileWork { src: format!("{name}/{rel}"), dst: format!("{target}/{rel}"), size: m.size, mtime: m.mtime });
                    }
                }
            }
        }
        if self.is_move {
            // Deepest first, so each folder is empty by the time it is removed
            move_dirs.sort_by_key(|d| std::cmp::Reverse(d.matches('/').count()));
            if let Ok(mut m) = self.move_dirs.lock() {
                *m = move_dirs;
            }
        }
        Ok((work, dirs))
    }

    fn free_name(&self, name: &str, is_dir: bool) -> String {
        let (stem, ext) = match name.rsplit_once('.') {
            Some((s, e)) if !is_dir && !s.is_empty() => (s.to_string(), format!(".{e}")),
            _ => (name.to_string(), String::new()),
        };
        (1..10_000).map(|i| format!("{stem} ({i}){ext}")).find(|n| !self.dst.exists(n)).unwrap_or_else(|| format!("{stem} (copy){ext}"))
    }

    /// Runs the job (or, after a Retry, the files that failed). Call on a thread of its own.
    pub fn run(self: &Arc<Self>, tasks: &TaskManager, retry: bool) {
        {
            let Ok(mut r) = self.running.lock() else { return };
            if *r {
                return;
            }
            *r = true;
        }
        let _slot = take_slot(tasks, &self.id, self.max_jobs);
        if _slot.is_some() {
            self.run_inner(tasks, retry);
        }
        if let Ok(mut r) = self.running.lock() {
            *r = false;
        }
    }

    fn run_inner(self: &Arc<Self>, tasks: &TaskManager, retry: bool) {
        let id = self.id.as_str();
        let work = if retry {
            let w = self.failed.lock().map(|mut f| std::mem::take(&mut *f)).unwrap_or_default();
            tasks.sync_log(id, &format!("Retrying {} file(s); unfinished copies continue where they stopped", w.len()));
            w
        } else {
            tasks.sync_edit(id, |t| t.current_file = Some("Listing files…".into()));
            let (work, dirs) = match self.plan(tasks) {
                Ok(p) => p,
                Err(e) => {
                    tasks.sync_fail_task(id, &e);
                    return;
                }
            };
            for d in &dirs {
                if let Err(e) = self.dst.make_dir(d) {
                    tasks.sync_log(id, &format!("✗ {e}"));
                }
            }
            work
        };
        let total_bytes: u64 = work.iter().map(|w| w.size).sum();
        let total_files = work.len() as u64;
        tasks.sync_edit(id, |t| {
            t.total_files = total_files;
            t.total_bytes = total_bytes;
            t.files_processed = 0;
            t.bytes_processed = 0;
            t.paranoid = self.verify;
        });

        let started = Instant::now();
        let mut done_bytes = 0u64;
        let mut moved_bytes = 0u64; // bytes actually sent this run, for the speed
        let mut failed = Vec::new();
        let mut cancelled = false;
        let mut verified = 0u64;
        for (i, w) in work.iter().enumerate() {
            if tasks.sync_is_cancelled(id) {
                cancelled = true;
                failed.extend(work[i..].iter().cloned());
                break;
            }
            tasks.sync_edit(id, |t| {
                t.current_file = Some(w.dst.clone());
                t.current_file_bytes = 0;
                t.current_file_total_bytes = w.size;
                t.files_processed = i as u64;
                t.bytes_processed = done_bytes;
            });
            match self.copy_file(tasks, w, done_bytes, &mut moved_bytes, started) {
                Ok(hash) => {
                    if let Some(h) = hash {
                        verified += 1;
                        tasks.sync_edit(id, |t| {
                            t.verified_files = verified;
                            t.last_hash = Some(format!("SHA-256 {h}"));
                        });
                    }
                    tasks.sync_log(id, &format!("✓ {}", w.dst));
                }
                Err(CopyError::Cancelled) => {
                    cancelled = true;
                    failed.extend(work[i..].iter().cloned());
                    break;
                }
                Err(CopyError::Failed(e)) => {
                    tasks.sync_log(id, &format!("✗ {}: {e}", w.dst));
                    failed.push(w.clone());
                }
            }
            done_bytes += w.size;
        }

        // A move: remove the source folders it emptied, when everything arrived
        if self.is_move && !cancelled && failed.is_empty() {
            if let Ok(dirs) = self.move_dirs.lock() {
                for d in dirs.iter() {
                    self.src.remove_dir_if_empty(d);
                }
            }
        }

        let n_failed = failed.len();
        if let Ok(mut f) = self.failed.lock() {
            *f = failed;
        }
        if cancelled {
            return; // the task list already says it was stopped
        }
        tasks.sync_edit(id, |t| {
            t.current_file = None;
            t.speed_bytes_per_sec = 0;
        });
        if n_failed == 0 {
            tasks.sync_complete_task(id);
        } else {
            tasks.sync_fail_task(id, &format!("{n_failed} of {total_files} file(s) could not be copied. Retry continues where they stopped."));
        }
    }

    /// Copies one file through its part file; returns its checksum when checked
    fn copy_file(self: &Arc<Self>, tasks: &TaskManager, w: &FileWork, done_before: u64, moved: &mut u64, started: Instant) -> Result<Option<String>, CopyError> {
        let id = self.id.as_str();
        let part = part_name(&w.dst);
        // Continue an unfinished copy only when the result is checked anyway
        let mut offset = if self.verify { self.dst.file_size(&part).filter(|&n| n > 0 && n <= w.size).unwrap_or(0) } else { 0 };
        if offset > 0 {
            tasks.sync_log(id, &format!("Continuing {} from {}", w.dst, human(offset)));
        }
        for attempt in 0..2 {
            // The source's checksum is worked out on its own machine while the file travels
            // On a thread of its own, so Stop does not wait for a large file's checksum
            let hasher = self.verify.then(|| {
                let (tx, rx) = std::sync::mpsc::channel();
                let (job, rel) = (Arc::clone(self), w.src.clone());
                std::thread::spawn(move || {
                    let _ = tx.send(job.src.sha256(&rel));
                });
                rx
            });
            let (src_hash, dst_hash) = (|| -> Result<(Option<String>, Option<String>), CopyError> {
                let mut reader = self.src.read_from(&w.src, offset).map_err(CopyError::Failed)?;
                let mut last = Instant::now();
                let mut on_bytes = |written: u64| -> bool {
                    // Pause holds the copy; Stop ends it
                    while tasks.sync_is_paused(id) {
                        if tasks.sync_is_cancelled(id) {
                            return false;
                        }
                        std::thread::sleep(Duration::from_millis(250));
                    }
                    if tasks.sync_is_cancelled(id) {
                        return false;
                    }
                    if last.elapsed() >= Duration::from_millis(200) {
                        last = Instant::now();
                        let sent = *moved + written.saturating_sub(offset);
                        let secs = started.elapsed().as_secs_f64().max(0.05);
                        tasks.sync_edit(id, |t| {
                            t.current_file_bytes = written;
                            t.bytes_processed = done_before + written;
                            t.speed_bytes_per_sec = (sent as f64 / secs) as u64;
                        });
                    }
                    true
                };
                let wrote = self.dst.write_part_at(&part, &mut reader, w.mtime, offset, self.verify, &mut on_bytes);
                let dst_hash = match wrote {
                    Err(e) if e == "Cancelled" || tasks.sync_is_cancelled(id) => {
                        let _ = self.dst.remove_file(&part);
                        return Err(CopyError::Cancelled);
                    }
                    Err(e) => {
                        if !self.verify {
                            let _ = self.dst.remove_file(&part);
                        }
                        return Err(CopyError::Failed(e));
                    }
                    Ok(h) => h,
                };
                let src = match &hasher {
                    Some(rx) => Some(rx.recv().map_err(|_| CopyError::Failed("Checking the source failed".into()))?.map_err(CopyError::Failed)?),
                    None => None,
                };
                Ok((src, dst_hash))
            })()?;
            let have = self.dst.file_size(&part).unwrap_or(0);
            *moved += have.saturating_sub(offset);
            let mut bad = have != w.size;
            if !bad {
                if let Some(expected) = &src_hash {
                    // Worked out while writing; read the part again only if that was not possible
                    let got = match dst_hash {
                        Some(h) => h,
                        None => self.dst.sha256(&part).map_err(CopyError::Failed)?,
                    };
                    bad = &got != expected;
                }
            }
            if bad {
                let _ = self.dst.remove_file(&part);
                if offset > 0 && attempt == 0 {
                    // The unfinished copy did not belong to this file any more: start over once
                    tasks.sync_log(id, &format!("{} changed since the last try; copying it again from the start", w.dst));
                    offset = 0;
                    continue;
                }
                return Err(CopyError::Failed(if have != w.size { format!("Size differs after copying ({} of {})", human(have), human(w.size)) } else { "The copy did not match the original (checksum differs); nothing was replaced".into() }));
            }
            self.dst.rename(&part, &w.dst).map_err(CopyError::Failed)?;
            if self.is_move {
                self.src.remove_file(&w.src).map_err(|e| CopyError::Failed(format!("Copied and checked, but the original could not be removed: {e}")))?;
            }
            return Ok(src_hash);
        }
        Err(CopyError::Failed("Could not copy".into()))
    }
}

enum CopyError {
    Cancelled,
    Failed(String),
}

fn human(n: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", U[i])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn wait(tasks: &TaskManager, id: &str) -> crate::tools::tasks::TaskInfo {
        for _ in 0..200 {
            let t = tasks.sync_get(id).unwrap();
            if t.status != "running" {
                return t;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("job did not finish");
    }

    #[test]
    fn copies_folders_and_checks_files() {
        let (a, b) = (tempdir().unwrap(), tempdir().unwrap());
        fs::create_dir_all(a.path().join("dir/empty")).unwrap();
        fs::write(a.path().join("dir/x.txt"), b"hello").unwrap();
        fs::write(a.path().join("one.bin"), vec![7u8; 3_000_000]).unwrap();
        fs::write(b.path().join("one.bin"), b"old").unwrap();
        let tasks = TaskManager::new();
        let id = tasks.sync_create_task("t", "copy", "a", "b", 0);
        let items = vec![ItemSpec { name: "dir".into(), conflict: None }, ItemSpec { name: "one.bin".into(), conflict: Some("rename".into()) }];
        let job = Job::new(id.clone(), false, true, Place::Local(a.path().into()), Place::Local(b.path().into()), items, 2);
        job.run(&tasks, false);
        let t = wait(&tasks, &id);
        assert_eq!(t.status, "completed", "{:?}", t.log_entries);
        assert_eq!(fs::read(b.path().join("dir/x.txt")).unwrap(), b"hello");
        assert!(b.path().join("dir/empty").is_dir());
        assert_eq!(fs::read(b.path().join("one.bin")).unwrap(), b"old", "keep both leaves the old file");
        assert_eq!(fs::read(b.path().join("one (1).bin")).unwrap().len(), 3_000_000);
        assert_eq!(t.verified_files, 2);
        assert!(a.path().join("dir/x.txt").exists(), "a copy keeps the originals");
    }

    #[test]
    fn move_removes_originals_after_checking() {
        let (a, b) = (tempdir().unwrap(), tempdir().unwrap());
        fs::create_dir_all(a.path().join("d/sub")).unwrap();
        fs::write(a.path().join("d/sub/f"), b"data").unwrap();
        let tasks = TaskManager::new();
        let id = tasks.sync_create_task("t", "move", "a", "b", 0);
        let job = Job::new(id.clone(), true, true, Place::Local(a.path().into()), Place::Local(b.path().into()), vec![ItemSpec { name: "d".into(), conflict: None }], 2);
        job.run(&tasks, false);
        assert_eq!(wait(&tasks, &id).status, "completed");
        assert_eq!(fs::read(b.path().join("d/sub/f")).unwrap(), b"data");
        assert!(!a.path().join("d").exists(), "the emptied source folders are gone");
    }

    #[test]
    fn continues_an_unfinished_copy() {
        let (a, b) = (tempdir().unwrap(), tempdir().unwrap());
        let data: Vec<u8> = (0..2_500_000u32).map(|i| (i % 251) as u8).collect();
        fs::write(a.path().join("big"), &data).unwrap();
        // Half of it arrived before the connection broke
        fs::write(b.path().join(part_name("big")), &data[..1_000_000]).unwrap();
        let tasks = TaskManager::new();
        let id = tasks.sync_create_task("t", "copy", "a", "b", 0);
        let job = Job::new(id.clone(), false, true, Place::Local(a.path().into()), Place::Local(b.path().into()), vec![ItemSpec { name: "big".into(), conflict: None }], 2);
        job.run(&tasks, false);
        let t = wait(&tasks, &id);
        assert_eq!(t.status, "completed", "{:?}", t.log_entries);
        assert!(t.log_entries.iter().any(|l| l.contains("Continuing big")), "{:?}", t.log_entries);
        assert_eq!(fs::read(b.path().join("big")).unwrap(), data);
        assert!(!b.path().join(part_name("big")).exists());
    }

    #[test]
    fn a_stale_part_file_is_replaced_not_trusted() {
        let (a, b) = (tempdir().unwrap(), tempdir().unwrap());
        fs::write(a.path().join("f"), vec![1u8; 4000]).unwrap();
        fs::write(b.path().join(part_name("f")), vec![9u8; 1000]).unwrap(); // from another version
        let tasks = TaskManager::new();
        let id = tasks.sync_create_task("t", "copy", "a", "b", 0);
        let job = Job::new(id.clone(), false, true, Place::Local(a.path().into()), Place::Local(b.path().into()), vec![ItemSpec { name: "f".into(), conflict: None }], 2);
        job.run(&tasks, false);
        assert_eq!(wait(&tasks, &id).status, "completed");
        assert_eq!(fs::read(b.path().join("f")).unwrap(), vec![1u8; 4000]);
    }
}
