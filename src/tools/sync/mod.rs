//! Backup & Sync: saved backups that copy, keep old versions, or keep two folders the same,
//! with a preview, schedules, a per-file log of every run, and restore.

pub mod engine;
pub mod exclude;
pub mod fleet_store;
pub mod git_repo;
pub mod manager;
pub mod proton_store;
pub mod rclone_store;
pub mod rsync;
pub mod sftp_store;
pub mod store;

pub use manager::{BackupManager, BackupProfile, PreviewResult, ProfileView, RestorePoint, RestorePreview, RestoreRequest, RunFile, RunRecord};

#[cfg(test)]
mod tests {
    use super::engine::{self, Action, JobSpec, Mode};
    use super::exclude::Exclusions;
    use super::manager::{next_due, BackupManager, BackupProfile, RestoreRequest};
    use super::store::{Listing, LocalStore};
    use crate::tools::tasks::TaskManager;
    use rusqlite::Connection;
    use std::fs;
    use std::path::Path;
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex};

    fn write(p: &Path, rel: &str, body: &str, mtime: i64) {
        let f = p.join(rel);
        fs::create_dir_all(f.parent().unwrap()).unwrap();
        fs::write(&f, body).unwrap();
        filetime::set_file_mtime(&f, filetime::FileTime::from_unix_time(mtime, 0)).unwrap();
    }

    fn spec(src: &Path, dst: &Path, mode: Mode) -> JobSpec {
        JobSpec {
            source: Arc::new(LocalStore::new(src)),
            dest: Arc::new(LocalStore::new(dst)),
            mode,
            never_remove: false,
            verify: true,
            partial_updates: true,
            exclusions: Exclusions::default(),
            versions_dir: engine::VERSIONS_DIR.into(),
            retention_days: 30,
        }
    }

    fn run(s: &JobSpec, state: Option<&Listing>) -> (engine::Plan, engine::RunOutcome, Vec<engine::LogEntry>) {
        let cancel = AtomicBool::new(false);
        let plan = engine::plan(s, state, &cancel).unwrap();
        let mut log = Vec::new();
        let out = engine::execute(s, &plan, state, &|_| {}, &mut |e| log.push(e), &cancel).unwrap();
        (plan, out, log)
    }

    fn versions_files(dst: &Path) -> Vec<String> {
        let root = dst.join(engine::VERSIONS_DIR);
        walkdir::WalkDir::new(&root)
            .into_iter()
            .flatten()
            .filter(|e| e.file_type().is_file())
            .map(|e| e.path().strip_prefix(&root).unwrap().components().skip(1).map(|c| c.as_os_str().to_string_lossy().to_string()).collect::<Vec<_>>().join("/"))
            .collect()
    }

    #[test]
    fn copy_mode_makes_an_exact_copy() {
        let t = tempfile::tempdir().unwrap();
        let (src, dst) = (t.path().join("src"), t.path().join("dst"));
        write(&src, "a.txt", "A", 1_700_000_000);
        write(&src, "sub/b.txt", "B", 1_700_000_000);
        write(&dst, "old.txt", "gone", 1_700_000_000);
        let s = spec(&src, &dst, Mode::Copy);
        let (plan, out, log) = run(&s, None);
        assert_eq!((plan.summary.copy, plan.summary.remove), (2, 1));
        assert_eq!((out.counts.copied, out.counts.removed, out.counts.failed), (2, 1, 0));
        assert_eq!(fs::read_to_string(dst.join("sub/b.txt")).unwrap(), "B");
        assert!(!dst.join("old.txt").exists());
        assert_eq!(log.len(), 3);
        // Second run: nothing to do
        let (plan2, _, _) = run(&s, None);
        assert!(plan2.items.is_empty());
        assert_eq!(plan2.summary.unchanged, 2);
    }

    #[test]
    fn versions_mode_keeps_replaced_and_removed_files() {
        let t = tempfile::tempdir().unwrap();
        let (src, dst) = (t.path().join("src"), t.path().join("dst"));
        write(&src, "doc.txt", "v1", 1_700_000_000);
        write(&src, "bye.txt", "bye", 1_700_000_000);
        let s = spec(&src, &dst, Mode::Versions);
        run(&s, None);
        write(&src, "doc.txt", "v2", 1_700_000_100);
        fs::remove_file(src.join("bye.txt")).unwrap();
        let (_, out, _) = run(&s, None);
        assert_eq!((out.counts.updated, out.counts.removed, out.counts.versions_kept), (1, 1, 2));
        assert_eq!(fs::read_to_string(dst.join("doc.txt")).unwrap(), "v2");
        assert!(!dst.join("bye.txt").exists());
        let mut kept = versions_files(&dst);
        kept.sort();
        assert_eq!(kept, vec!["bye.txt", "doc.txt"]);
        // The versions folder itself is never backed up or removed
        let (plan, _, _) = run(&s, None);
        assert!(plan.items.is_empty(), "{:?}", plan.items);
    }

    #[test]
    fn never_remove_keeps_everything_in_the_backup() {
        let t = tempfile::tempdir().unwrap();
        let (src, dst) = (t.path().join("src"), t.path().join("dst"));
        write(&src, "a.txt", "A", 1_700_000_000);
        write(&dst, "extra.txt", "keep me", 1_700_000_000);
        let mut s = spec(&src, &dst, Mode::Copy);
        s.never_remove = true;
        let (plan, _, _) = run(&s, None);
        assert_eq!(plan.summary.remove, 0);
        assert!(dst.join("extra.txt").exists());
    }

    #[test]
    fn more_than_ten_thousand_files_are_all_backed_up() {
        let t = tempfile::tempdir().unwrap();
        let (src, dst) = (t.path().join("src"), t.path().join("dst"));
        for i in 0..10_050 {
            let f = src.join(format!("d{}/f{i}.txt", i % 50));
            fs::create_dir_all(f.parent().unwrap()).unwrap();
            fs::write(f, "x").unwrap();
        }
        let mut s = spec(&src, &dst, Mode::Copy);
        s.verify = false;
        let (plan, out, _) = run(&s, None);
        assert_eq!(plan.summary.copy, 10_050);
        assert_eq!(out.counts.copied, 10_050);
    }

    #[test]
    fn a_folder_named_archive_is_backed_up() {
        let t = tempfile::tempdir().unwrap();
        let (src, dst) = (t.path().join("src"), t.path().join("dst"));
        write(&src, "_archive/2019/tax.pdf", "pdf", 1_700_000_000);
        let (_, out, _) = run(&spec(&src, &dst, Mode::Versions), None);
        assert_eq!(out.counts.copied, 1);
        assert!(dst.join("_archive/2019/tax.pdf").exists());
    }

    #[test]
    fn exclusions_skip_exact_names_on_both_sides() {
        let t = tempfile::tempdir().unwrap();
        let (src, dst) = (t.path().join("src"), t.path().join("dst"));
        write(&src, "p/.git/HEAD", "x", 1_700_000_000);
        write(&src, "p/.gitignore", "x", 1_700_000_000);
        write(&dst, "p/.git/OLD", "x", 1_700_000_000);
        let mut s = spec(&src, &dst, Mode::Copy);
        s.exclusions = Exclusions::parse(".git");
        let (plan, _, _) = run(&s, None);
        let paths: Vec<_> = plan.items.iter().map(|i| (i.path.as_str(), i.action)).collect();
        assert_eq!(paths, vec![("p/.gitignore", Action::Copy)]);
        assert!(dst.join("p/.git/OLD").exists(), "excluded files in the backup are left alone");
    }

    #[test]
    fn empty_source_stops_instead_of_wiping_the_backup() {
        let t = tempfile::tempdir().unwrap();
        let (src, dst) = (t.path().join("src"), t.path().join("dst"));
        fs::create_dir_all(&src).unwrap();
        write(&dst, "precious.txt", "x", 1_700_000_000);
        let err = engine::plan(&spec(&src, &dst, Mode::Copy), None, &AtomicBool::new(false)).err().unwrap();
        assert!(err.contains("empty"), "{err}");
        let missing = engine::plan(&spec(&t.path().join("nope"), &dst, Mode::Copy), None, &AtomicBool::new(false)).err().unwrap();
        assert!(missing.contains("missing"), "{missing}");
        assert!(dst.join("precious.txt").exists());
    }

    #[test]
    fn two_way_sync_remembers_deletions() {
        let t = tempfile::tempdir().unwrap();
        let (a, b) = (t.path().join("a"), t.path().join("b"));
        write(&a, "shared.txt", "1", 1_700_000_000);
        write(&a, "doomed.txt", "x", 1_700_000_000);
        write(&b, "from_b.txt", "b", 1_700_000_000);
        let s = spec(&a, &b, Mode::TwoWay);
        let (_, out, _) = run(&s, None);
        let state = out.new_state.unwrap();
        assert_eq!(state.len(), 3);
        assert!(a.join("from_b.txt").exists() && b.join("doomed.txt").exists());

        // Delete on one side, change on the other
        fs::remove_file(b.join("doomed.txt")).unwrap();
        write(&b, "shared.txt", "2", 1_700_000_500);
        let (plan, out, _) = run(&s, Some(&state));
        let acts: Vec<_> = plan.items.iter().map(|i| (i.path.as_str(), i.action)).collect();
        assert_eq!(acts, vec![("shared.txt", Action::UpdateBack), ("doomed.txt", Action::RemoveBack)]);
        assert!(!a.join("doomed.txt").exists(), "deleted file must not come back");
        assert_eq!(fs::read_to_string(a.join("shared.txt")).unwrap(), "2");
        // Removed and replaced files are kept as old versions on that side
        let mut kept = versions_files(&a);
        kept.sort();
        assert_eq!(kept, vec!["doomed.txt", "shared.txt"]);
        let state = out.new_state.unwrap();
        let (plan, _, _) = run(&s, Some(&state));
        assert!(plan.items.is_empty(), "{:?}", plan.items);
    }

    #[test]
    fn two_way_conflict_keeps_newer_and_saves_the_other() {
        let t = tempfile::tempdir().unwrap();
        let (a, b) = (t.path().join("a"), t.path().join("b"));
        write(&a, "f.txt", "base", 1_700_000_000);
        let s = spec(&a, &b, Mode::TwoWay);
        let state = run(&s, None).1.new_state.unwrap();
        write(&a, "f.txt", "from a", 1_700_000_100);
        write(&b, "f.txt", "from b, newer", 1_700_000_200);
        let (plan, _, log) = run(&s, Some(&state));
        assert_eq!(plan.summary.conflicts, 1);
        assert_eq!(fs::read_to_string(a.join("f.txt")).unwrap(), "from b, newer");
        assert_eq!(versions_files(&a), vec!["f.txt"]);
        assert!(log[0].detail.as_deref().unwrap().contains("both sides"));
    }

    #[test]
    fn two_way_stops_when_one_side_is_suddenly_empty() {
        let t = tempfile::tempdir().unwrap();
        let (a, b) = (t.path().join("a"), t.path().join("b"));
        write(&a, "f.txt", "x", 1_700_000_000);
        let s = spec(&a, &b, Mode::TwoWay);
        let state = run(&s, None).1.new_state.unwrap();
        fs::remove_file(b.join("f.txt")).unwrap();
        let err = engine::plan(&s, Some(&state), &AtomicBool::new(false)).err().unwrap();
        assert!(err.contains("connected"), "{err}");
        assert!(a.join("f.txt").exists());
    }

    #[test]
    fn old_versions_are_pruned_after_the_retention_days() {
        let t = tempfile::tempdir().unwrap();
        let (src, dst) = (t.path().join("src"), t.path().join("dst"));
        write(&src, "a.txt", "A", 1_700_000_000);
        write(&dst, ".brum-versions/2001-01-01_000000/old.txt", "ancient", 1_000_000_000);
        let recent = chrono::Local::now().format(engine::STAMP_FORMAT).to_string();
        write(&dst, &format!(".brum-versions/{recent}/new.txt"), "recent", 1_700_000_000);
        let (_, _, log) = run(&spec(&src, &dst, Mode::Versions), None);
        assert!(!dst.join(".brum-versions/2001-01-01_000000").exists());
        assert!(dst.join(format!(".brum-versions/{recent}/new.txt")).exists());
        assert!(log.iter().any(|e| e.action == "prune"));
    }

    #[test]
    fn modes_from_the_old_tool() {
        assert_eq!(Mode::parse("echo"), (Mode::Copy, false));
        assert_eq!(Mode::parse("contribute"), (Mode::Copy, true));
        assert_eq!(Mode::parse("subscribe"), (Mode::Versions, false));
        assert_eq!(Mode::parse("synchronize"), (Mode::TwoWay, false));
    }

    fn manager() -> BackupManager {
        BackupManager::new(Arc::new(Mutex::new(Connection::open_in_memory().unwrap()))).unwrap()
    }

    fn profile(src: &Path, dst: &Path) -> BackupProfile {
        serde_json::from_value(serde_json::json!({
            "name": "Photos",
            "source_dir": src.to_string_lossy(),
            "dest_dir": dst.to_string_lossy(),
        }))
        .unwrap()
    }

    #[test]
    fn old_profiles_are_carried_over() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE backup_profiles (id TEXT PRIMARY KEY, name TEXT NOT NULL, source_dir TEXT NOT NULL, dest_dir TEXT NOT NULL, profile_mode TEXT NOT NULL,
               block_delta INTEGER DEFAULT 1, verify_checksum INTEGER DEFAULT 1, archive_dir TEXT, retention_days INTEGER DEFAULT 30,
               schedule_type TEXT DEFAULT 'manual', schedule_interval_mins INTEGER DEFAULT 60, schedule_time TEXT, webhook_url TEXT,
               enabled INTEGER DEFAULT 1, last_run INTEGER, last_status TEXT, last_result TEXT, created_at INTEGER NOT NULL, exclusions TEXT, owner TEXT);
             INSERT INTO backup_profiles (id, name, source_dir, dest_dir, profile_mode, archive_dir, schedule_type, created_at) VALUES
               ('c', 'C', '/a', '/b', 'contribute', '_archive', 'realtime', 1),
               ('r', 'R', '/left', '/right', 'mirror_right_to_left', NULL, 'manual', 1);
             CREATE TABLE backup_history (id TEXT PRIMARY KEY, profile_id TEXT NOT NULL, profile_name TEXT NOT NULL, started_at INTEGER NOT NULL, finished_at INTEGER,
               status TEXT NOT NULL, files_copied INTEGER DEFAULT 0, files_updated INTEGER DEFAULT 0, files_deleted INTEGER DEFAULT 0, files_archived INTEGER DEFAULT 0,
               bytes_transferred INTEGER DEFAULT 0, duration_ms INTEGER DEFAULT 0, error_message TEXT);
             INSERT INTO backup_history (id, profile_id, profile_name, started_at, status, files_copied) VALUES ('h1', 'c', 'C', 5, 'success', 7);",
        )
        .unwrap();
        let m = BackupManager::new(Arc::new(Mutex::new(conn))).unwrap();
        let c = m.get_profile("c").unwrap();
        assert_eq!((c.profile_mode.as_str(), c.never_remove, c.schedule_type.as_str(), c.schedule_interval_mins), ("copy", true, "interval", 5));
        assert_eq!(c.versions_dir(), "_archive", "old backups keep their old-versions folder");
        let r = m.get_profile("r").unwrap();
        assert_eq!((r.profile_mode.as_str(), r.source_dir.as_str(), r.dest_dir.as_str()), ("copy", "/right", "/left"));
        assert_eq!(r.versions_dir(), ".brum-versions");
        let runs = m.list_runs(Some("c"), 10);
        assert_eq!((runs.len(), runs[0].status.as_str(), runs[0].counts.copied), (1, "ok", 7));
    }

    #[test]
    fn save_checks_the_folders() {
        let m = manager();
        let t = tempfile::tempdir().unwrap();
        let p = profile(&t.path().join("x"), &t.path().join("x/backup"));
        assert!(m.save_profile(p).unwrap_err().contains("inside"));
        let p = profile(&t.path().join("x"), &t.path().join("x"));
        assert!(m.save_profile(p).unwrap_err().contains("same"));
        let mut p = profile(&t.path().join("x"), &t.path().join("y"));
        p.schedule_type = "daily".into();
        assert!(m.save_profile(p.clone()).unwrap_err().contains("time"));
        p.schedule_time = Some("02:00".into());
        let saved = m.save_profile(p).unwrap();
        assert!(!saved.id.is_empty() && saved.created_at > 0);
        assert_eq!(saved.profile_mode, "versions");
    }

    #[test]
    fn schedule_times() {
        let mut p = profile(Path::new("/a"), Path::new("/b"));
        p.created_at = 1_000;
        assert_eq!(next_due(&p), None);
        p.schedule_type = "interval".into();
        p.schedule_interval_mins = 5;
        assert_eq!(next_due(&p), Some(1_000), "a new interval backup runs right away");
        p.last_started = Some(10_000);
        assert_eq!(next_due(&p), Some(10_300), "counted from when the last run started");
        p.schedule_type = "daily".into();
        p.schedule_time = Some("02:00".into());
        let due = next_due(&p).unwrap();
        assert!(due > 10_000 && due - 10_000 <= 86_400);
    }

    async fn wait_idle(m: &BackupManager, id: &str) {
        for _ in 0..500 {
            if !m.is_running(id) {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("run did not finish");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn backup_run_is_logged_and_never_started_twice() {
        let m = manager();
        let tasks = Arc::new(TaskManager::new());
        let t = tempfile::tempdir().unwrap();
        let (src, dst) = (t.path().join("src"), t.path().join("dst"));
        for i in 0..300 {
            write(&src, &format!("f{i}.bin"), &"x".repeat(20_000), 1_700_000_000);
        }
        let p = m.save_profile(profile(&src, &dst)).unwrap();
        let run_id = m.start_backup(&p.id, "manual", tasks.clone()).unwrap();
        assert!(m.start_backup(&p.id, "manual", tasks.clone()).unwrap_err().contains("already running"));
        wait_idle(&m, &p.id).await;
        let rec = m.get_run(&run_id).unwrap();
        assert_eq!((rec.status.as_str(), rec.counts.copied, rec.counts.bytes), ("ok", 300, 300 * 20_000));
        assert_eq!(rec.message.as_deref(), Some("Copied 300 files, 5.7 MB"));
        let (files, total) = m.run_files(&run_id, 0, 10, false);
        assert_eq!((files.len(), total), (10, 300));
        assert!(m.run_log_text(&run_id).unwrap().contains("copied       f0.bin"));

        // Next run has nothing to do
        let run2 = m.start_backup(&p.id, "manual", tasks.clone()).unwrap();
        wait_idle(&m, &p.id).await;
        let rec2 = m.get_run(&run2).unwrap();
        assert_eq!(rec2.status, "nothing");
        assert!(rec2.message.unwrap().contains("300 files checked"));
        let view = m.view(m.get_profile(&p.id).unwrap());
        assert!(view.last_success_at.is_some() && view.profile.last_started.is_some());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn several_sources_each_get_their_own_folder() {
        let m = manager();
        let tasks = Arc::new(TaskManager::new());
        let t = tempfile::tempdir().unwrap();
        let (docs, pics, pics2, dst) = (t.path().join("Documents"), t.path().join("Pictures"), t.path().join("old/Pictures"), t.path().join("backup"));
        write(&docs, "report.pdf", "R", 1_700_000_000);
        write(&pics, "a.jpg", "A", 1_700_000_000);
        write(&pics2, "b.jpg", "B", 1_700_000_000);
        let mut p = profile(&docs, &dst);
        p.sources = vec![docs.to_string_lossy().into(), pics.to_string_lossy().into(), pics2.to_string_lossy().into()];
        let p = m.save_profile(p).unwrap();
        assert_eq!(p.source_dir, docs.to_string_lossy());
        m.start_backup(&p.id, "manual", tasks.clone()).unwrap();
        wait_idle(&m, &p.id).await;
        assert_eq!(fs::read_to_string(dst.join("Documents/report.pdf")).unwrap(), "R");
        assert_eq!(fs::read_to_string(dst.join("Pictures/a.jpg")).unwrap(), "A");
        assert_eq!(fs::read_to_string(dst.join("Pictures (2)/b.jpg")).unwrap(), "B");
        let top: Vec<_> = m.browse(&p, "latest", "").unwrap().into_iter().map(|i| i.name).collect();
        assert_eq!(top, vec!["Documents", "Pictures", "Pictures (2)"]);

        // One source suddenly empty: stop instead of emptying its part of the backup
        fs::remove_file(pics.join("a.jpg")).unwrap();
        let run = m.start_backup(&p.id, "manual", tasks.clone()).unwrap();
        wait_idle(&m, &p.id).await;
        let rec = m.get_run(&run).unwrap();
        assert_eq!(rec.status, "failed");
        assert!(rec.message.unwrap().contains("\"Pictures\" is missing or empty"));
        assert!(dst.join("Pictures/a.jpg").exists());

        // Restore into the original places
        let req: RestoreRequest = serde_json::from_value(serde_json::json!({ "target": "original" })).unwrap();
        let pre = m.restore(&p, &req, &p.source_dir, tasks.clone()).unwrap();
        assert!(!pre.needs_confirmation, "nothing exists there to overwrite");
        wait_idle(&m, &p.id).await;
        assert_eq!(fs::read_to_string(pics.join("a.jpg")).unwrap(), "A");
        assert!(BackupManager::default_restore_folder(&p).contains("Photos (restored "));

        // Several sources only for one-way backups, never nested
        let mut two = p.clone();
        two.profile_mode = "two_way".into();
        assert!(m.save_profile(two).unwrap_err().contains("one folder at a time"));
        let mut nested = p.clone();
        nested.sources = vec![t.path().join("old").to_string_lossy().into(), pics2.to_string_lossy().into()];
        assert!(m.save_profile(nested).unwrap_err().contains("inside the other"));
    }

    #[test]
    fn mirrored_paths() {
        use super::store::mirror_name;
        assert_eq!(mirror_name("/home/bolt/.bashrc", Some("/home/bolt")), "home/.bashrc");
        assert_eq!(mirror_name("/home/bolt/.config/nvim/", Some("/home/bolt/")), "home/.config/nvim");
        assert_eq!(mirror_name("/etc/hosts", Some("/home/bolt")), "root/etc/hosts");
        assert_eq!(mirror_name("/home/bolter/x", Some("/home/bolt")), "root/home/bolter/x");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn git_collection_saves_versions_restores_and_pushes() {
        if !super::git_repo::is_available() {
            return;
        }
        let m = manager();
        let tasks = Arc::new(TaskManager::new());
        let t = tempfile::tempdir().unwrap();
        let home = dirs::home_dir().unwrap();
        // Tracked: one single file and one folder (outside home, so they land under root/)
        let rc = t.path().join("etc/app.conf");
        let proj = t.path().join("projects/board");
        write(t.path(), "etc/app.conf", "v1", 1_700_000_000);
        write(&proj, "board.kicad_sch", "schematic", 1_700_000_000);
        write(&proj, "notes.md", "notes", 1_700_000_000);
        let repo = t.path().join("collections/workstation");
        let bare = t.path().join("remote.git");
        std::process::Command::new("git").args(["init", "--bare", "--quiet"]).arg(&bare).status().unwrap();
        let mut p = profile(&rc, &repo);
        p.name = "Workstation".into();
        p.profile_mode = "git".into();
        p.sources = vec![rc.to_string_lossy().into(), proj.to_string_lossy().into()];
        p.git_remote = Some(bare.to_string_lossy().into());
        p.git_push = true;
        let p = m.save_profile(p).unwrap();
        assert_eq!(p.profile_mode, "git");

        let r1 = m.start_backup(&p.id, "manual", tasks.clone()).unwrap();
        wait_idle(&m, &p.id).await;
        let rec = m.get_run(&r1).unwrap();
        assert_eq!(rec.status, "ok", "{:?}", rec.message);
        assert!(rec.message.as_deref().unwrap().contains("saved as version"), "{:?}", rec.message);
        assert!(rec.message.as_deref().unwrap().contains("pushed"), "{:?}", rec.message);
        let mirror = |path: &Path| super::store::mirror_name(&path.to_string_lossy(), Some(&home.to_string_lossy()));
        assert_eq!(fs::read_to_string(repo.join(mirror(&rc))).unwrap(), "v1");
        assert!(repo.join(mirror(&proj)).join("board.kicad_sch").exists());
        assert!(fs::read_to_string(repo.join("brum-collection.toml")).unwrap().contains("in_repository"));

        // Nothing changed: no new version
        let r2 = m.start_backup(&p.id, "manual", tasks.clone()).unwrap();
        wait_idle(&m, &p.id).await;
        assert_eq!(m.get_run(&r2).unwrap().status, "nothing");
        assert_eq!(super::git_repo::versions(&repo, 10).len(), 1);

        // A change: a second version, the repository copy follows
        write(t.path(), "etc/app.conf", "v2", 1_700_000_500);
        let r3 = m.start_backup(&p.id, "manual", tasks.clone()).unwrap();
        wait_idle(&m, &p.id).await;
        assert_eq!(m.get_run(&r3).unwrap().status, "ok");
        let versions = super::git_repo::versions(&repo, 10);
        assert_eq!(versions.len(), 2);
        assert_eq!(fs::read_to_string(repo.join(mirror(&rc))).unwrap(), "v2");
        // The remote has both versions
        let remote_log = std::process::Command::new("git").arg("-C").arg(&bare).args(["log", "--oneline", "main"]).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&remote_log.stdout).lines().count(), 2);

        // Restore points are the saved versions; the first one still has v1
        let points = m.restore_points(&p).unwrap();
        assert_eq!(points.len(), 3, "{points:?}");
        let first = points.last().unwrap().id.clone();
        let top: Vec<_> = m.browse(&p, &first, "").unwrap().into_iter().map(|i| i.name).collect();
        assert_eq!(top, vec!["root"], "manifest and .git are hidden");
        let req: RestoreRequest = serde_json::from_value(serde_json::json!({ "point": first })).unwrap();
        let sep = BackupManager::default_restore_folder(&p);
        assert!(sep.contains("Workstation (restored "));
        let pre = m.restore(&p, &req, &sep, tasks.clone()).unwrap();
        wait_idle(&m, &p.id).await;
        assert_eq!(pre.files, 3);
        assert_eq!(fs::read_to_string(Path::new(&sep).join(mirror(&rc))).unwrap(), "v1");

        // Back into the original places: asks first, then puts v1 back
        let req: RestoreRequest = serde_json::from_value(serde_json::json!({ "point": first, "target": "original" })).unwrap();
        let pre = m.restore(&p, &req, &p.source_dir, tasks.clone()).unwrap();
        assert!(pre.needs_confirmation && pre.overwrites == 1, "{pre:?}");
        let req: RestoreRequest = serde_json::from_value(serde_json::json!({ "point": first, "target": "original", "confirm_overwrite": true })).unwrap();
        m.restore(&p, &req, &p.source_dir, tasks.clone()).unwrap();
        wait_idle(&m, &p.id).await;
        assert_eq!(fs::read_to_string(&rc).unwrap(), "v1");

        // Push on its own
        let push = m.start_push(&p.id).unwrap();
        wait_idle(&m, &p.id).await;
        let rec = m.get_run(&push).unwrap();
        assert_eq!((rec.kind.as_str(), rec.status.as_str()), ("push", "ok"), "{:?}", rec.message);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn restore_to_a_separate_folder_and_to_the_original_with_confirmation() {
        let m = manager();
        let tasks = Arc::new(TaskManager::new());
        let t = tempfile::tempdir().unwrap();
        let (src, dst) = (t.path().join("Photos"), t.path().join("backup"));
        write(&src, "2024/a.jpg", "A1", 1_700_000_000);
        write(&src, "2024/b.jpg", "B1", 1_700_000_000);
        let p = m.save_profile(profile(&src, &dst)).unwrap();
        m.start_backup(&p.id, "manual", tasks.clone()).unwrap();
        wait_idle(&m, &p.id).await;
        // Change a file and back up again, so an old version exists
        write(&src, "2024/a.jpg", "A2", 1_700_000_100);
        m.start_backup(&p.id, "manual", tasks.clone()).unwrap();
        wait_idle(&m, &p.id).await;

        let points = m.restore_points(&p).unwrap();
        assert_eq!(points[0].id, "latest");
        assert_eq!(points.len(), 2, "{points:?}");
        let top = m.browse(&p, "latest", "").unwrap();
        assert_eq!(top.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), vec!["2024"], "versions folder is hidden");
        assert!(m.browse(&p, "latest", "../..").is_err());

        // Separate folder
        let sep = BackupManager::default_restore_folder(&p);
        assert!(sep.contains("Photos (restored "));
        let req: RestoreRequest = serde_json::from_value(serde_json::json!({ "items": ["2024/a.jpg"] })).unwrap();
        let pre = m.restore(&p, &req, &sep, tasks.clone()).unwrap();
        assert_eq!((pre.files, pre.overwrites, pre.needs_confirmation), (1, 0, false));
        wait_idle(&m, &p.id).await;
        assert_eq!(fs::read_to_string(Path::new(&sep).join("2024/a.jpg")).unwrap(), "A2");

        // The older version, back into the original folder: needs confirmation
        let old = points[1].id.clone();
        let req: RestoreRequest = serde_json::from_value(serde_json::json!({ "point": old, "target": "original" })).unwrap();
        let pre = m.restore(&p, &req, &src.to_string_lossy(), tasks.clone()).unwrap();
        assert_eq!((pre.files, pre.overwrites, pre.needs_confirmation, pre.run_id.is_none()), (1, 1, true, true));
        assert_eq!(fs::read_to_string(src.join("2024/a.jpg")).unwrap(), "A2", "nothing changed without confirmation");
        let req: RestoreRequest = serde_json::from_value(serde_json::json!({ "point": old, "target": "original", "confirm_overwrite": true })).unwrap();
        let pre = m.restore(&p, &req, &src.to_string_lossy(), tasks.clone()).unwrap();
        wait_idle(&m, &p.id).await;
        assert_eq!(fs::read_to_string(src.join("2024/a.jpg")).unwrap(), "A1");
        let rec = m.get_run(&pre.run_id.unwrap()).unwrap();
        assert_eq!((rec.kind.as_str(), rec.status.as_str(), rec.counts.copied), ("restore", "ok", 1));
        let (files, _) = m.run_files(&rec.id, 0, 10, false);
        assert_eq!(files[0].detail.as_deref(), Some("replaced the existing file"));
    }
}
