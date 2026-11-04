//! Backups to and from any rclone remote (`rclone://remote/folder`), using the remotes set up
//! with `rclone config` by the user Brum runs as. Logins stay in rclone's own config.
//!
//! One rclone helper (`rclone rcd`) serves the whole run on a private local port with a random
//! password; files are copied by rclone itself, straight from place to place.

use super::store::{DirItem, FileMeta, Listing, RcloneLocation, Store};
use serde_json::{json, Value};
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub fn is_available() -> bool {
    Command::new("rclone").arg("version").stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false)
}

/// The remotes set up with `rclone config`, for choosing one in the window
pub fn list_remotes() -> Result<Vec<String>, String> {
    let out = Command::new("rclone").arg("listremotes").stdin(Stdio::null()).output().map_err(|e| format!("rclone is not installed: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).lines().map(|l| l.trim().trim_end_matches(':').to_string()).filter(|l| !l.is_empty()).collect())
}

struct Helper {
    child: Child,
    base: String,
    pass: String,
    client: reqwest::blocking::Client,
}

pub struct RcloneStore {
    /// `remote:`
    fs: String,
    /// Folder inside the remote, without slashes at the ends
    root: String,
    label: String,
    helper: Mutex<Option<Helper>>,
}

fn join(a: &str, b: &str) -> String {
    match (a.trim_matches('/'), b.trim_matches('/')) {
        ("", b) => b.to_string(),
        (a, "") => a.to_string(),
        (a, b) => format!("{a}/{b}"),
    }
}

fn parse_time(s: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(s).map(|d| d.timestamp()).unwrap_or(0)
}

/// Percent-encodes one path for a URL, keeping the slashes
fn url_path(p: &str) -> String {
    p.split('/')
        .map(|part| {
            part.bytes()
                .map(|b| match b {
                    b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
                    _ => format!("%{b:02X}"),
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("/")
}

impl RcloneStore {
    /// `rclone://remote/folder/inside`
    pub fn new(uri: &str) -> Result<Self, String> {
        let rest = uri.strip_prefix("rclone://").or_else(|| uri.strip_prefix("RCLONE://")).ok_or("An rclone place looks like rclone://remote/folder")?;
        let (remote, path) = rest.split_once('/').unwrap_or((rest, ""));
        let remote = remote.trim_end_matches(':');
        if remote.is_empty() || remote.starts_with('-') || !remote.chars().all(|c| c.is_alphanumeric() || " ._-@".contains(c)) {
            return Err("Use the name of a remote set up with rclone config, like rclone://nas/Backups".into());
        }
        Ok(Self {
            fs: format!("{remote}:"),
            root: path.trim_matches('/').to_string(),
            label: format!("rclone://{remote}/{}", path.trim_matches('/')),
            helper: Mutex::new(None),
        })
    }

    fn start_helper() -> Result<Helper, String> {
        let port = std::net::TcpListener::bind("127.0.0.1:0").and_then(|l| l.local_addr()).map(|a| a.port()).map_err(|e| e.to_string())?;
        let pass: String = {
            use rand::Rng;
            rand::thread_rng().sample_iter(&rand::distributions::Alphanumeric).take(32).map(char::from).collect()
        };
        let child = Command::new("rclone")
            .args(["rcd", "--rc-serve", "--rc-user", "brum", "--rc-pass", &pass, "--rc-addr"])
            .arg(format!("127.0.0.1:{port}"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("Could not start rclone (is it installed?): {e}"))?;
        let client = reqwest::blocking::Client::builder().timeout(None).connect_timeout(Duration::from_secs(5)).build().map_err(|e| e.to_string())?;
        let mut helper = Helper { child, base: format!("http://127.0.0.1:{port}"), pass, client };
        let started = Instant::now();
        loop {
            let ok = helper.client.post(format!("{}/core/version", helper.base)).basic_auth("brum", Some(&helper.pass)).json(&json!({})).send().map(|r| r.status().is_success()).unwrap_or(false);
            if ok {
                return Ok(helper);
            }
            if let Ok(Some(_)) = helper.child.try_wait() {
                return Err("rclone stopped right after starting".into());
            }
            if started.elapsed() > Duration::from_secs(15) {
                let _ = helper.child.kill();
                return Err("rclone did not start in time".into());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// One rclone remote-control call
    fn rc(&self, method: &str, body: Value) -> Result<Value, String> {
        let mut guard = self.helper.lock().map_err(|_| "rclone helper is unusable".to_string())?;
        if guard.is_none() {
            *guard = Some(Self::start_helper()?);
        }
        let h = guard.as_ref().expect("started above");
        let resp = h.client.post(format!("{}/{method}", h.base)).basic_auth("brum", Some(&h.pass)).json(&body).send().map_err(|e| format!("rclone: {e}"))?;
        let ok = resp.status().is_success();
        let v: Value = resp.json().unwrap_or(Value::Null);
        if ok {
            Ok(v)
        } else {
            Err(v.get("error").and_then(|e| e.as_str()).map(|e| format!("rclone: {e}")).unwrap_or_else(|| format!("rclone: {method} failed")))
        }
    }

    fn path(&self, rel: &str) -> String {
        join(&self.root, rel)
    }

    fn item(v: &Value) -> Option<(String, bool, FileMeta)> {
        Some((
            v.get("Path")?.as_str()?.to_string(),
            v.get("IsDir").and_then(|d| d.as_bool()).unwrap_or(false),
            FileMeta { size: v.get("Size").and_then(|s| s.as_i64()).unwrap_or(0).max(0) as u64, mtime: v.get("ModTime").and_then(|t| t.as_str()).map(parse_time).unwrap_or(0) },
        ))
    }

    fn stat_value(&self, rel: &str) -> Option<Value> {
        let v = self.rc("operations/stat", json!({ "fs": self.fs, "remote": self.path(rel), "opt": { "noMimeType": true } })).ok()?;
        v.get("item").filter(|i| !i.is_null()).cloned()
    }
}

impl Drop for RcloneStore {
    fn drop(&mut self) {
        if let Ok(mut g) = self.helper.lock() {
            if let Some(mut h) = g.take() {
                let _ = h.child.kill();
                let _ = h.child.wait();
            }
        }
    }
}

/// File contents served by the rclone helper
struct Served(reqwest::blocking::Response);

impl Read for Served {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl Store for RcloneStore {
    fn label(&self) -> String {
        self.label.clone()
    }

    fn root_exists(&self) -> bool {
        if self.root.is_empty() {
            return self.rc("operations/list", json!({ "fs": self.fs, "remote": "" })).is_ok();
        }
        self.stat_value("").and_then(|i| i.get("IsDir").and_then(|d| d.as_bool())).unwrap_or(false)
    }

    fn ensure_root(&self) -> Result<(), String> {
        self.rc("operations/mkdir", json!({ "fs": self.fs, "remote": self.root })).map(|_| ())
    }

    fn list(&self, base: &str, skip: &dyn Fn(&str, bool) -> bool, cancel: &AtomicBool, _warnings: &mut Vec<String>) -> Result<Listing, String> {
        let start = self.path(base);
        let mut out = Listing::new();
        if let Some(item) = self.stat_value(base) {
            if !item.get("IsDir").and_then(|d| d.as_bool()).unwrap_or(false) {
                if let Some((p, _, m)) = Self::item(&item) {
                    out.insert(p.rsplit('/').next().unwrap_or(&p).to_string(), m);
                }
                return Ok(out);
            }
        } else if !start.is_empty() {
            return Ok(out); // missing folder: nothing in it
        }
        let v = self.rc("operations/list", json!({ "fs": self.fs, "remote": start, "opt": { "recurse": true, "filesOnly": true, "noMimeType": true } }))?;
        if cancel.load(Ordering::Relaxed) {
            return Err("Cancelled".into());
        }
        let prefix = if start.is_empty() { String::new() } else { format!("{start}/") };
        for entry in v.get("list").and_then(|l| l.as_array()).cloned().unwrap_or_default() {
            let Some((path, is_dir, m)) = Self::item(&entry) else { continue };
            if is_dir {
                continue;
            }
            let rel = path.strip_prefix(&prefix).unwrap_or(&path).to_string();
            // rclone lists everything at once, so folder rules are checked against each file's folders too
            let mut acc = String::new();
            let mut skipped = false;
            for part in rel.split('/') {
                acc = join(&acc, part);
                let is_file = acc.len() == rel.len();
                if skip(&acc, !is_file) {
                    skipped = true;
                    break;
                }
            }
            if !skipped {
                out.insert(rel, m);
            }
        }
        Ok(out)
    }

    fn list_dir(&self, rel: &str) -> Result<Vec<DirItem>, String> {
        let dir = self.path(rel);
        let v = self.rc("operations/list", json!({ "fs": self.fs, "remote": dir, "opt": { "noMimeType": true } }))?;
        let mut items: Vec<DirItem> = v
            .get("list")
            .and_then(|l| l.as_array())
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter_map(|e| {
                let (path, is_dir, m) = Self::item(e)?;
                let name = path.rsplit('/').next()?.to_string();
                Some(DirItem { name, is_dir, size: if is_dir { 0 } else { m.size }, mtime: m.mtime })
            })
            .collect();
        items.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        Ok(items)
    }

    fn stat(&self, rel: &str) -> Option<FileMeta> {
        let item = self.stat_value(rel)?;
        let (_, is_dir, m) = Self::item(&item)?;
        (!is_dir).then_some(m)
    }

    fn is_dir(&self, rel: &str) -> bool {
        self.stat_value(rel).and_then(|i| i.get("IsDir").and_then(|d| d.as_bool())).unwrap_or(rel.is_empty() && self.root_exists())
    }

    fn open_read(&self, rel: &str) -> Result<Box<dyn Read + Send>, String> {
        let mut guard = self.helper.lock().map_err(|_| "rclone helper is unusable".to_string())?;
        if guard.is_none() {
            *guard = Some(Self::start_helper()?);
        }
        let h = guard.as_ref().expect("started above");
        // Served as /[remote:]/path
        let url = format!("{}/{}/{}", h.base, url_path(&format!("[{}]", self.fs)), url_path(&self.path(rel)));
        let resp = h.client.get(url).basic_auth("brum", Some(&h.pass)).send().map_err(|e| format!("rclone: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("rclone could not read {rel} ({})", resp.status()));
        }
        Ok(Box::new(Served(resp)))
    }

    fn write_part(&self, rel: &str, data: &mut dyn Read, mtime: i64, on_bytes: &mut dyn FnMut(u64) -> bool) -> Result<String, String> {
        // Only used when the other side has no rclone location; rclone_copy handles the rest
        let part = super::store::part_name(rel);
        let target = format!("{}{}", self.fs, self.path(&part));
        let mut child = Command::new("rclone")
            .args(["rcat", "--"])
            .arg(&target)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Could not start rclone: {e}"))?;
        let mut stdin = child.stdin.take().ok_or("rclone gave no input")?;
        let mut buf = vec![0u8; 256 * 1024];
        let mut written = 0u64;
        let copy_result = loop {
            match data.read(&mut buf) {
                Ok(0) => break Ok(()),
                Ok(n) => {
                    if let Err(e) = std::io::Write::write_all(&mut stdin, &buf[..n]) {
                        break Err(format!("Write failed: {e}"));
                    }
                    written += n as u64;
                    if !on_bytes(written) {
                        break Err("Cancelled".to_string());
                    }
                }
                Err(e) => break Err(format!("Read failed: {e}")),
            }
        };
        drop(stdin);
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        if let Err(e) = copy_result {
            let _ = self.remove_file(&part);
            return Err(e);
        }
        if !out.status.success() {
            return Err(format!("rclone: {}", String::from_utf8_lossy(&out.stderr).trim()));
        }
        let stamp = chrono::DateTime::from_timestamp(mtime, 0).map(|d| d.format("%Y-%m-%dT%H:%M:%S").to_string()).unwrap_or_default();
        let touched = Command::new("rclone").args(["touch", "--no-create", "--timestamp", &stamp, "--"]).arg(&target).stdin(Stdio::null()).output();
        if !touched.map(|o| o.status.success()).unwrap_or(false) {
            let _ = self.remove_file(&part);
            return Err("rclone could not set the file's time".into());
        }
        Ok(part)
    }

    fn rename(&self, from: &str, to: &str) -> Result<(), String> {
        self.rc("operations/movefile", json!({ "srcFs": self.fs, "srcRemote": self.path(from), "dstFs": self.fs, "dstRemote": self.path(to) })).map(|_| ())
    }

    fn remove_file(&self, rel: &str) -> Result<(), String> {
        self.rc("operations/deletefile", json!({ "fs": self.fs, "remote": self.path(rel) })).map(|_| ())
    }

    fn remove_dir_all(&self, rel: &str) -> Result<(), String> {
        self.rc("operations/purge", json!({ "fs": self.fs, "remote": self.path(rel) })).map(|_| ())
    }

    fn remove_empty_parents(&self, rel: &str) {
        let mut cur = rel.to_string();
        while let Some((dir, _)) = cur.rsplit_once('/') {
            if self.rc("operations/rmdir", json!({ "fs": self.fs, "remote": self.path(dir) })).is_err() {
                break;
            }
            cur = dir.to_string();
        }
    }

    fn local_path(&self, _rel: &str) -> Option<PathBuf> {
        None
    }

    fn rclone_location(&self, rel: &str) -> Option<RcloneLocation> {
        Some((self.fs.clone(), self.path(rel)))
    }

    fn rclone_copy(&self, src: &RcloneLocation, dst: &RcloneLocation) -> Option<Result<(), String>> {
        Some(self.rc("operations/copyfile", json!({ "srcFs": src.0, "srcRemote": src.1, "dstFs": dst.0, "dstRemote": dst.1 })).map(|_| ()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses() {
        let s = RcloneStore::new("rclone://nas/Backups/Photos/").unwrap();
        assert_eq!((s.fs.as_str(), s.root.as_str()), ("nas:", "Backups/Photos"));
        assert_eq!(s.path("2024/a.jpg"), "Backups/Photos/2024/a.jpg");
        assert_eq!(RcloneStore::new("rclone://box").unwrap().root, "");
        assert!(RcloneStore::new("rclone:///x").is_err());
        assert!(RcloneStore::new("rclone://-x/y").is_err());
        assert!(RcloneStore::new("rclone://a;b/y").is_err());
        assert_eq!(url_path("[nas:]"), "%5Bnas%3A%5D");
        assert_eq!(url_path("a b/c#d"), "a%20b/c%23d");
    }
}
