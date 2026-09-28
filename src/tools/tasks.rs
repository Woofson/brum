use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskInfo {
    pub id: String,
    pub name: String,
    pub action: String, // "copy", "move", "archive", "extract", "checksum", "download", "deltacopy", "upload"
    pub source: String,
    pub destination: String,
    pub bytes_processed: u64,
    pub total_bytes: u64,
    pub speed_bytes_per_sec: u64,
    pub files_processed: u64,
    pub total_files: u64,
    pub current_file: Option<String>,
    pub current_file_bytes: u64,
    pub current_file_total_bytes: u64,
    pub status: String, // "running", "completed", "failed", "cancelled", "paused"
    pub error_message: Option<String>,
    pub log_entries: Vec<String>,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    #[serde(default)]
    pub paranoid: bool,
    #[serde(default)]
    pub verified_files: u64,
    #[serde(default)]
    pub last_hash: Option<String>,
}

#[derive(Clone)]
pub struct TaskManager {
    tasks: Arc<RwLock<HashMap<String, TaskInfo>>>,
}

static GLOBAL_TASK_MANAGER: parking_lot::RwLock<Option<Arc<TaskManager>>> = parking_lot::RwLock::new(None);

pub fn set_global_task_manager(tm: Arc<TaskManager>) {
    *GLOBAL_TASK_MANAGER.write() = Some(tm);
}

pub fn get_global_task_manager() -> Option<Arc<TaskManager>> {
    GLOBAL_TASK_MANAGER.read().clone()
}

/// Masks credentials in any URI strings so passwords are never stored in task lists or logs
pub fn sanitize_credentials(text: &str) -> String {
    let re = regex::Regex::new(r"://([^:@\s/]+):([^@\s/]+)@").unwrap();
    re.replace_all(text, "://$1:***@").to_string()
}

impl TaskManager {
    pub fn new() -> Self {
        Self {
            tasks: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn create_task(
        &self,
        name: &str,
        action: &str,
        source: &str,
        destination: &str,
        total_bytes: u64,
    ) -> String {
        self.sync_create_task(name, action, source, destination, total_bytes)
    }

    pub fn sync_create_task(
        &self,
        name: &str,
        action: &str,
        source: &str,
        destination: &str,
        total_bytes: u64,
    ) -> String {
        let id = Uuid::new_v4().to_string();
        let safe_name = sanitize_credentials(name);
        let safe_src = sanitize_credentials(source);
        let safe_dest = sanitize_credentials(destination);

        let task = TaskInfo {
            id: id.clone(),
            name: safe_name.clone(),
            action: action.to_string(),
            source: safe_src,
            destination: safe_dest,
            bytes_processed: 0,
            total_bytes,
            speed_bytes_per_sec: 0,
            files_processed: 0,
            total_files: 1,
            current_file: None,
            current_file_bytes: 0,
            current_file_total_bytes: 0,
            status: "running".to_string(),
            error_message: None,
            log_entries: vec![format!(
                "[{}] Task started: {} (Action: {})",
                Utc::now().format("%H:%M:%S"),
                safe_name,
                action
            )],
            started_at: Utc::now().timestamp(),
            finished_at: None,
            paranoid: false,
            verified_files: 0,
            last_hash: None,
        };

        if let Ok(mut map) = self.tasks.write() {
            map.insert(id.clone(), task);
        }
        id
    }

    pub async fn is_cancelled(&self, id: &str) -> bool {
        self.sync_is_cancelled(id)
    }

    pub fn sync_is_cancelled(&self, id: &str) -> bool {
        if let Ok(map) = self.tasks.read() {
            if let Some(task) = map.get(id) {
                return task.status == "cancelled";
            }
        }
        false
    }

    pub async fn is_paused(&self, id: &str) -> bool {
        self.sync_is_paused(id)
    }

    pub fn sync_is_paused(&self, id: &str) -> bool {
        if let Ok(map) = self.tasks.read() {
            if let Some(task) = map.get(id) {
                return task.status == "paused";
            }
        }
        false
    }

    pub async fn set_task_totals(&self, id: &str, total_files: u64, total_bytes: u64) {
        if let Ok(mut map) = self.tasks.write() {
            if let Some(task) = map.get_mut(id) {
                task.total_files = total_files;
                task.total_bytes = total_bytes;
            }
        }
    }

    pub async fn set_paranoid(&self, id: &str, paranoid: bool) {
        if let Ok(mut map) = self.tasks.write() {
            if let Some(task) = map.get_mut(id) {
                task.paranoid = paranoid;
            }
        }
    }

    pub async fn update_progress(&self, id: &str, bytes_processed: u64, speed_bps: u64) {
        if let Ok(mut map) = self.tasks.write() {
            if let Some(task) = map.get_mut(id) {
                task.bytes_processed = bytes_processed;
                task.speed_bytes_per_sec = speed_bps;
            }
        }
    }

    pub fn sync_update_stream_progress(
        &self,
        id: &str,
        current_file: Option<&str>,
        cur_file_bytes: u64,
        cur_file_total: u64,
        files_processed: u64,
        total_files: u64,
        total_bytes_processed: u64,
        speed_bps: u64,
    ) {
        if let Ok(mut map) = self.tasks.write() {
            if let Some(task) = map.get_mut(id) {
                if let Some(f) = current_file {
                    task.current_file = Some(sanitize_credentials(f));
                }
                task.current_file_bytes = cur_file_bytes;
                task.current_file_total_bytes = cur_file_total;
                task.files_processed = files_processed;
                if total_files > 0 {
                    task.total_files = total_files;
                }
                task.bytes_processed = total_bytes_processed;
                task.speed_bytes_per_sec = speed_bps;
            }
        }
    }

    pub async fn update_task_details(
        &self,
        id: &str,
        current_file: Option<&str>,
        cur_file_bytes: u64,
        cur_file_total: u64,
        files_processed: u64,
        total_files: u64,
        total_bytes_processed: u64,
        speed_bps: u64,
        verified_files: Option<u64>,
        last_hash: Option<&str>,
        log_msg: Option<&str>,
    ) {
        if let Ok(mut map) = self.tasks.write() {
            if let Some(task) = map.get_mut(id) {
                if let Some(f) = current_file {
                    task.current_file = Some(sanitize_credentials(f));
                }
                task.current_file_bytes = cur_file_bytes;
                task.current_file_total_bytes = cur_file_total;
                task.files_processed = files_processed;
                task.total_files = total_files;
                task.bytes_processed = total_bytes_processed;
                task.speed_bytes_per_sec = speed_bps;
                if let Some(v) = verified_files {
                    task.verified_files = v;
                }
                if let Some(h) = last_hash {
                    task.last_hash = Some(h.to_string());
                }

                if let Some(msg) = log_msg {
                    let entry = format!("[{}] {}", Utc::now().format("%H:%M:%S"), sanitize_credentials(msg));
                    if task.log_entries.len() > 200 {
                        task.log_entries.remove(0);
                    }
                    task.log_entries.push(entry);
                }
            }
        }
    }

    pub async fn add_log_entry(&self, id: &str, log_msg: &str) {
        if let Ok(mut map) = self.tasks.write() {
            if let Some(task) = map.get_mut(id) {
                let entry = format!("[{}] {}", Utc::now().format("%H:%M:%S"), sanitize_credentials(log_msg));
                if task.log_entries.len() > 200 {
                    task.log_entries.remove(0);
                }
                task.log_entries.push(entry);
            }
        }
    }

    pub fn sync_complete_task(&self, id: &str) {
        if let Ok(mut map) = self.tasks.write() {
            if let Some(task) = map.get_mut(id) {
                task.status = "completed".to_string();
                task.bytes_processed = task.total_bytes;
                task.files_processed = task.total_files;
                task.finished_at = Some(Utc::now().timestamp());
                task.log_entries.push(format!(
                    "[{}] Task completed successfully",
                    Utc::now().format("%H:%M:%S")
                ));
            }
        }
    }

    pub async fn complete_task(&self, id: &str) {
        self.sync_complete_task(id);
    }

    pub fn sync_fail_task(&self, id: &str, error: &str) {
        if let Ok(mut map) = self.tasks.write() {
            if let Some(task) = map.get_mut(id) {
                let safe_err = sanitize_credentials(error);
                task.status = "failed".to_string();
                task.error_message = Some(safe_err.clone());
                task.finished_at = Some(Utc::now().timestamp());
                task.log_entries.push(format!(
                    "[{}] Error: {}",
                    Utc::now().format("%H:%M:%S"),
                    safe_err
                ));
            }
        }
    }

    pub async fn fail_task(&self, id: &str, error: &str) {
        self.sync_fail_task(id, error);
    }

    pub async fn cancel_task(&self, id: &str) -> bool {
        if let Ok(mut map) = self.tasks.write() {
            if let Some(task) = map.get_mut(id) {
                if task.status == "running" || task.status == "paused" {
                    task.status = "cancelled".to_string();
                    task.finished_at = Some(Utc::now().timestamp());
                    task.log_entries.push(format!(
                        "[{}] Task cancelled by user",
                        Utc::now().format("%H:%M:%S")
                    ));
                    return true;
                }
            }
        }
        false
    }

    pub async fn pause_task(&self, id: &str) -> bool {
        if let Ok(mut map) = self.tasks.write() {
            if let Some(task) = map.get_mut(id) {
                if task.status == "running" {
                    task.status = "paused".to_string();
                    task.log_entries.push(format!(
                        "[{}] Task paused",
                        Utc::now().format("%H:%M:%S")
                    ));
                    return true;
                }
            }
        }
        false
    }

    pub async fn resume_task(&self, id: &str) -> bool {
        if let Ok(mut map) = self.tasks.write() {
            if let Some(task) = map.get_mut(id) {
                if task.status == "paused" {
                    task.status = "running".to_string();
                    task.log_entries.push(format!(
                        "[{}] Task resumed",
                        Utc::now().format("%H:%M:%S")
                    ));
                    return true;
                }
            }
        }
        false
    }

    pub async fn clear_completed(&self) {
        if let Ok(mut map) = self.tasks.write() {
            map.retain(|_, task| task.status == "running" || task.status == "paused");
        }
    }

    pub async fn prune_old_completed(&self) {
        let now = Utc::now().timestamp();
        if let Ok(mut map) = self.tasks.write() {
            if map.len() > 15 {
                map.retain(|_, task| {
                    if task.status == "running" || task.status == "paused" {
                        return true;
                    }
                    if let Some(finished) = task.finished_at {
                        if now - finished > 60 {
                            return false;
                        }
                    }
                    true
                });
            }
        }
    }

    pub async fn get_task(&self, id: &str) -> Option<TaskInfo> {
        if let Ok(map) = self.tasks.read() {
            map.get(id).cloned()
        } else {
            None
        }
    }

    pub async fn list_tasks(&self) -> Vec<TaskInfo> {
        self.prune_old_completed().await;
        if let Ok(map) = self.tasks.read() {
            let mut list: Vec<TaskInfo> = map.values().cloned().collect();
            list.sort_by(|a, b| b.started_at.cmp(&a.started_at));
            list
        } else {
            Vec::new()
        }
    }
}
