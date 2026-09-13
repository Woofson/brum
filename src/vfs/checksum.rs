use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChecksumResult {
    pub path: String,
    pub algorithm: String,
    pub hash: String,
    pub file_size: u64,
}

pub fn calculate_sha256<P: AsRef<Path>>(path: P) -> Result<String, std::io::Error> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 65536];

    loop {
        let bytes_read = reader.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }

    Ok(hex::encode(hasher.finalize()))
}

pub fn calculate_crc32<P: AsRef<Path>>(path: P) -> Result<String, std::io::Error> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut hasher = crc32fast::Hasher::new();
    let mut buffer = [0u8; 65536];

    loop {
        let bytes_read = reader.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }

    Ok(format!("{:08x}", hasher.finalize()))
}

pub fn calculate_checksum<P: AsRef<Path>>(path: P, algorithm: &str) -> Result<ChecksumResult, std::io::Error> {
    let p = path.as_ref();
    let meta = std::fs::metadata(p)?;
    let size = meta.len();

    let hash_str = match algorithm.to_lowercase().as_str() {
        "crc32" => calculate_crc32(p)?,
        "sha256" | _ => calculate_sha256(p)?,
    };

    Ok(ChecksumResult {
        path: p.to_string_lossy().to_string(),
        algorithm: algorithm.to_string(),
        hash: hash_str,
        file_size: size,
    })
}
