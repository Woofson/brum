//! Backups kept on another Brum in the Fleet (`fleet://node/folder`). The receiving Brum
//! writes to its own disks through its backup-target addresses, as the user its token
//! belongs to; files are streamed and keep their times.

use super::store::{DirItem, FileMeta, Listing, Store};
use serde_json::{json, Value};
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

pub struct FleetStore {
    endpoint: String,
    token: Option<String>,
    /// Absolute folder on the receiving machine
    root: String,
    label: String,
    client: std::sync::OnceLock<reqwest::blocking::Client>,
}

/// Splits `fleet://node/abs/folder` into the node id and the folder
pub fn parse(uri: &str) -> Result<(String, String), String> {
    let rest = uri.strip_prefix("fleet://").ok_or("A Fleet place looks like fleet://node/folder")?;
    let (node, path) = rest.split_once('/').unwrap_or((rest, ""));
    if node.is_empty() {
        return Err("Choose a Fleet machine, like fleet://nas/Backups".into());
    }
    let path = format!("/{}", path.trim_matches('/'));
    Ok((node.to_string(), path))
}

impl FleetStore {
    pub fn new(uri: &str, endpoint: &str, token: Option<&str>) -> Result<Self, String> {
        let (node, root) = parse(uri)?;
        let endpoint = endpoint.trim().trim_end_matches('/').to_string();
        if !(endpoint.starts_with("http://") || endpoint.starts_with("https://")) {
            return Err(format!("The address of Fleet machine '{node}' is unknown. Edit and save the backup again."));
        }
        Ok(Self {
            endpoint,
            token: token.map(str::trim).filter(|t| !t.is_empty()).map(String::from),
            root,
            label: uri.trim_end_matches('/').to_string(),
            client: std::sync::OnceLock::new(),
        })
    }

    fn client(&self) -> &reqwest::blocking::Client {
        self.client.get_or_init(|| reqwest::blocking::Client::builder().timeout(None).connect_timeout(Duration::from_secs(10)).build().expect("HTTP client"))
    }

    fn path(&self, rel: &str) -> String {
        let rel = rel.trim_matches('/');
        match (self.root.as_str(), rel) {
            (r, "") => r.to_string(),
            ("/", r) => format!("/{r}"),
            (root, r) => format!("{root}/{r}"),
        }
    }

    fn req(&self, method: reqwest::Method, what: &str) -> reqwest::blocking::RequestBuilder {
        let mut b = self.client().request(method, format!("{}/api/backup-target/{what}", self.endpoint));
        if let Some(t) = &self.token {
            b = b.bearer_auth(t);
        }
        b
    }

    fn check(resp: reqwest::blocking::Response) -> Result<reqwest::blocking::Response, String> {
        if resp.status().is_success() {
            return Ok(resp);
        }
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        let msg = serde_json::from_str::<Value>(&text).ok().and_then(|v| v.get("error").and_then(|e| e.as_str()).map(String::from)).unwrap_or(text);
        Err(match status.as_u16() {
            401 => "The Fleet machine refused the login. Check the node's token.".to_string(),
            404 if msg.is_empty() => "The Fleet machine does not accept backups yet. Update Brum there.".to_string(),
            _ => format!("Fleet machine: {}", if msg.is_empty() { status.to_string() } else { msg }),
        })
    }

    fn get_json(&self, what: &str, query: &[(&str, &str)]) -> Result<Value, String> {
        let resp = self.req(reqwest::Method::GET, what).query(query).send().map_err(|e| format!("Cannot reach the Fleet machine: {e}"))?;
        Self::check(resp)?.json().map_err(|e| e.to_string())
    }

    fn op(&self, body: Value) -> Result<(), String> {
        let resp = self.req(reqwest::Method::POST, "op").json(&body).send().map_err(|e| format!("Cannot reach the Fleet machine: {e}"))?;
        Self::check(resp).map(|_| ())
    }

    fn stat_value(&self, rel: &str) -> Option<Value> {
        let v = self.get_json("stat", &[("path", &self.path(rel))]).ok()?;
        v.get("kind").and_then(|k| k.as_str()).is_some().then_some(v)
    }

    fn entries(v: &Value) -> Vec<(String, bool, FileMeta)> {
        v.get("entries")
            .and_then(|e| e.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|e| {
                        Some((
                            e.get("path")?.as_str()?.to_string(),
                            e.get("is_dir").and_then(|d| d.as_bool()).unwrap_or(false),
                            FileMeta { size: e.get("size").and_then(|s| s.as_u64()).unwrap_or(0), mtime: e.get("mtime").and_then(|m| m.as_i64()).unwrap_or(0) },
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Feeds an upload from another thread, so the file being backed up can be read while it streams
struct ChannelReader {
    rx: mpsc::Receiver<Vec<u8>>,
    buf: Vec<u8>,
    pos: usize,
}

impl Read for ChannelReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if self.pos >= self.buf.len() {
            match self.rx.recv() {
                Ok(chunk) => {
                    self.buf = chunk;
                    self.pos = 0;
                }
                Err(_) => return Ok(0), // sender finished
            }
        }
        let n = out.len().min(self.buf.len() - self.pos);
        out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

struct Served(reqwest::blocking::Response);

impl Read for Served {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl Store for FleetStore {
    fn label(&self) -> String {
        self.label.clone()
    }

    fn root_exists(&self) -> bool {
        self.stat_value("").and_then(|v| v.get("kind").and_then(|k| k.as_str()).map(|k| k == "dir")).unwrap_or(false)
    }

    fn ensure_root(&self) -> Result<(), String> {
        self.op(json!({ "op": "mkdir", "path": self.root }))
    }

    fn list(&self, base: &str, skip: &dyn Fn(&str, bool) -> bool, cancel: &AtomicBool, warnings: &mut Vec<String>) -> Result<Listing, String> {
        let start = self.path(base);
        let mut out = Listing::new();
        if let Some(v) = self.stat_value(base) {
            if v.get("kind").and_then(|k| k.as_str()) == Some("file") {
                let name = start.rsplit('/').next().unwrap_or_default().to_string();
                out.insert(name, FileMeta { size: v.get("size").and_then(|s| s.as_u64()).unwrap_or(0), mtime: v.get("mtime").and_then(|m| m.as_i64()).unwrap_or(0) });
                return Ok(out);
            }
        }
        let v = self.get_json("list", &[("path", &start), ("recursive", "1")])?;
        if cancel.load(Ordering::Relaxed) {
            return Err("Cancelled".into());
        }
        if let Some(w) = v.get("warnings").and_then(|w| w.as_array()) {
            warnings.extend(w.iter().filter_map(|x| x.as_str()).map(|x| format!("On the Fleet machine: {x}")));
        }
        for (rel, _, m) in Self::entries(&v) {
            // Everything comes in one list, so folder rules are checked against each file's folders
            let mut acc = String::new();
            let mut skipped = false;
            for part in rel.split('/') {
                acc = if acc.is_empty() { part.to_string() } else { format!("{acc}/{part}") };
                if skip(&acc, acc.len() != rel.len()) {
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
        let v = self.get_json("list", &[("path", &self.path(rel)), ("recursive", "0")])?;
        Ok(Self::entries(&v).into_iter().map(|(name, is_dir, m)| DirItem { name, is_dir, size: m.size, mtime: m.mtime }).collect())
    }

    fn stat(&self, rel: &str) -> Option<FileMeta> {
        let v = self.stat_value(rel)?;
        (v.get("kind")?.as_str()? == "file").then(|| FileMeta { size: v.get("size").and_then(|s| s.as_u64()).unwrap_or(0), mtime: v.get("mtime").and_then(|m| m.as_i64()).unwrap_or(0) })
    }

    fn is_dir(&self, rel: &str) -> bool {
        self.stat_value(rel).and_then(|v| v.get("kind").and_then(|k| k.as_str()).map(|k| k == "dir")).unwrap_or(false)
    }

    fn open_read(&self, rel: &str) -> Result<Box<dyn Read + Send>, String> {
        let resp = self.req(reqwest::Method::GET, "read").query(&[("path", self.path(rel))]).send().map_err(|e| format!("Cannot reach the Fleet machine: {e}"))?;
        Ok(Box::new(Served(Self::check(resp)?)))
    }

    fn write_part(&self, rel: &str, data: &mut dyn Read, mtime: i64, on_bytes: &mut dyn FnMut(u64) -> bool) -> Result<String, String> {
        let part = super::store::part_name(rel);
        let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(4);
        let cancelled = AtomicBool::new(false);
        let mtime_s = mtime.to_string();
        let part_path = self.path(&part);
        let result = std::thread::scope(|scope| {
            // The upload runs on a helper thread while this one reads the file and feeds it in
            let upload = scope.spawn(|| {
                let body = reqwest::blocking::Body::new(ChannelReader { rx, buf: Vec::new(), pos: 0 });
                self.req(reqwest::Method::PUT, "write").query(&[("path", part_path.as_str()), ("mtime", mtime_s.as_str())]).body(body).send()
            });
            let fed = (|| -> Result<(), String> {
                let mut written = 0u64;
                loop {
                    let mut buf = vec![0u8; 256 * 1024];
                    let n = data.read(&mut buf).map_err(|e| format!("Read failed: {e}"))?;
                    if n == 0 {
                        return Ok(());
                    }
                    buf.truncate(n);
                    written += n as u64;
                    if tx.send(buf).is_err() {
                        return Err("The upload stopped".into());
                    }
                    if !on_bytes(written) {
                        cancelled.store(true, Ordering::Relaxed);
                        return Err("Cancelled".into());
                    }
                }
            })();
            drop(tx); // ends the upload body
            let sent = upload.join().map_err(|_| "The upload stopped unexpectedly".to_string());
            match (fed, sent) {
                (Err(e), _) | (Ok(()), Err(e)) => Err(e),
                (Ok(()), Ok(Err(e))) => Err(format!("Cannot reach the Fleet machine: {e}")),
                (Ok(()), Ok(Ok(resp))) => Self::check(resp).map(|_| ()),
            }
        });
        match result {
            Ok(()) => Ok(part),
            Err(e) => {
                let _ = self.remove_file(&part);
                Err(e)
            }
        }
    }

    fn rename(&self, from: &str, to: &str) -> Result<(), String> {
        self.op(json!({ "op": "rename", "path": self.path(from), "to": self.path(to) }))
    }

    fn remove_file(&self, rel: &str) -> Result<(), String> {
        self.op(json!({ "op": "remove_file", "path": self.path(rel) }))
    }

    fn remove_dir_all(&self, rel: &str) -> Result<(), String> {
        self.op(json!({ "op": "remove_dir_all", "path": self.path(rel) }))
    }

    fn remove_empty_parents(&self, rel: &str) {
        let _ = self.op(json!({ "op": "remove_empty_parents", "path": self.path(rel), "stop_at": self.root }));
    }

    fn local_path(&self, _rel: &str) -> Option<PathBuf> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses() {
        assert_eq!(parse("fleet://nas/mnt/backups/").unwrap(), ("nas".to_string(), "/mnt/backups".to_string()));
        assert_eq!(parse("fleet://nas").unwrap().1, "/");
        assert!(parse("fleet:///x").is_err());
        let s = FleetStore::new("fleet://nas/mnt/b", "http://nas:3140/", None).unwrap();
        assert_eq!((s.endpoint.as_str(), s.path("x/y")), ("http://nas:3140", "/mnt/b/x/y".to_string()));
        assert!(FleetStore::new("fleet://nas/x", "", None).is_err());
    }
}
