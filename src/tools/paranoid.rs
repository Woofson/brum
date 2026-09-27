use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParanoidVerificationReport {
    pub source: String,
    pub destination: String,
    pub source_sha256: String,
    pub dest_sha256: String,
    pub verified: bool,
    pub file_size: u64,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DryRunPreview {
    pub action: String, // "copy", "move", "delete"
    pub sources: Vec<String>,
    pub destination_target: Option<String>,
    pub estimated_total_bytes: u64,
    pub potential_overwrites: Vec<String>,
    pub disk_space_available_bytes: u64,
    pub safe_to_proceed: bool,
    pub warnings: Vec<String>,
}

pub struct ParanoidEngine;

fn percent_decode(s: &str) -> String {
    let mut bytes = Vec::new();
    let mut chars = s.bytes();
    while let Some(b) = chars.next() {
        if b == b'%' {
            let h1 = chars.next();
            let h2 = chars.next();
            if let (Some(h1), Some(h2)) = (h1, h2) {
                if let Ok(val) = u8::from_str_radix(
                    &format!("{}{}", h1 as char, h2 as char),
                    16,
                ) {
                    bytes.push(val);
                    continue;
                }
            }
        }
        bytes.push(b);
    }
    String::from_utf8_lossy(&bytes).to_string()
}

impl ParanoidEngine {
    pub fn dry_run(
        action: &str,
        sources: &[String],
        dest_dir: Option<&str>,
    ) -> Result<DryRunPreview, std::io::Error> {
        let mut total_bytes = 0u64;
        let mut potential_overwrites = Vec::new();
        let mut warnings = Vec::new();

        let decoded_dest = dest_dir.map(percent_decode);

        for raw_src in sources {
            let decoded_src = percent_decode(raw_src);
            let is_remote_protocol = decoded_src.contains("://");

            if is_remote_protocol {
                // Remote VFS URI (e.g. smb://, sftp://, nfs://, client://)
                // Cannot stat directly with local Path::new, assume valid remote source item
                total_bytes += 1024 * 1024;
                continue;
            }

            let p = if Path::new(&decoded_src).exists() {
                Path::new(&decoded_src).to_path_buf()
            } else if Path::new(raw_src).exists() {
                Path::new(raw_src).to_path_buf()
            } else {
                warnings.push(format!("Source does not exist: {}", decoded_src));
                continue;
            };

            if let Ok(meta) = p.metadata() {
                total_bytes += meta.len();
            }

            if let Some(ref dest) = decoded_dest {
                if !dest.contains("://") {
                    let dest_path = Path::new(dest);
                    if let Some(name) = p.file_name() {
                        let target = dest_path.join(name);
                        if let (Ok(can_p), Ok(can_t)) = (p.canonicalize(), target.canonicalize()) {
                            if can_p == can_t {
                                warnings.push(format!("Source and destination are identical: {}", decoded_src));
                            } else if p.is_dir() && can_t.starts_with(&can_p) {
                                warnings.push(format!("Cannot {} directory into its own subdirectory: {} -> {}", action, decoded_src, dest));
                            }
                        } else if target.exists() {
                            potential_overwrites.push(target.to_string_lossy().to_string());
                        }
                    }
                }
            }
        }

        let disk_space_available = 100_000_000_000u64; // Fallback ~100GB or sysinfo

        if !potential_overwrites.is_empty() {
            warnings.push(format!("{} destination files will be overwritten", potential_overwrites.len()));
        }

        let safe = warnings.is_empty() || (warnings.len() == 1 && !potential_overwrites.is_empty());

        Ok(DryRunPreview {
            action: action.to_string(),
            sources: sources.to_vec(),
            destination_target: dest_dir.map(|s| s.to_string()),
            estimated_total_bytes: total_bytes,
            potential_overwrites,
            disk_space_available_bytes: disk_space_available,
            safe_to_proceed: safe,
            warnings,
        })
    }
}
