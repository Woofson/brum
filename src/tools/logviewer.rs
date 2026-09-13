use regex::Regex;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::Path;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LogEntry {
    pub line_number: usize,
    pub level: String, // "FATAL", "ERROR", "WARN", "INFO", "DEBUG", "TRACE", "PLAIN"
    pub timestamp: Option<String>,
    pub text: String,
}

#[derive(Debug, Deserialize)]
pub struct LogTailRequest {
    pub path: String,
    pub lines: Option<usize>,
    pub filter: Option<String>,
    pub invert_filter: Option<bool>,
    pub level_filter: Option<String>, // "ALL", "ERROR", "WARN", "INFO", etc.
}

#[derive(Debug, Serialize)]
pub struct LogTailResponse {
    pub file_path: String,
    pub file_size: u64,
    pub total_lines_read: usize,
    pub matched_lines: usize,
    pub counts: LogLevelCounts,
    pub entries: Vec<LogEntry>,
}

#[derive(Debug, Serialize, Default)]
pub struct LogLevelCounts {
    pub total: usize,
    pub errors: usize,
    pub warnings: usize,
    pub info: usize,
    pub debug: usize,
}

/// Detects log level from a line of text
fn detect_log_level(line: &str) -> &'static str {
    let upper = line.to_uppercase();
    if upper.contains("FATAL") || upper.contains("CRITICAL") || upper.contains("EMERGENCY") {
        "FATAL"
    } else if upper.contains("ERROR") || upper.contains(" ERR ") || upper.contains("[ERR]") {
        "ERROR"
    } else if upper.contains("WARN") || upper.contains("WARNING") || upper.contains("[WARN]") {
        "WARN"
    } else if upper.contains("INFO") || upper.contains("[INFO]") || upper.contains("NOTICE") {
        "INFO"
    } else if upper.contains("DEBUG") || upper.contains("[DEBUG]") {
        "DEBUG"
    } else if upper.contains("TRACE") || upper.contains("[TRACE]") {
        "TRACE"
    } else {
        "PLAIN"
    }
}

/// Attempts to extract an ISO/standard timestamp from the start of a log line
fn extract_timestamp(line: &str) -> Option<String> {
    if line.len() < 10 {
        return None;
    }
    // Pattern 1: ISO8601 (2026-09-13...)
    if line.chars().take(4).all(|c| c.is_ascii_digit()) && line.chars().nth(4) == Some('-') {
        let ts_len = line.find(' ').or_else(|| line.find(']')).unwrap_or(19).min(30);
        return Some(line[..ts_len].trim_matches('[').trim_matches(']').to_string());
    }
    // Pattern 2: [2026-09-13...] or [18:30:15]
    if line.starts_with('[') {
        if let Some(end) = line.find(']') {
            if end <= 32 {
                return Some(line[1..end].to_string());
            }
        }
    }
    None
}

/// Reads the tail of a log file efficiently
pub fn tail_log_file(req: LogTailRequest) -> Result<LogTailResponse, String> {
    let path = Path::new(&req.path);
    if !path.exists() {
        return Err("Log file does not exist".to_string());
    }

    let file = File::open(path).map_err(|e| e.to_string())?;
    let file_size = file.metadata().map(|m| m.len()).unwrap_or(0);
    let max_lines = req.lines.unwrap_or(500).min(10000);

    let regex_filter = if let Some(ref f) = req.filter {
        if !f.trim().is_empty() {
            Regex::new(f).ok()
        } else {
            None
        }
    } else {
        None
    };

    let invert = req.invert_filter.unwrap_or(false);
    let target_level = req.level_filter.as_deref().unwrap_or("ALL").to_uppercase();

    // For files > 2MB, seek backwards roughly estimated bytes
    let mut reader = BufReader::new(file);
    if file_size > 2 * 1024 * 1024 {
        let avg_line_len = 150;
        let estimated_seek = (max_lines as u64 * 3 * avg_line_len).min(file_size);
        let seek_pos = file_size.saturating_sub(estimated_seek);
        if seek_pos > 0 {
            let _ = reader.seek(SeekFrom::Start(seek_pos));
            // Discard first partial line
            let mut discard = String::new();
            let _ = reader.read_line(&mut discard);
        }
    }

    let mut raw_lines = Vec::new();
    let mut line_num = 1;

    for line_res in reader.lines() {
        if let Ok(line) = line_res {
            raw_lines.push((line_num, line));
            line_num += 1;
        }
    }

    // Keep last max_lines
    let total_read = raw_lines.len();
    let start_idx = total_read.saturating_sub(max_lines);
    let slice = &raw_lines[start_idx..];

    let mut counts = LogLevelCounts::default();
    let mut entries = Vec::new();

    for (ln, text) in slice {
        let level = detect_log_level(text);
        counts.total += 1;
        match level {
            "FATAL" | "ERROR" => counts.errors += 1,
            "WARN" => counts.warnings += 1,
            "INFO" => counts.info += 1,
            "DEBUG" | "TRACE" => counts.debug += 1,
            _ => {}
        }

        // Apply level filter
        if target_level != "ALL" {
            if target_level == "ERROR" && level != "ERROR" && level != "FATAL" {
                continue;
            }
            if target_level == "WARN" && level != "WARN" && level != "ERROR" && level != "FATAL" {
                continue;
            }
            if target_level == "INFO" && level != "INFO" {
                continue;
            }
            if target_level == "DEBUG" && level != "DEBUG" && level != "TRACE" {
                continue;
            }
        }

        // Apply regex/substring filter
        if let Some(ref re) = regex_filter {
            let matches = re.is_match(text);
            if (!invert && !matches) || (invert && matches) {
                continue;
            }
        }

        let timestamp = extract_timestamp(text);
        entries.push(LogEntry {
            line_number: *ln,
            level: level.to_string(),
            timestamp,
            text: text.clone(),
        });
    }

    let matched_lines = entries.len();

    Ok(LogTailResponse {
        file_path: req.path,
        file_size,
        total_lines_read: total_read,
        matched_lines,
        counts,
        entries,
    })
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_log_viewer_tail_and_filters() {
        let dir = tempdir().unwrap();
        let log_file = dir.path().join("app.log");

        let log_content = "\
2026-09-13T12:00:01Z [INFO] Server started on port 3000
2026-09-13T12:00:02Z [DEBUG] Loaded config master.json
2026-09-13T12:00:03Z [WARN] Disk space high on /mnt/data
2026-09-13T12:00:04Z [ERROR] Failed to connect to database host: timeout
2026-09-13T12:00:05Z [FATAL] Panic in worker thread
";
        fs::write(&log_file, log_content).unwrap();

        let req = LogTailRequest {
            path: log_file.to_string_lossy().to_string(),
            lines: Some(10),
            filter: None,
            level_filter: Some("ALL".to_string()),
            invert_filter: Some(false),
        };

        let res = tail_log_file(req).unwrap();
        assert_eq!(res.total_lines_read, 5);
        assert_eq!(res.matched_lines, 5);
        assert_eq!(res.counts.errors, 2); // ERROR + FATAL
        assert_eq!(res.counts.warnings, 1);
        assert_eq!(res.counts.info, 1);
        assert_eq!(res.counts.debug, 1);

        // Filter only ERROR
        let req_err = LogTailRequest {
            path: log_file.to_string_lossy().to_string(),
            lines: Some(10),
            filter: None,
            level_filter: Some("ERROR".to_string()),
            invert_filter: Some(false),
        };
        let res_err = tail_log_file(req_err).unwrap();
        assert_eq!(res_err.matched_lines, 2);

        // Search text "database"
        let req_filter = LogTailRequest {
            path: log_file.to_string_lossy().to_string(),
            lines: Some(10),
            filter: Some("database".to_string()),
            level_filter: Some("ALL".to_string()),
            invert_filter: Some(false),
        };
        let res_filter = tail_log_file(req_filter).unwrap();
        assert_eq!(res_filter.matched_lines, 1);
        assert!(res_filter.entries[0].text.contains("Failed to connect to database"));
    }
}

