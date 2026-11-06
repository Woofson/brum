//! Works out what a backup needs to do (the preview) and then does it.

use super::exclude::Exclusions;
use super::store::{checksum, join_rel, CrcReader, FileMeta, Listing, Store, PART_SUFFIX};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Default folder (at the destination) that keeps replaced and removed files
pub const VERSIONS_DIR: &str = ".brum-versions";
/// Format of the dated folders inside the versions folder
pub const STAMP_FORMAT: &str = "%Y-%m-%d_%H%M%S";
/// Large files that changed are updated in place, part by part, when both sides are local
const PARTIAL_UPDATE_MIN: u64 = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Exact copy: removed source files are removed from the backup
    Copy,
    /// Exact copy, but replaced and removed files are kept as old versions
    Versions,
    /// Keep two folders the same, both ways
    TwoWay,
    /// Exact copy into a git repository; every run saves a version (commit)
    Git,
}

impl Mode {
    /// Accepts today's names and the old tool's names. The bool is true for the old
    /// "contribute" mode, which never removed anything from the backup.
    pub fn parse(s: &str) -> (Mode, bool) {
        match s.to_lowercase().as_str() {
            "versions" | "subscribe" | "archive" | "versioning" => (Mode::Versions, false),
            "two_way" | "twoway" | "synchronize" | "sync" | "bidirectional_newer" | "2way" => (Mode::TwoWay, false),
            "git" | "collection" => (Mode::Git, false),
            "contribute" | "additive" => (Mode::Copy, true),
            _ => (Mode::Copy, false),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Mode::Copy => "copy",
            Mode::Versions => "versions",
            Mode::TwoWay => "two_way",
            Mode::Git => "git",
        }
    }
}

/// What a backup run compares and how
pub struct JobSpec {
    pub source: Arc<dyn Store>,
    pub dest: Arc<dyn Store>,
    pub mode: Mode,
    pub never_remove: bool,
    pub verify: bool,
    pub partial_updates: bool,
    pub exclusions: Exclusions,
    pub versions_dir: String,
    pub retention_days: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// New file, copied to the destination
    Copy,
    /// Changed file, updated at the destination
    Update,
    /// Gone from the source, removed from the destination
    Remove,
    /// Two-way: new on the destination side, copied to the source side
    CopyBack,
    /// Two-way: changed on the destination side
    UpdateBack,
    /// Two-way: removed on the destination side, so removed on the source side too
    RemoveBack,
}

impl Action {
    pub fn as_str(&self) -> &'static str {
        match self {
            Action::Copy => "copy",
            Action::Update => "update",
            Action::Remove => "remove",
            Action::CopyBack => "copy_back",
            Action::UpdateBack => "update_back",
            Action::RemoveBack => "remove_back",
        }
    }
    fn towards_source(&self) -> bool {
        matches!(self, Action::CopyBack | Action::UpdateBack | Action::RemoveBack)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanItem {
    pub path: String,
    pub action: Action,
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct PlanSummary {
    pub copy: u64,
    pub update: u64,
    pub remove: u64,
    pub copy_back: u64,
    pub update_back: u64,
    pub remove_back: u64,
    pub unchanged: u64,
    pub conflicts: u64,
    /// Data to copy, in bytes
    pub bytes: u64,
    pub source_files: u64,
    pub source_bytes: u64,
}

pub struct Plan {
    pub items: Vec<PlanItem>,
    pub summary: PlanSummary,
    pub warnings: Vec<String>,
    pub source_files: Listing,
    pub dest_files: Listing,
}

/// One line of the per-file log
#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    pub path: String,
    pub action: String,
    pub size: u64,
    pub ok: bool,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RunCounts {
    pub copied: u64,
    pub updated: u64,
    pub removed: u64,
    pub versions_kept: u64,
    pub failed: u64,
    pub bytes: u64,
}

/// Live progress of a run
#[derive(Debug, Clone, Default, Serialize)]
pub struct Progress {
    pub phase: String, // "checking", "copying", "cleaning"
    pub files_done: u64,
    pub files_total: u64,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub current: Option<String>,
}

pub struct RunOutcome {
    pub counts: RunCounts,
    pub cancelled: bool,
    /// Two-way only: what both sides look like after the run, remembered for next time
    pub new_state: Option<Listing>,
}

fn skip_fn<'a>(spec: &'a JobSpec, is_dest_side: bool) -> impl Fn(&str, bool) -> bool + 'a {
    let versions_on_this_side = is_dest_side || spec.mode == Mode::TwoWay;
    move |rel: &str, _is_dir: bool| {
        if rel.ends_with(PART_SUFFIX) {
            return true;
        }
        // A git collection's own files are never part of the copy
        if is_dest_side && spec.mode == Mode::Git && super::git_repo::is_own_file(rel) {
            return true;
        }
        if versions_on_this_side && (rel == spec.versions_dir || rel.starts_with(&format!("{}/", spec.versions_dir))) {
            return true;
        }
        spec.exclusions.is_excluded(rel)
    }
}

/// Compares both sides. `state` is what both sides looked like after the last two-way run.
pub fn plan(spec: &JobSpec, state: Option<&Listing>, cancel: &AtomicBool) -> Result<Plan, String> {
    if !spec.source.root_exists() {
        return Err(format!("The source folder is missing or not reachable: {}", spec.source.label()));
    }
    // A first two-way sync may create the second folder; after that, a missing one is a disconnected drive
    if spec.mode == Mode::TwoWay && !spec.dest.root_exists() && state.is_some_and(|s| !s.is_empty()) {
        return Err(format!("The second folder is missing or not reachable: {}", spec.dest.label()));
    }
    let mut warnings = Vec::new();
    let src = spec.source.list("", &skip_fn(spec, false), cancel, &mut warnings)?;
    let dst = if spec.dest.root_exists() {
        spec.dest.list("", &skip_fn(spec, true), cancel, &mut warnings)?
    } else {
        Listing::new()
    };
    let mut summary = PlanSummary {
        source_files: src.len() as u64,
        source_bytes: src.values().map(|m| m.size).sum(),
        ..Default::default()
    };
    let mut items = Vec::new();

    if spec.mode == Mode::TwoWay {
        let empty = Listing::new();
        let last = state.unwrap_or(&empty);
        // A side that is suddenly empty is far more likely an unmounted drive than a wish to delete everything
        if !last.is_empty() && (src.is_empty() || dst.is_empty()) {
            let which = if src.is_empty() { spec.source.label() } else { spec.dest.label() };
            return Err(format!("{which} is empty, but it had files at the last sync. Is the drive or share connected? Nothing was changed."));
        }
        plan_two_way(&src, &dst, last, &mut items, &mut summary);
    } else {
        if src.is_empty() && !dst.is_empty() && !spec.never_remove {
            return Err(format!(
                "The source folder {} is empty, but the backup has {} files. Is the drive or share connected? Nothing was changed.",
                spec.source.label(),
                dst.len()
            ));
        }
        // With several sources, the same check for each one
        if !spec.never_remove {
            for name in spec.source.part_names() {
                let prefix = format!("{name}/");
                let inside = |k: &String| k == &name || k.starts_with(&prefix);
                let in_backup = dst.keys().filter(|k| inside(k)).count();
                if in_backup > 0 && !src.keys().any(inside) {
                    return Err(format!(
                        "\"{name}\" is missing or empty, but the backup has {in_backup} files from it. Is its drive or share connected? If you removed it on purpose, take it out of the backup. Nothing was changed."
                    ));
                }
            }
        }
        for (rel, s) in &src {
            match dst.get(rel) {
                None => items.push(PlanItem { path: rel.clone(), action: Action::Copy, size: s.size, note: None }),
                Some(d) if s.same_as(d) => summary.unchanged += 1,
                Some(_) => items.push(PlanItem { path: rel.clone(), action: Action::Update, size: s.size, note: None }),
            }
        }
        if !spec.never_remove {
            for (rel, d) in &dst {
                if !src.contains_key(rel) {
                    items.push(PlanItem { path: rel.clone(), action: Action::Remove, size: d.size, note: None });
                }
            }
        }
    }

    for it in &items {
        match it.action {
            Action::Copy => summary.copy += 1,
            Action::Update => summary.update += 1,
            Action::Remove => summary.remove += 1,
            Action::CopyBack => summary.copy_back += 1,
            Action::UpdateBack => summary.update_back += 1,
            Action::RemoveBack => summary.remove_back += 1,
        }
        if !matches!(it.action, Action::Remove | Action::RemoveBack) {
            summary.bytes += it.size;
        }
        if it.note.as_deref().is_some_and(|n| n.starts_with("Changed on both sides")) {
            summary.conflicts += 1;
        }
    }
    // Copies first, removals last, so an interrupted run never loses data
    items.sort_by(|a, b| {
        let rank = |x: &PlanItem| matches!(x.action, Action::Remove | Action::RemoveBack) as u8;
        rank(a).cmp(&rank(b)).then_with(|| a.path.cmp(&b.path))
    });
    Ok(Plan { items, summary, warnings, source_files: src, dest_files: dst })
}

fn plan_two_way(a: &Listing, b: &Listing, last: &Listing, items: &mut Vec<PlanItem>, summary: &mut PlanSummary) {
    let mut paths: Vec<&String> = a.keys().chain(b.keys()).collect();
    paths.sort();
    paths.dedup();
    for rel in paths {
        let (sa, sb, prev) = (a.get(rel), b.get(rel), last.get(rel));
        let changed = |m: &FileMeta| prev.map_or(true, |p| !m.same_as(p));
        let item = |action, size, note: Option<&str>| PlanItem { path: rel.clone(), action, size, note: note.map(String::from) };
        match (sa, sb) {
            (Some(x), Some(y)) if x.same_as(y) => summary.unchanged += 1,
            (Some(x), Some(y)) => {
                let (ca, cb) = (changed(x), changed(y));
                if ca && !cb {
                    items.push(item(Action::Update, x.size, None));
                } else if cb && !ca {
                    items.push(item(Action::UpdateBack, y.size, None));
                } else if x.mtime >= y.mtime {
                    // Both changed (or first sync): the newer one wins, the other is kept as an old version
                    items.push(item(Action::Update, x.size, Some("Changed on both sides; the newer one was kept, the other saved as an old version")));
                } else {
                    items.push(item(Action::UpdateBack, y.size, Some("Changed on both sides; the newer one was kept, the other saved as an old version")));
                }
            }
            (Some(x), None) => {
                if prev.is_some() && !changed(x) {
                    items.push(item(Action::RemoveBack, x.size, None));
                } else {
                    items.push(item(Action::Copy, x.size, None));
                }
            }
            (None, Some(y)) => {
                if prev.is_some() && !changed(y) {
                    items.push(item(Action::Remove, y.size, None));
                } else {
                    items.push(item(Action::CopyBack, y.size, None));
                }
            }
            (None, None) => {}
        }
    }
}

/// Carries out a plan. Every file action is reported through `log`.
pub fn execute(
    spec: &JobSpec,
    plan: &Plan,
    state: Option<&Listing>,
    progress: &dyn Fn(Progress),
    log: &mut dyn FnMut(LogEntry),
    cancel: &AtomicBool,
) -> Result<RunOutcome, String> {
    spec.dest.ensure_root()?;
    if let Some(pd) = spec.dest.proton() {
        if matches!(spec.mode, Mode::Copy | Mode::Versions) {
            return execute_proton(spec, pd, plan, progress, log, cancel);
        }
        return Err("Proton Drive backups can copy, or copy and keep old versions".into());
    }
    // Git keeps its own history, so only these modes keep old versions in a folder
    let keep_versions = matches!(spec.mode, Mode::Versions | Mode::TwoWay);
    let stamp_dir = join_rel(&spec.versions_dir, &chrono::Local::now().format(STAMP_FORMAT).to_string());
    let mut counts = RunCounts::default();
    let mut p = Progress {
        phase: "copying".into(),
        files_total: plan.items.len() as u64,
        bytes_total: plan.summary.bytes,
        ..Default::default()
    };
    let mut succeeded = vec![false; plan.items.len()];
    let mut cancelled = false;

    for (idx, it) in plan.items.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            cancelled = true;
            break;
        }
        p.current = Some(it.path.clone());
        progress(p.clone());
        let (from, to): (&dyn Store, &dyn Store) = if it.action.towards_source() {
            (spec.dest.as_ref(), spec.source.as_ref())
        } else {
            (spec.source.as_ref(), spec.dest.as_ref())
        };
        let result = match it.action {
            Action::Remove | Action::RemoveBack => remove(to, &it.path, keep_versions, &stamp_dir),
            _ => {
                let meta = if it.action.towards_source() { plan.dest_files.get(&it.path) } else { plan.source_files.get(&it.path) };
                let meta = *meta.ok_or("missing file details")?;
                let base = p.bytes_done;
                let mut on_bytes = |n: u64| {
                    let mut q = p.clone();
                    q.bytes_done = base + n;
                    progress(q);
                    !cancel.load(Ordering::Relaxed)
                };
                transfer(spec, from, to, &it.path, meta, keep_versions, &stamp_dir, cancel, &mut on_bytes)
            }
        };
        match result {
            Ok(done) => {
                succeeded[idx] = true;
                match it.action {
                    Action::Copy | Action::CopyBack => counts.copied += 1,
                    Action::Update | Action::UpdateBack => counts.updated += 1,
                    Action::Remove | Action::RemoveBack => counts.removed += 1,
                }
                if done.version_kept {
                    counts.versions_kept += 1;
                }
                counts.bytes += done.bytes;
                let mut detail = Vec::new();
                if let Some(n) = &it.note {
                    detail.push(n.clone());
                }
                if done.version_kept {
                    detail.push("old version kept".to_string());
                }
                if done.partial {
                    detail.push("only changed parts written".to_string());
                }
                log(LogEntry { path: it.path.clone(), action: it.action.as_str().into(), size: it.size, ok: true, detail: (!detail.is_empty()).then(|| detail.join("; ")) });
            }
            Err(e) if e == "Cancelled" => {
                cancelled = true;
                break;
            }
            Err(e) => {
                counts.failed += 1;
                log(LogEntry { path: it.path.clone(), action: it.action.as_str().into(), size: it.size, ok: false, detail: Some(e) });
            }
        }
        p.files_done += 1;
        if !matches!(it.action, Action::Remove | Action::RemoveBack) {
            p.bytes_done += it.size;
        }
    }

    if !cancelled && keep_versions && spec.retention_days > 0 {
        p.phase = "cleaning".into();
        p.current = None;
        progress(p.clone());
        prune_versions(spec.dest.as_ref(), &spec.versions_dir, spec.retention_days, log);
        if spec.mode == Mode::TwoWay {
            prune_versions(spec.source.as_ref(), &spec.versions_dir, spec.retention_days, log);
        }
    }

    let new_state = (spec.mode == Mode::TwoWay).then(|| next_state(plan, &succeeded, state));
    Ok(RunOutcome { counts, cancelled, new_state })
}

/// A backup into Proton Drive: uploads and removals in batches (each call to Proton's program is
/// slow). Replaced files stay in Proton's version history and removed ones in Proton's trash.
fn execute_proton(
    spec: &JobSpec,
    pd: &super::proton_store::ProtonStore,
    plan: &Plan,
    progress: &dyn Fn(Progress),
    log: &mut dyn FnMut(LogEntry),
    cancel: &AtomicBool,
) -> Result<RunOutcome, String> {
    use std::sync::Mutex;
    let mut counts = RunCounts::default();
    let mut p = Progress { phase: "copying".into(), files_total: plan.items.len() as u64, bytes_total: plan.summary.bytes, ..Default::default() };
    let mut cancelled = false;
    progress(p.clone());

    let mut uploads: Vec<(String, std::path::PathBuf)> = Vec::new();
    let mut upload_items: Vec<&PlanItem> = Vec::new();
    let mut one_by_one: Vec<&PlanItem> = Vec::new();
    let mut removals: Vec<&PlanItem> = Vec::new();
    for it in &plan.items {
        match it.action {
            Action::Copy | Action::Update => match spec.source.local_path(&it.path) {
                Some(lp) => {
                    uploads.push((it.path.clone(), lp));
                    upload_items.push(it);
                }
                None => one_by_one.push(it),
            },
            Action::Remove => removals.push(it),
            _ => {}
        }
    }

    // Applies finished items to the counts, the log and the progress
    let mut apply = |it: &PlanItem, res: Result<u64, String>, p: &mut Progress, counts: &mut RunCounts, cancelled: &mut bool, log: &mut dyn FnMut(LogEntry)| {
        match res {
            Ok(bytes) => {
                let detail = match it.action {
                    Action::Update if spec.mode == Mode::Versions => Some("old version kept in Proton's version history".to_string()),
                    Action::Remove => Some("moved to Proton's trash".to_string()),
                    _ => None,
                };
                match it.action {
                    Action::Copy => counts.copied += 1,
                    Action::Update => counts.updated += 1,
                    _ => counts.removed += 1,
                }
                counts.bytes += bytes;
                log(LogEntry { path: it.path.clone(), action: it.action.as_str().into(), size: it.size, ok: true, detail });
            }
            Err(e) if e == "Cancelled" => *cancelled = true,
            Err(e) => {
                counts.failed += 1;
                log(LogEntry { path: it.path.clone(), action: it.action.as_str().into(), size: it.size, ok: false, detail: Some(e) });
            }
        }
        p.files_done += 1;
        if it.action != Action::Remove {
            p.bytes_done += it.size;
        }
    };

    // Runs a batch on a helper thread while this one reports progress as items finish
    let run_batch = |items: &[&PlanItem], start: &(dyn Fn(&(dyn Fn(usize, Result<u64, String>) + Sync)) + Sync), p: &mut Progress, counts: &mut RunCounts, cancelled: &mut bool, log: &mut dyn FnMut(LogEntry), apply: &mut dyn FnMut(&PlanItem, Result<u64, String>, &mut Progress, &mut RunCounts, &mut bool, &mut dyn FnMut(LogEntry))| {
        let finished: Mutex<Vec<(usize, Result<u64, String>)>> = Mutex::new(Vec::new());
        std::thread::scope(|scope| {
            let handle = scope.spawn(|| start(&|i, r| {
                if let Ok(mut f) = finished.lock() {
                    f.push((i, r));
                }
            }));
            loop {
                let done_now = handle.is_finished();
                let batch: Vec<_> = finished.lock().map(|mut f| f.drain(..).collect()).unwrap_or_default();
                for (i, r) in batch {
                    if let Some(it) = items.get(i) {
                        p.current = Some(it.path.clone());
                        apply(it, r, p, counts, cancelled, log);
                    }
                }
                progress(p.clone());
                if done_now {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(300));
            }
        });
    };

    if !uploads.is_empty() {
        run_batch(&upload_items, &|done| pd.upload_batch(&uploads, cancel, done), &mut p, &mut counts, &mut cancelled, log, &mut apply);
    }

    // Sources without a local path (SFTP, rclone…) are copied one file at a time
    for it in one_by_one {
        if cancelled || cancel.load(Ordering::Relaxed) {
            cancelled = true;
            break;
        }
        p.current = Some(it.path.clone());
        progress(p.clone());
        let res = match plan.source_files.get(&it.path) {
            Some(meta) => {
                let mut on_bytes = |_n: u64| !cancel.load(Ordering::Relaxed);
                transfer(spec, spec.source.as_ref(), spec.dest.as_ref(), &it.path, *meta, false, "", cancel, &mut on_bytes).map(|t| t.bytes)
            }
            None => Err("missing file details".to_string()),
        };
        apply(it, res, &mut p, &mut counts, &mut cancelled, log);
    }

    // Removals last, so an interrupted run never loses data
    if !removals.is_empty() && !cancelled && !cancel.load(Ordering::Relaxed) {
        p.phase = "removing".into();
        let rels: Vec<String> = removals.iter().map(|it| it.path.clone()).collect();
        run_batch(&removals, &|done| pd.trash_batch(&rels, cancel, done), &mut p, &mut counts, &mut cancelled, log, &mut apply);
    }
    if cancel.load(Ordering::Relaxed) {
        cancelled = true;
    }
    Ok(RunOutcome { counts, cancelled, new_state: None })
}

/// What both sides look like after a two-way run; failed items keep their old state so they are retried
fn next_state(plan: &Plan, succeeded: &[bool], old: Option<&Listing>) -> Listing {
    let mut state = Listing::new();
    let touched: std::collections::HashMap<&str, usize> = plan.items.iter().enumerate().map(|(i, it)| (it.path.as_str(), i)).collect();
    for (rel, a) in &plan.source_files {
        if !touched.contains_key(rel.as_str()) && plan.dest_files.get(rel).is_some_and(|b| a.same_as(b)) {
            state.insert(rel.clone(), *a);
        }
    }
    for (i, it) in plan.items.iter().enumerate() {
        if succeeded[i] {
            let meta = match it.action {
                Action::Copy | Action::Update => plan.source_files.get(&it.path),
                Action::CopyBack | Action::UpdateBack => plan.dest_files.get(&it.path),
                Action::Remove | Action::RemoveBack => None,
            };
            if let Some(m) = meta {
                state.insert(it.path.clone(), *m);
            }
        } else if let Some(m) = old.and_then(|o| o.get(&it.path)) {
            state.insert(it.path.clone(), *m);
        }
    }
    state
}

struct Transferred {
    bytes: u64,
    version_kept: bool,
    partial: bool,
}

#[allow(clippy::too_many_arguments)]
fn transfer(
    spec: &JobSpec,
    from: &dyn Store,
    to: &dyn Store,
    rel: &str,
    meta: FileMeta,
    keep_versions: bool,
    stamp_dir: &str,
    cancel: &AtomicBool,
    on_bytes: &mut dyn FnMut(u64) -> bool,
) -> Result<Transferred, String> {
    let existing = to.stat(rel);

    // Exact copies of large files on local disks: rewrite only the parts that changed
    if spec.partial_updates && !keep_versions && meta.size >= PARTIAL_UPDATE_MIN && existing.is_some() {
        if let (Some(s), Some(d)) = (from.local_path(rel), to.local_path(rel)) {
            let written = super::store::update_changed_blocks(&s, &d, meta.mtime, cancel, on_bytes)?;
            if spec.verify && checksum(from, rel)? != checksum(to, rel)? {
                return Err("Verification failed: the copy does not match the original".into());
            }
            copy_permissions(from, to, rel);
            return Ok(Transferred { bytes: written, version_kept: false, partial: true });
        }
    }

    let (part, bytes) = copy_to_part(from, rel, to, rel, meta, spec.verify, on_bytes)?;
    let fail = |msg: String| -> Result<Transferred, String> {
        let _ = to.remove_file(&part);
        Err(msg)
    };
    let mut version_kept = false;
    if existing.is_some() && keep_versions {
        if let Err(e) = to.rename(rel, &join_rel(stamp_dir, rel)) {
            return fail(format!("Could not keep the old version: {e}"));
        }
        version_kept = true;
    }
    if let Err(e) = to.rename(&part, rel) {
        if version_kept {
            let _ = to.rename(&join_rel(stamp_dir, rel), rel);
        }
        return fail(e);
    }
    copy_permissions(from, to, rel);
    Ok(Transferred { bytes, version_kept, partial: false })
}

/// Copies `from_rel` to a temporary name beside `to_rel`, checked and with the original's time.
/// Returns the temporary name and the bytes copied; the caller swaps it in or removes it.
pub fn copy_to_part(
    from: &dyn Store,
    from_rel: &str,
    to: &dyn Store,
    to_rel: &str,
    meta: FileMeta,
    verify: bool,
    on_bytes: &mut dyn FnMut(u64) -> bool,
) -> Result<(String, u64), String> {
    // rclone copies straight from place to place, keeping times and checking the transfer
    let part = super::store::part_name(to_rel);
    if let (Some(src), Some(dst)) = (from.rclone_location(from_rel), to.rclone_location(&part)) {
        if let Some(res) = to.rclone_copy(&src, &dst).or_else(|| from.rclone_copy(&src, &dst)) {
            let fail = |msg: String| -> Result<(String, u64), String> {
                let _ = to.remove_file(&part);
                Err(msg)
            };
            if let Err(e) = res {
                return fail(e);
            }
            match to.stat(&part) {
                Some(m) if m.size == meta.size => {}
                Some(_) => return fail("The file changed while it was being copied; it will be copied again next time".into()),
                None => return fail("rclone reported success, but the copy is missing".into()),
            }
            if verify {
                match (checksum(from, from_rel), checksum(to, &part)) {
                    (Ok(a), Ok(b)) if a == b => {}
                    (Ok(_), Ok(_)) => return fail("Verification failed: the copy does not match the original".into()),
                    (Err(e), _) | (_, Err(e)) => return fail(format!("Verification failed: {e}")),
                }
            }
            on_bytes(meta.size);
            return Ok((part, meta.size));
        }
    }

    let mut reader = CrcReader::new(from.open_read(from_rel)?);
    let part = to.write_part(to_rel, &mut reader, meta.mtime, on_bytes)?;
    let fail = |msg: String| -> Result<(String, u64), String> {
        let _ = to.remove_file(&part);
        Err(msg)
    };
    if reader.bytes != meta.size {
        return fail("The file changed while it was being copied; it will be copied again next time".into());
    }
    if verify {
        match checksum(to, &part) {
            Ok(c) if c == reader.crc() => {}
            Ok(_) => return fail("Verification failed: the copy does not match the original".into()),
            Err(e) => return fail(format!("Verification failed: {e}")),
        }
    }
    Ok((part, reader.bytes))
}

fn remove(store: &dyn Store, rel: &str, keep_versions: bool, stamp_dir: &str) -> Result<Transferred, String> {
    if keep_versions {
        store.rename(rel, &join_rel(stamp_dir, rel))?;
    } else {
        store.remove_file(rel)?;
    }
    store.remove_empty_parents(rel);
    Ok(Transferred { bytes: 0, version_kept: keep_versions, partial: false })
}

#[cfg(unix)]
fn copy_permissions(from: &dyn Store, to: &dyn Store, rel: &str) {
    use std::os::unix::fs::PermissionsExt;
    if let (Some(s), Some(d)) = (from.local_path(rel), to.local_path(rel)) {
        if let Ok(meta) = std::fs::metadata(s) {
            let _ = std::fs::set_permissions(d, std::fs::Permissions::from_mode(meta.permissions().mode() & 0o7777));
        }
    }
}

#[cfg(not(unix))]
fn copy_permissions(_from: &dyn Store, _to: &dyn Store, _rel: &str) {}

/// Removes dated old-version folders older than `days`
pub fn prune_versions(store: &dyn Store, versions_dir: &str, days: u32, log: &mut dyn FnMut(LogEntry)) {
    let Ok(entries) = store.list_dir(versions_dir) else { return };
    let limit = chrono::Local::now().naive_local() - chrono::Duration::days(days as i64);
    for e in entries.into_iter().filter(|e| e.is_dir) {
        if let Ok(when) = chrono::NaiveDateTime::parse_from_str(&e.name, STAMP_FORMAT) {
            if when < limit {
                let rel = join_rel(versions_dir, &e.name);
                let res = store.remove_dir_all(&rel);
                log(LogEntry {
                    path: rel,
                    action: "prune".into(),
                    size: 0,
                    ok: res.is_ok(),
                    detail: Some(match res {
                        Ok(()) => format!("old versions older than {days} days removed"),
                        Err(e) => e,
                    }),
                });
            }
        }
    }
}
