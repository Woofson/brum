//! Copying with rsync instead of Brum's built-in copier: good at sending only the changed
//! parts of large files over SSH. Preview, old versions, the per-file log, restore and
//! history work the same; rsync only does the copying. Logs in with SSH keys only.

use super::engine::{LogEntry, Mode, Progress, RunCounts, STAMP_FORMAT};
use crate::vfs::sftp::SftpClient;
use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub fn is_available() -> bool {
    Command::new("rsync").arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false)
}

/// Where rsync copies to, in rsync's own words
#[derive(Debug)]
struct Target {
    /// `/local/path/` or `user@host:/path/`
    arg: String,
    /// The `ssh` command rsync should use, for remote targets
    ssh: Option<String>,
}

/// Only plain host and user names: they end up on an ssh command line
fn safe_name(s: &str) -> bool {
    !s.is_empty() && !s.starts_with('-') && s.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
}

fn target_for(dest: &str, password: Option<&str>) -> Result<Target, String> {
    let lower = dest.to_lowercase();
    if lower.starts_with("sftp://") || lower.starts_with("ssh://") {
        if password.is_some_and(|p| !p.is_empty()) {
            return Err("rsync can only log in with an SSH key, not a password. Remove the password and set up a key, or copy with Brum instead.".into());
        }
        let params = SftpClient::parse_uri(dest, None, None)?;
        let host = params.host.trim_matches(|c| c == '[' || c == ']');
        if !safe_name(host) || !safe_name(&params.user) {
            return Err("The server or user name contains characters rsync cannot use".into());
        }
        let mut path = params.remote_path.clone();
        if let Some(rest) = path.strip_prefix("/~/") {
            path = rest.to_string(); // relative to the login's home
        } else if path == "/~" || path.is_empty() {
            path = ".".into();
        }
        let mut ssh = format!("ssh -p {} -o BatchMode=yes -o StrictHostKeyChecking=accept-new -o ConnectTimeout=15", params.port);
        // Same keys and known servers as Brum's own SFTP (which follows Brum's home folder)
        if let Some(dot_ssh) = dirs::home_dir().map(|h| h.join(".ssh")) {
            let quoted = |p: &std::path::Path| -> Option<String> {
                let s = p.to_str()?;
                (!s.contains('"')).then(|| format!("\"{s}\""))
            };
            if let Some(q) = quoted(&dot_ssh.join("known_hosts")) {
                ssh.push_str(&format!(" -o UserKnownHostsFile={q}"));
            }
            for key in ["id_ed25519", "id_ecdsa", "id_rsa"] {
                let k = dot_ssh.join(key);
                if k.is_file() {
                    if let Some(q) = quoted(&k) {
                        ssh.push_str(&format!(" -i {q}"));
                    }
                }
            }
        }
        Ok(Target { arg: format!("{}@{}:{}/", params.user, host, path.trim_end_matches('/')), ssh: Some(ssh) })
    } else if dest.contains("://") {
        Err("rsync can copy to a folder on this machine, a mounted share or an SSH/SFTP server".into())
    } else {
        Ok(Target { arg: format!("{}/", dest.trim_end_matches('/')), ssh: None })
    }
}

/// Brum's skip rules in rsync's words. Rules match case-sensitively in rsync.
fn exclusion_rules(text: &str, versions_dir: &str) -> Vec<String> {
    let mut rules = vec![format!("/{versions_dir}/"), format!("*{}", super::store::PART_SUFFIX)];
    for raw in text.split(['\n', ',', ';']) {
        let pat = raw.trim().trim_matches('/');
        if pat.is_empty() {
            continue;
        }
        // A path pattern counts from the top of the backup; a plain name matches anywhere
        rules.push(if pat.contains('/') { format!("/{pat}") } else { pat.to_string() });
    }
    rules
}

pub struct RsyncJob<'a> {
    pub source_dir: &'a str,
    pub dest_dir: &'a str,
    pub dest_password: Option<&'a str>,
    pub mode: Mode,
    pub never_remove: bool,
    pub exclusions: &'a str,
    pub versions_dir: &'a str,
    /// Files and bytes the preview expects, for the progress bar
    pub expected_files: u64,
    pub expected_bytes: u64,
}

/// Checks a job before saving it
pub fn check(dest_dir: &str, dest_password: Option<&str>, source_dir: &str, mode: Mode) -> Result<(), String> {
    if mode == Mode::TwoWay {
        return Err("rsync cannot keep two folders the same. Choose Brum's own copier for two-way sync.".into());
    }
    if source_dir.contains("://") {
        return Err("With rsync, what you back up must be a folder on this machine or a mounted share".into());
    }
    target_for(dest_dir, dest_password)?;
    if !is_available() {
        return Err("rsync is not installed on the machine running Brum".into());
    }
    Ok(())
}

pub fn run(job: &RsyncJob, progress: &dyn Fn(Progress), log: &mut dyn FnMut(LogEntry), cancel: Arc<AtomicBool>) -> Result<(RunCounts, Vec<String>), String> {
    let target = target_for(job.dest_dir, job.dest_password)?;
    let stamp = chrono::Local::now().format(STAMP_FORMAT).to_string();
    let mut cmd = Command::new("rsync");
    cmd.args(["--recursive", "--links", "--perms", "--times", "--partial", "--protect-args"]);
    cmd.args(["--itemize-changes", "--out-format=%i|%l|%n"]);
    if !job.never_remove {
        cmd.arg("--delete");
    }
    if job.mode == Mode::Versions {
        cmd.arg("--backup").arg(format!("--backup-dir={}/{}", job.versions_dir, stamp));
    }
    for rule in exclusion_rules(job.exclusions, job.versions_dir) {
        cmd.arg(format!("--exclude={rule}"));
    }
    if let Some(ssh) = &target.ssh {
        cmd.arg("-e").arg(ssh);
    }
    // "--" ends the options, so no path can be mistaken for one
    cmd.arg("--").arg(format!("{}/", job.source_dir.trim_end_matches('/'))).arg(&target.arg);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(Stdio::null());

    let mut child = cmd.spawn().map_err(|e| format!("Could not start rsync: {e}"))?;
    let stdout = child.stdout.take().ok_or("rsync gave no output")?;
    let mut stderr = child.stderr.take().ok_or("rsync gave no output")?;
    let errors = Arc::new(Mutex::new(String::new()));
    let err_sink = errors.clone();
    let err_thread = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        if let Ok(mut e) = err_sink.lock() {
            *e = s;
        }
    });

    let mut counts = RunCounts::default();
    let mut p = Progress { phase: "copying".into(), files_total: job.expected_files, bytes_total: job.expected_bytes, ..Default::default() };
    let child = Arc::new(Mutex::new(child));
    // Stops rsync when the user presses Stop
    let done = Arc::new(AtomicBool::new(false));
    let watcher = {
        let (child, cancel, done) = (child.clone(), cancel.clone(), done.clone());
        std::thread::spawn(move || {
            while !done.load(Ordering::Relaxed) {
                if cancel.load(Ordering::Relaxed) {
                    if let Ok(mut c) = child.lock() {
                        let _ = c.kill();
                    }
                    break;
                }
                std::thread::sleep(Duration::from_millis(200));
            }
        })
    };

    for line in BufReader::new(stdout).lines() {
        let Ok(line) = line else { break };
        let mut parts = line.splitn(3, '|');
        let (Some(code), Some(len), Some(name)) = (parts.next(), parts.next(), parts.next()) else { continue };
        let size: u64 = len.trim().parse().unwrap_or(0);
        let name = name.trim_end_matches('/').to_string();
        let action = if code.starts_with("*deleting") {
            "remove"
        } else if code.len() >= 2 && &code[1..2] == "f" && (code.starts_with('>') || code.starts_with('<')) {
            if code.contains('+') { "copy" } else { "update" }
        } else {
            continue; // folders, links and attribute-only changes
        };
        match action {
            "copy" => counts.copied += 1,
            "update" => counts.updated += 1,
            _ => counts.removed += 1,
        }
        let version_kept = job.mode == Mode::Versions && action != "copy";
        if version_kept {
            counts.versions_kept += 1;
        }
        if action != "remove" {
            counts.bytes += size;
            p.bytes_done += size;
        }
        p.files_done += 1;
        p.current = Some(name.clone());
        progress(p.clone());
        log(LogEntry { path: name, action: action.into(), size, ok: true, detail: version_kept.then(|| "old version kept".to_string()) });
    }

    let status = loop {
        let waited = child.lock().map_err(|_| "rsync stopped unexpectedly".to_string())?.try_wait();
        match waited {
            Ok(Some(s)) => break s,
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(format!("rsync: {e}")),
        }
    };
    done.store(true, Ordering::Relaxed);
    let _ = watcher.join();
    let _ = err_thread.join();
    let stderr_text = errors.lock().map(|e| e.clone()).unwrap_or_default();
    let warnings: Vec<String> = stderr_text.lines().filter(|l| !l.trim().is_empty()).map(|l| format!("rsync: {}", l.trim())).collect();

    if cancel.load(Ordering::Relaxed) {
        return Err("Cancelled".into());
    }
    match status.code() {
        Some(0) => Ok((counts, warnings)),
        // 23/24: some files could not be copied or vanished while copying; the rest is done
        Some(23) | Some(24) => {
            counts.failed += warnings.len().max(1) as u64;
            Ok((counts, warnings))
        }
        code => {
            let last = warnings.last().cloned().unwrap_or_else(|| format!("rsync ended with code {}", code.unwrap_or(-1)));
            Err(last.trim_start_matches("rsync: ").to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets() {
        let t = target_for("sftp://bob@nas.local:2222/backups/photos/", None).unwrap();
        assert_eq!(t.arg, "bob@nas.local:/backups/photos/");
        assert!(t.ssh.unwrap().contains("-p 2222"));
        assert_eq!(target_for("sftp://u1@box:23/~/photos", None).unwrap().arg, "u1@box:photos/");
        assert_eq!(target_for("/mnt/nas/photos", None).unwrap().arg, "/mnt/nas/photos/");
        assert!(target_for("sftp://bob@nas/x", Some("pw")).unwrap_err().contains("SSH key"));
        assert!(target_for("sftp://-oProxyCommand=x@nas/x", None).is_err());
        assert!(target_for("smb://nas/share", None).is_err());
    }

    #[test]
    fn rules() {
        let r = exclusion_rules("node_modules\n*.tmp, Photos/Raw/", ".brum-versions");
        assert_eq!(r, vec!["/.brum-versions/", "*.brum-part", "node_modules", "*.tmp", "/Photos/Raw"]);
    }
}
