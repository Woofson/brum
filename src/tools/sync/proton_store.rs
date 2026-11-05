//! Backups to Proton Drive (`proton:///my-files/Backups/Photos`), through Proton's official
//! program and the backup owner's own Proton login (see `crate::vfs::proton`).
//!
//! Every call to the program takes a second or more, so this store works in batches: folders
//! are read several at a time, changed files are uploaded one folder per call (several folders
//! at once), and removals go to Proton's trash in groups. Proton keeps replaced files in its own
//! version history, so no `_archive` folder is needed.

use super::store::{safe_rel, DirItem, FileMeta, Listing, Store};
use crate::vfs::proton;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// How many calls to Proton's program run at the same time
const PARALLEL: usize = 4;
/// Files per upload call, so the command line stays short
const FILES_PER_CALL: usize = 100;

pub struct ProtonStore {
    user: String,
    /// Program path of the backup folder, e.g. `/my-files/Backups/Photos`
    root: String,
    label: String,
    /// Folders known to exist (relative to the root), filled while listing and creating
    known_dirs: Mutex<BTreeSet<String>>,
}

/// What a batch did with one item: Ok(bytes) or the reason it failed
pub type BatchResult = Result<u64, String>;

impl ProtonStore {
    pub fn new(uri: &str, user: &str) -> Result<Self, String> {
        if user.trim().is_empty() {
            return Err("A Proton Drive backup needs the Brum user whose Proton login it uses".into());
        }
        let root = proton::cli_path_of(uri);
        proton::check_writable(&root).map_err(|_| "Backups to Proton Drive go into a folder inside My files, for example proton:///my-files/Backups".to_string())?;
        if proton::find_cli().is_none() {
            return Err("The Proton Drive program is not installed. Set it up in Settings → Proton Drive.".into());
        }
        Ok(Self { user: user.to_string(), label: format!("proton://{root}"), root, known_dirs: Mutex::new(BTreeSet::new()) })
    }

    fn cli(&self, rel: &str) -> String {
        let rel = rel.trim_matches('/');
        if rel.is_empty() {
            return self.root.clone();
        }
        rel.split('/').fold(self.root.clone(), |acc, part| proton::join_cli(&acc, part))
    }

    fn never() -> impl Fn() -> bool {
        || false
    }

    /// One folder level; missing folders are empty
    fn read_dir(&self, rel: &str, cancel: &dyn Fn() -> bool) -> Result<Vec<crate::vfs::FileEntry>, String> {
        match proton::list_sync(&self.user, &self.cli(rel), cancel) {
            Ok(v) => Ok(v),
            Err(e) if e.contains("Node not found") => Ok(Vec::new()),
            Err(e) => Err(e),
        }
    }

    fn meta_of(e: &crate::vfs::FileEntry) -> FileMeta {
        FileMeta { size: e.size, mtime: e.modified.unwrap_or(0) as i64 }
    }

    /// Proton Docs and Sheets cannot be downloaded, so a backup never touches them
    fn is_proton_doc(e: &crate::vfs::FileEntry) -> bool {
        e.mime_type.as_deref().map(|m| m.starts_with("application/vnd.proton.")).unwrap_or(false)
    }

    fn entry(&self, rel: &str) -> Option<crate::vfs::FileEntry> {
        let rel = rel.trim_matches('/');
        let (parent, name) = match rel.rsplit_once('/') {
            Some((p, n)) => (p, n),
            None => ("", rel),
        };
        self.read_dir(parent, &Self::never()).ok()?.into_iter().find(|e| e.name == name)
    }

    fn forget(&self, rel: &str) {
        proton::forget_listing(&self.user, &self.cli(rel));
    }

    /// Creates `rel` and the folders above it that are missing
    fn ensure_dir(&self, rel: &str) -> Result<(), String> {
        let rel = rel.trim_matches('/');
        if rel.is_empty() || self.known_dirs.lock().map(|k| k.contains(rel)).unwrap_or(false) {
            return Ok(());
        }
        let mut path = String::new();
        for part in rel.split('/') {
            let parent = path.clone();
            path = if path.is_empty() { part.to_string() } else { format!("{path}/{part}") };
            if self.known_dirs.lock().map(|k| k.contains(&path)).unwrap_or(false) {
                continue;
            }
            let exists = self.read_dir(&parent, &Self::never())?.iter().any(|e| e.is_dir && e.name == part);
            if !exists {
                match proton::run_sync(&self.user, &["filesystem", "create-folder", "-j", &self.cli(&parent), part], &Self::never(), None) {
                    Ok(_) => {}
                    // Another upload in the same run may have just made it
                    Err(e) if e.contains("already exists") => {}
                    Err(e) => return Err(e),
                }
                self.forget(&parent);
            }
            if let Ok(mut k) = self.known_dirs.lock() {
                k.insert(path.clone());
            }
        }
        Ok(())
    }

    /// Runs `jobs` on up to `PARALLEL` threads
    fn parallel<T: Send, R: Send>(jobs: Vec<T>, work: impl Fn(T) -> R + Sync) -> Vec<R> {
        let queue = Mutex::new(jobs.into_iter().enumerate().collect::<Vec<_>>());
        let results = Mutex::new(Vec::new());
        std::thread::scope(|scope| {
            for _ in 0..PARALLEL {
                scope.spawn(|| loop {
                    let next = queue.lock().ok().and_then(|mut q| q.pop());
                    let Some((i, job)) = next else { break };
                    let r = work(job);
                    if let Ok(mut out) = results.lock() {
                        out.push((i, r));
                    }
                });
            }
        });
        let mut out = results.into_inner().unwrap_or_default();
        out.sort_by_key(|(i, _)| *i);
        out.into_iter().map(|(_, r)| r).collect()
    }

    /// Uploads local files into the backup: `items` are (path in the backup, local file).
    /// `done(index, result)` is called as each item finishes.
    pub fn upload_batch(&self, items: &[(String, PathBuf)], cancel: &AtomicBool, done: &(dyn Fn(usize, BatchResult) + Sync)) {
        let is_cancelled = || cancel.load(Ordering::Relaxed);
        // Group by folder; each group becomes one or more upload calls
        let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (i, (rel, _)) in items.iter().enumerate() {
            let parent = rel.rsplit_once('/').map(|(p, _)| p.to_string()).unwrap_or_default();
            groups.entry(parent).or_default().push(i);
        }
        // Folders first, parents before children (sorted order guarantees that)
        let mut failed_dirs: HashMap<String, String> = HashMap::new();
        if let Err(e) = self.ensure_root() {
            for i in 0..items.len() {
                done(i, Err(e.clone()));
            }
            return;
        }
        for dir in groups.keys() {
            if is_cancelled() {
                break;
            }
            if let Err(e) = self.ensure_dir(dir) {
                failed_dirs.insert(dir.clone(), e);
            }
        }
        let mut calls: Vec<(String, Vec<usize>)> = Vec::new();
        for (dir, idx) in groups {
            if let Some(e) = failed_dirs.get(&dir) {
                for i in idx {
                    done(i, Err(format!("Could not create the folder: {e}")));
                }
                continue;
            }
            for chunk in idx.chunks(FILES_PER_CALL) {
                calls.push((dir.clone(), chunk.to_vec()));
            }
        }
        Self::parallel(calls, |(dir, idx)| {
            if is_cancelled() {
                for &i in &idx {
                    done(i, Err("Cancelled".into()));
                }
                return;
            }
            let mut args: Vec<String> = ["filesystem", "upload", "-j", "-f", "create-new-revision", "-d", "merge"].iter().map(|s| s.to_string()).collect();
            args.extend(idx.iter().map(|&i| items[i].1.to_string_lossy().to_string()));
            args.push(self.cli(&dir));
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            let res = proton::run_sync(&self.user, &refs, &is_cancelled, None);
            self.forget(&dir);
            match res.and_then(|out| Self::failures(&out)) {
                Ok(failures) => {
                    for &i in &idx {
                        let name = items[i].1.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                        match failures.iter().find(|(p, _)| p.ends_with(&name)) {
                            Some((_, why)) => done(i, Err(why.clone())),
                            None => done(i, Ok(std::fs::metadata(&items[i].1).map(|m| m.len()).unwrap_or(0))),
                        }
                    }
                }
                Err(e) => {
                    for &i in &idx {
                        done(i, Err(e.clone()));
                    }
                }
            }
        });
    }

    /// The failed items of an upload: (path, reason)
    fn failures(out: &str) -> Result<Vec<(String, String)>, String> {
        let v: serde_json::Value = out.lines().find(|l| l.trim_start().starts_with('{')).and_then(|l| serde_json::from_str(l).ok()).unwrap_or(serde_json::Value::Null);
        let list = v["failures"].as_array().cloned().unwrap_or_default();
        Ok(list
            .iter()
            .map(|f| {
                let p = f["path"].as_str().or_else(|| f["name"].as_str()).or_else(|| f["localPath"].as_str()).unwrap_or("").to_string();
                let why = f["error"].as_str().or_else(|| f["message"].as_str()).map(|s| s.to_string()).unwrap_or_else(|| f.to_string());
                (p, why)
            })
            .collect())
    }

    /// Moves backup files to Proton's trash, many per call
    pub fn trash_batch(&self, rels: &[String], cancel: &AtomicBool, done: &(dyn Fn(usize, BatchResult) + Sync)) {
        let is_cancelled = || cancel.load(Ordering::Relaxed);
        let idx: Vec<usize> = (0..rels.len()).collect();
        let chunks: Vec<Vec<usize>> = idx.chunks(FILES_PER_CALL).map(|c| c.to_vec()).collect();
        Self::parallel(chunks, |chunk| {
            if is_cancelled() {
                for &i in &chunk {
                    done(i, Err("Cancelled".into()));
                }
                return;
            }
            let paths: Vec<String> = chunk.iter().map(|&i| self.cli(&rels[i])).collect();
            let mut args = vec!["filesystem", "trash", "-j"];
            args.extend(paths.iter().map(String::as_str));
            let res = proton::run_sync(&self.user, &args, &is_cancelled, None);
            for &i in &chunk {
                let parent = rels[i].rsplit_once('/').map(|(p, _)| p.to_string()).unwrap_or_default();
                self.forget(&parent);
            }
            match res {
                // The answer lists each item with "ok"; a missing file counts as removed
                Ok(out) => {
                    let answers: Vec<serde_json::Value> = serde_json::from_str(out.trim()).unwrap_or_default();
                    for (k, &i) in chunk.iter().enumerate() {
                        let ok = answers.get(k).map(|a| a["ok"].as_bool().unwrap_or(true)).unwrap_or(true);
                        let why = answers.get(k).and_then(|a| a["error"].as_str()).unwrap_or("Proton Drive did not remove it");
                        done(i, if ok { Ok(0) } else { Err(why.to_string()) });
                    }
                }
                Err(e) => {
                    for &i in &chunk {
                        done(i, Err(e.clone()));
                    }
                }
            }
        });
    }
}

/// A downloaded copy that is deleted when reading is finished
struct TempRead {
    file: std::fs::File,
    dir: PathBuf,
}

impl Read for TempRead {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.file.read(buf)
    }
}

impl Drop for TempRead {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Store for ProtonStore {
    fn label(&self) -> String {
        self.label.clone()
    }

    fn root_exists(&self) -> bool {
        let (parent, name) = proton::split_cli_path(&self.root);
        proton::list_sync(&self.user, &parent, &Self::never()).map(|v| v.iter().any(|e| e.is_dir && e.name == name)).unwrap_or(false)
    }

    fn ensure_root(&self) -> Result<(), String> {
        let (parent, name) = proton::split_cli_path(&self.root);
        if proton::list_sync(&self.user, &parent, &Self::never()).map(|v| v.iter().any(|e| e.is_dir && e.name == name)).unwrap_or(false) {
            return Ok(());
        }
        // Create each missing level below My files
        let mut path = String::new();
        for part in self.root.trim_matches('/').split('/') {
            let parent = if path.is_empty() { "/".to_string() } else { path.clone() };
            path = proton::join_cli(&parent, part);
            if parent == "/" {
                continue; // my-files itself
            }
            let exists = proton::list_sync(&self.user, &parent, &Self::never())?.iter().any(|e| e.is_dir && e.name == part);
            if !exists {
                proton::run_sync(&self.user, &["filesystem", "create-folder", "-j", &parent, part], &Self::never(), None)?;
                proton::forget_listing(&self.user, &parent);
            }
        }
        Ok(())
    }

    fn list(&self, base: &str, skip: &dyn Fn(&str, bool) -> bool, cancel: &AtomicBool, warnings: &mut Vec<String>) -> Result<Listing, String> {
        let base = safe_rel(base)?;
        let is_cancelled = || cancel.load(Ordering::Relaxed);
        let out = Arc::new(Mutex::new(Listing::new()));
        let notes = Arc::new(Mutex::new(Vec::<String>::new()));
        let mut level = vec![String::new()];
        // One folder level at a time, the folders of a level side by side
        while !level.is_empty() {
            if is_cancelled() {
                return Err("Cancelled".into());
            }
            let results = Self::parallel(level, |rel: String| (rel.clone(), self.read_dir(&super::store::join_rel(&base, &rel), &is_cancelled)));
            let mut next = Vec::new();
            for (rel, res) in results {
                let entries = res?;
                if let Ok(mut k) = self.known_dirs.lock() {
                    k.insert(super::store::join_rel(&base, &rel));
                }
                for e in entries {
                    if e.name.contains('/') {
                        if let Ok(mut n) = notes.lock() {
                            n.push(format!("Skipped \"{}\" in Proton Drive: names with / cannot be copied", e.name));
                        }
                        continue;
                    }
                    let child = super::store::join_rel(&rel, &e.name);
                    if e.is_dir {
                        if !skip(&child, true) {
                            next.push(child);
                        }
                    } else if !Self::is_proton_doc(&e) && !skip(&child, false) {
                        if let Ok(mut o) = out.lock() {
                            o.insert(child, Self::meta_of(&e));
                        }
                    }
                }
            }
            level = next;
        }
        warnings.extend(notes.lock().map(|n| n.clone()).unwrap_or_default());
        let listing = out.lock().map(|o| o.clone()).unwrap_or_default();
        Ok(listing)
    }

    fn list_dir(&self, rel: &str) -> Result<Vec<DirItem>, String> {
        let rel = safe_rel(rel)?;
        Ok(self
            .read_dir(&rel, &Self::never())?
            .into_iter()
            .filter(|e| !Self::is_proton_doc(e))
            .map(|e| DirItem { name: e.name.clone(), is_dir: e.is_dir, size: e.size, mtime: e.modified.unwrap_or(0) as i64 })
            .collect())
    }

    fn stat(&self, rel: &str) -> Option<FileMeta> {
        self.entry(&safe_rel(rel).ok()?).filter(|e| !e.is_dir).map(|e| Self::meta_of(&e))
    }

    fn is_dir(&self, rel: &str) -> bool {
        let Ok(rel) = safe_rel(rel) else { return false };
        if rel.is_empty() {
            return self.root_exists();
        }
        self.entry(&rel).map(|e| e.is_dir).unwrap_or(false)
    }

    fn open_read(&self, rel: &str) -> Result<Box<dyn Read + Send>, String> {
        let rel = safe_rel(rel)?;
        let home = proton::user_home(&self.user);
        let dir = home.join("restore-tmp").join(format!("{}-{:x}", std::process::id(), rand::random::<u64>()));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let dir_s = dir.to_string_lossy().to_string();
        let res = proton::run_sync(&self.user, &["filesystem", "download", "-j", "-f", "remove", &self.cli(&rel), &dir_s], &Self::never(), None);
        if let Err(e) = res {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(e);
        }
        let name = rel.rsplit('/').next().unwrap_or(&rel);
        match std::fs::File::open(dir.join(name)) {
            Ok(file) => Ok(Box::new(TempRead { file, dir })),
            Err(_) => {
                let _ = std::fs::remove_dir_all(&dir);
                Err(format!("Proton Drive did not deliver {name}"))
            }
        }
    }

    fn write_part(&self, rel: &str, data: &mut dyn Read, mtime: i64, on_bytes: &mut dyn FnMut(u64) -> bool) -> Result<String, String> {
        // Rarely used (the backup run uploads in batches): write locally, then upload under the temporary name
        let rel = safe_rel(rel)?;
        let part = super::store::part_name(&rel);
        let home = proton::user_home(&self.user);
        let dir = home.join("upload-tmp").join(format!("{}-{:x}", std::process::id(), rand::random::<u64>()));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let local = dir.join(part.rsplit('/').next().unwrap_or(&part));
        let result = (|| {
            let mut f = std::fs::File::create(&local).map_err(|e| e.to_string())?;
            let mut buf = vec![0u8; 256 * 1024];
            let mut total = 0u64;
            loop {
                let n = data.read(&mut buf).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                std::io::Write::write_all(&mut f, &buf[..n]).map_err(|e| e.to_string())?;
                total += n as u64;
                if !on_bytes(total) {
                    return Err("Cancelled".to_string());
                }
            }
            drop(f);
            let ft = std::time::UNIX_EPOCH + std::time::Duration::from_secs(mtime.max(0) as u64);
            let _ = std::fs::File::options().write(true).open(&local).and_then(|f| f.set_modified(ft));
            let parent = part.rsplit_once('/').map(|(p, _)| p.to_string()).unwrap_or_default();
            self.ensure_root()?;
            self.ensure_dir(&parent)?;
            let local_s = local.to_string_lossy().to_string();
            let out = proton::run_sync(&self.user, &["filesystem", "upload", "-j", "-f", "replace", &local_s, &self.cli(&parent)], &Self::never(), None)?;
            proton::check_transfer_result(&out)?;
            self.forget(&parent);
            Ok(part.clone())
        })();
        let _ = std::fs::remove_dir_all(&dir);
        result
    }

    fn rename(&self, from: &str, to: &str) -> Result<(), String> {
        let (from, to) = (safe_rel(from)?, safe_rel(to)?);
        let from_parent = from.rsplit_once('/').map(|(p, _)| p.to_string()).unwrap_or_default();
        let to_parent = to.rsplit_once('/').map(|(p, _)| p.to_string()).unwrap_or_default();
        let to_name = to.rsplit('/').next().unwrap_or(&to).to_string();
        let from_name = from.rsplit('/').next().unwrap_or(&from).to_string();
        // Replaces an existing target, like a local rename (Proton keeps it in the trash)
        if self.entry(&to).is_some() {
            proton::run_sync(&self.user, &["filesystem", "trash", "-j", &self.cli(&to)], &Self::never(), None)?;
            self.forget(&to_parent);
        }
        let mut current = self.cli(&from);
        if from_parent != to_parent {
            self.ensure_dir(&to_parent)?;
            proton::run_sync(&self.user, &["filesystem", "move", "-j", &current, &self.cli(&to_parent)], &Self::never(), None)?;
            current = proton::join_cli(&self.cli(&to_parent), &from_name);
        }
        if from_name != to_name {
            proton::run_sync(&self.user, &["filesystem", "rename", "-j", &current, &to_name], &Self::never(), None)?;
        }
        self.forget(&from_parent);
        self.forget(&to_parent);
        Ok(())
    }

    fn remove_file(&self, rel: &str) -> Result<(), String> {
        let rel = safe_rel(rel)?;
        let parent = rel.rsplit_once('/').map(|(p, _)| p.to_string()).unwrap_or_default();
        let r = proton::run_sync(&self.user, &["filesystem", "trash", "-j", &self.cli(&rel)], &Self::never(), None);
        self.forget(&parent);
        match r {
            Ok(_) => Ok(()),
            Err(e) if e.contains("Node not found") => Ok(()),
            Err(e) => Err(e),
        }
    }

    fn remove_dir_all(&self, rel: &str) -> Result<(), String> {
        self.remove_file(rel)
    }

    fn remove_empty_parents(&self, _rel: &str) {}

    fn local_path(&self, _rel: &str) -> Option<PathBuf> {
        None
    }

    fn proton(&self) -> Option<&ProtonStore> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_failures_are_read() {
        let out = r#"{"transferredItems":1,"transferredBytes":4,"skippedItems":0,"failedItems":1,"failures":[{"path":"/tmp/x/b.txt","error":"Quota exceeded"}]}"#;
        assert_eq!(ProtonStore::failures(out).unwrap(), vec![("/tmp/x/b.txt".to_string(), "Quota exceeded".to_string())]);
        assert!(ProtonStore::failures(r#"{"transferredItems":2,"failedItems":0,"failures":[]}"#).unwrap().is_empty());
        assert!(ProtonStore::failures("undefined").unwrap().is_empty());
    }

    #[test]
    fn parallel_keeps_order_and_runs_everything() {
        let out = ProtonStore::parallel((0..20).collect(), |i: i32| i * 2);
        assert_eq!(out, (0..20).map(|i| i * 2).collect::<Vec<_>>());
    }
}
