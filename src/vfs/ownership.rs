//! People work as themselves on the system (#208).
//!
//! When the Brum service runs as root, file actions are carried out by root. So that a
//! signed-in user's new files do not end up owned by root, Brum hands what it creates to
//! the user's own Linux account, but only inside folders that already belong to that
//! account (their home, their own folders). System areas and folders owned by root or
//! by someone else are left as they are. Saving an existing file keeps its owner and
//! permissions wherever it is.
//!
//! Nothing happens when Brum runs as a normal user (desktop app), when the signed-in
//! user has no matching Linux account, for root, and on Windows.

use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// A Linux account that new files are handed to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Owner {
    pub uid: u32,
    pub gid: u32,
}

/// The account to hand new files to for `username`: only when Brum runs as root and the
/// user maps to a Linux account that is not root.
#[cfg(unix)]
pub fn for_user(username: &str) -> Option<Owner> {
    if unsafe { libc::geteuid() } != 0 {
        return None;
    }
    let pu = crate::server::terminal::lookup_posix_user(username)?;
    (pu.uid != 0).then_some(Owner { uid: pu.uid, gid: pu.gid })
}

#[cfg(not(unix))]
pub fn for_user(_username: &str) -> Option<Owner> {
    None
}

#[cfg(unix)]
fn owner_of(path: &Path) -> Option<(u32, u32)> {
    use std::os::unix::fs::MetadataExt;
    std::fs::symlink_metadata(path).ok().map(|m| (m.uid(), m.gid()))
}

/// Give `path` to `owner` if root owns it and its folder belongs to `owner`.
/// Symlinks are changed themselves, never followed. Returns whether it was handed over.
#[cfg(unix)]
fn adopt_one(path: &Path, owner: Owner) -> bool {
    let Some((uid, _)) = owner_of(path) else { return false };
    if uid != 0 {
        return false;
    }
    let parent_ok = path
        .parent()
        .and_then(owner_of)
        .map(|(puid, _)| puid == owner.uid)
        .unwrap_or(false);
    if !parent_ok {
        return false;
    }
    match std::os::unix::fs::lchown(path, Some(owner.uid), Some(owner.gid)) {
        Ok(()) => true,
        Err(e) => {
            tracing::debug!("Could not hand {} to uid {}: {}", path.display(), owner.uid, e);
            false
        }
    }
}

/// Hand a newly created file or folder to `owner` (see the module notes). For a folder,
/// everything inside that root owns is handed over too, top-down, so each entry's folder
/// already belongs to the user when it is checked.
#[cfg(unix)]
pub fn adopt(path: &Path, owner: Option<Owner>) {
    let Some(owner) = owner else { return };
    adopt_one(path, owner);
    let is_dir = std::fs::symlink_metadata(path).map(|m| m.is_dir()).unwrap_or(false);
    if !is_dir {
        return;
    }
    for entry in walkdir::WalkDir::new(path).min_depth(1).follow_links(false).into_iter().filter_map(|e| e.ok()) {
        adopt_one(entry.path(), owner);
    }
}

#[cfg(not(unix))]
pub fn adopt(_path: &Path, _owner: Option<Owner>) {}

/// After `mkdir -p`: hand over the folder and any parents it had to create.
#[cfg(unix)]
pub fn adopt_with_parents(path: &Path, owner: Option<Owner>) {
    let Some(owner) = owner else { return };
    let mut chain: Vec<PathBuf> = Vec::new();
    let mut cur = Some(path);
    while let Some(p) = cur {
        match owner_of(p) {
            Some((0, _)) => chain.push(p.to_path_buf()),
            _ => break,
        }
        cur = p.parent();
    }
    for p in chain.iter().rev() {
        adopt_one(p, owner);
    }
}

#[cfg(not(unix))]
pub fn adopt_with_parents(_path: &Path, _owner: Option<Owner>) {}

/// An existing file's owner and permissions, kept across a save that replaces the file.
#[derive(Debug, Clone, Copy)]
pub struct Snapshot {
    #[allow(dead_code)]
    uid: u32,
    #[allow(dead_code)]
    gid: u32,
    #[allow(dead_code)]
    mode: u32,
}

#[cfg(unix)]
pub fn snapshot(path: &Path) -> Option<Snapshot> {
    use std::os::unix::fs::MetadataExt;
    let m = std::fs::symlink_metadata(path).ok()?;
    m.is_file().then(|| Snapshot { uid: m.uid(), gid: m.gid(), mode: m.mode() & 0o7777 })
}

#[cfg(not(unix))]
pub fn snapshot(_path: &Path) -> Option<Snapshot> {
    None
}

/// Put back the owner and permissions a file had before it was saved.
#[cfg(unix)]
pub fn restore(path: &Path, snap: &Snapshot) {
    use std::os::unix::fs::PermissionsExt;
    if unsafe { libc::geteuid() } == 0 {
        if let Err(e) = std::os::unix::fs::lchown(path, Some(snap.uid), Some(snap.gid)) {
            tracing::debug!("Could not restore the owner of {}: {}", path.display(), e);
        }
    }
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(snap.mode));
}

#[cfg(not(unix))]
pub fn restore(_path: &Path, _snap: &Snapshot) {}

/// Remembers which entries a folder had before an operation, so the new ones can be
/// handed over afterwards (copy, move, upload, unpack may rename on conflicts).
pub struct Watch {
    dir: PathBuf,
    before: HashSet<OsString>,
    owner: Option<Owner>,
}

impl Watch {
    /// The folder being watched.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn new(dir: impl Into<PathBuf>, owner: Option<Owner>) -> Self {
        let dir = dir.into();
        let before = if owner.is_some() {
            std::fs::read_dir(&dir)
                .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.file_name()).collect())
                .unwrap_or_default()
        } else {
            HashSet::new()
        };
        Watch { dir, before, owner }
    }

    /// Hand over every new entry, plus `also` (entries that may have been merged into
    /// folders that existed before).
    pub fn finish(self, also: &[PathBuf]) {
        if self.owner.is_none() {
            return;
        }
        if let Ok(rd) = std::fs::read_dir(&self.dir) {
            for e in rd.filter_map(|e| e.ok()) {
                if !self.before.contains(&e.file_name()) {
                    adopt(&e.path(), self.owner);
                }
            }
        }
        for p in also {
            if p.starts_with(&self.dir) && p.exists() {
                adopt(p, self.owner);
            }
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn nothing_happens_without_an_owner() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.txt");
        std::fs::write(&f, b"x").unwrap();
        let before = owner_of(&f);
        adopt(&f, None);
        adopt_with_parents(&f, None);
        Watch::new(dir.path(), None).finish(&[f.clone()]);
        assert_eq!(owner_of(&f), before);
    }

    #[test]
    fn files_not_owned_by_root_are_never_touched() {
        // Tests run as a normal user: the file is not root's, so it must stay as it is.
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.txt");
        std::fs::write(&f, b"x").unwrap();
        let (uid, gid) = owner_of(&f).unwrap();
        if uid == 0 {
            return;
        }
        assert!(!adopt_one(&f, Owner { uid: uid + 1, gid }));
        assert_eq!(owner_of(&f), Some((uid, gid)));
    }

    #[test]
    fn snapshot_restores_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("script.sh");
        std::fs::write(&f, b"#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o750)).unwrap();
        let snap = snapshot(&f).unwrap();
        std::fs::remove_file(&f).unwrap();
        std::fs::write(&f, b"#!/bin/sh\necho saved\n").unwrap();
        restore(&f, &snap);
        assert_eq!(std::fs::metadata(&f).unwrap().permissions().mode() & 0o7777, 0o750);
    }
}
