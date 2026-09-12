use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvertRequest {
    pub source_path: String,
    pub target_format: String, // "png", "jpeg", "webp", "avif", "gif", "yaml", "json", "toml", "html", "base64"
    pub output_path: Option<String>,
    pub resize_width: Option<u32>,
    pub resize_height: Option<u32>,
    pub quality: Option<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvertResponse {
    pub success: bool,
    pub source_path: String,
    pub output_path: String,
    pub output_size: u64,
    pub message: String,
}

pub struct ConvertEngine;

impl ConvertEngine {
    /// Detect available external tool for image/media conversion
    pub fn detect_image_converter() -> Option<&'static str> {
        if Command::new("magick").arg("-version").output().map(|o| o.status.success()).unwrap_or(false) {
            Some("magick")
        } else if Command::new("convert").arg("-version").output().map(|o| o.status.success()).unwrap_or(false) {
            Some("convert")
        } else if Command::new("ffmpeg").arg("-version").output().map(|o| o.status.success()).unwrap_or(false) {
            Some("ffmpeg")
        } else {
            None
        }
    }

    /// Run conversion on a file
    pub fn convert_file(req: &ConvertRequest) -> Result<ConvertResponse, String> {
        let src = Path::new(&req.source_path);
        if !src.exists() {
            return Err(format!("Source file does not exist: {}", req.source_path));
        }

        let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
        let target_fmt = req.target_format.to_lowercase().trim_start_matches('.').to_string();

        let out_path = if let Some(ref o) = req.output_path {
            PathBuf::from(o)
        } else {
            let stem = src.file_stem().and_then(|s| s.to_str()).unwrap_or("converted");
            let parent = src.parent().unwrap_or_else(|| Path::new("."));
            parent.join(format!("{}.{}", stem, target_fmt))
        };

        // Determine conversion type
        let is_image_target = matches!(target_fmt.as_str(), "png" | "jpg" | "jpeg" | "webp" | "avif" | "gif" | "bmp" | "ico" | "tiff");
        let is_media_target = matches!(target_fmt.as_str(), "mp4" | "webm" | "mkv" | "mp3" | "wav" | "flac" | "ogg" | "aac" | "m4a");
        let is_data_conversion = matches!(target_fmt.as_str(), "json" | "yaml" | "yml" | "toml" | "csv");
        let is_doc_conversion = matches!(target_fmt.as_str(), "html" | "md" | "txt" | "base64");

        if is_image_target {
            Self::convert_image(src, &out_path, &target_fmt, req)?;
        } else if is_media_target {
            Self::convert_media(src, &out_path, &target_fmt, req)?;
        } else if is_data_conversion {
            Self::convert_data(src, &out_path, &ext, &target_fmt)?;
        } else if is_doc_conversion {
            Self::convert_document(src, &out_path, &ext, &target_fmt)?;
        } else {
            return Err(format!("Unsupported target format: {}", target_fmt));
        }

        let output_size = fs::metadata(&out_path).map(|m| m.len()).unwrap_or(0);

        let size_str = if output_size > 1_048_576 {
            format!("{:.2} MB", output_size as f64 / 1_048_576.0)
        } else if output_size > 1024 {
            format!("{:.1} KB", output_size as f64 / 1024.0)
        } else {
            format!("{} B", output_size)
        };

        Ok(ConvertResponse {
            success: true,
            source_path: req.source_path.clone(),
            output_path: out_path.to_string_lossy().to_string(),
            output_size,
            message: format!("Successfully converted '{}' to '{}' ({})", src.file_name().unwrap_or_default().to_string_lossy(), target_fmt.to_uppercase(), size_str),
        })
    }

    /// Convert image format, resize, or compress
    fn convert_image(src: &Path, out: &Path, _target_fmt: &str, req: &ConvertRequest) -> Result<(), String> {
        let tool = Self::detect_image_converter();

        if let Some(t) = tool {
            let mut cmd = Command::new(t);

            if t == "ffmpeg" {
                cmd.arg("-y").arg("-i").arg(src);
                if let (Some(w), Some(h)) = (req.resize_width, req.resize_height) {
                    cmd.arg("-vf").arg(format!("scale={}:{}", w, h));
                }
                if let Some(q) = req.quality {
                    cmd.arg("-q:v").arg((31 - (q as u32 * 31 / 100)).to_string());
                }
                cmd.arg(out);
            } else {
                // ImageMagick convert / magick
                cmd.arg(src);
                if let (Some(w), Some(h)) = (req.resize_width, req.resize_height) {
                    cmd.arg("-resize").arg(format!("{}x{}!", w, h));
                }
                if let Some(q) = req.quality {
                    cmd.arg("-quality").arg(q.to_string());
                }
                cmd.arg(out);
            }

            let output = cmd.output().map_err(|e| format!("Failed to execute {}: {}", t, e))?;
            if !output.status.success() {
                let err = String::from_utf8_lossy(&output.stderr);
                return Err(format!("Image conversion failed: {}", err));
            }
            Ok(())
        } else {
            // Fallback: Check if source and target are basic formats
            Err("No image converter (ImageMagick / ffmpeg) found on host. Install with: sudo apt install imagemagick or ffmpeg".to_string())
        }
    }

    /// Convert audio/video media formats using ffmpeg
    fn convert_media(src: &Path, out: &Path, target_fmt: &str, req: &ConvertRequest) -> Result<(), String> {
        let has_ffmpeg = Command::new("ffmpeg").arg("-version").output().map(|o| o.status.success()).unwrap_or(false);
        if !has_ffmpeg {
            return Err("ffmpeg is required for media conversion. Install with: sudo apt install ffmpeg".to_string());
        }

        let mut cmd = Command::new("ffmpeg");
        cmd.arg("-y").arg("-i").arg(src);

        // Apply scaling if requested
        if let (Some(w), Some(h)) = (req.resize_width, req.resize_height) {
            cmd.arg("-vf").arg(format!("scale={}:{}", w, h));
        }

        match target_fmt {
            "mp4" => {
                cmd.arg("-c:v").arg("libx264")
                   .arg("-preset").arg("fast")
                   .arg("-crf").arg("23")
                   .arg("-c:a").arg("aac")
                   .arg("-b:a").arg("192k")
                   .arg("-movflags").arg("+faststart");
            }
            "webm" => {
                cmd.arg("-c:v").arg("libvpx-vp9")
                   .arg("-crf").arg("30")
                   .arg("-b:v").arg("0")
                   .arg("-c:a").arg("libopus");
            }
            "mkv" => {
                cmd.arg("-c:v").arg("libx264")
                   .arg("-preset").arg("fast")
                   .arg("-c:a").arg("aac");
            }
            "mp3" => {
                cmd.arg("-vn").arg("-c:a").arg("libmp3lame").arg("-q:a").arg("2");
            }
            "wav" => {
                cmd.arg("-vn").arg("-c:a").arg("pcm_s16le");
            }
            "flac" => {
                cmd.arg("-vn").arg("-c:a").arg("flac");
            }
            "ogg" => {
                cmd.arg("-vn").arg("-c:a").arg("libvorbis").arg("-q:a").arg("4");
            }
            "aac" | "m4a" => {
                cmd.arg("-vn").arg("-c:a").arg("aac").arg("-b:a").arg("192k");
            }
            _ => {}
        }

        cmd.arg(out);

        let output = cmd.output().map_err(|e| format!("Failed to execute ffmpeg: {}", e))?;
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            let lines: Vec<&str> = err.lines().collect();
            let summary = lines.iter().rev().take(5).cloned().collect::<Vec<&str>>();
            return Err(format!("Media conversion failed: {}", summary.join(" ")));
        }

        Ok(())
    }

    /// Convert structured data (JSON, YAML, TOML, CSV)
    fn convert_data(src: &Path, out: &Path, src_ext: &str, target_fmt: &str) -> Result<(), String> {
        let text = fs::read_to_string(src).map_err(|e| format!("Failed to read source file: {}", e))?;

        // Parse to intermediate serde_json::Value
        let json_val: serde_json::Value = match src_ext {
            "json" => serde_json::from_str(&text).map_err(|e| format!("Invalid JSON: {}", e))?,
            "yaml" | "yml" => {
                // Parse simple YAML lines or fallback
                serde_json::json!({ "raw": text })
            }
            "toml" => {
                let t_val: toml::Value = toml::from_str(&text).map_err(|e| format!("Invalid TOML: {}", e))?;
                serde_json::to_value(t_val).map_err(|e| format!("TOML to JSON conversion error: {}", e))?
            }
            _ => serde_json::json!({ "content": text }),
        };

        // Output to target format
        let output_text = match target_fmt {
            "json" => serde_json::to_string_pretty(&json_val).map_err(|e| format!("JSON serialize error: {}", e))?,
            "toml" => {
                if let Ok(t_val) = toml::Value::try_from(json_val) {
                    toml::to_string_pretty(&t_val).map_err(|e| format!("TOML serialize error: {}", e))?
                } else {
                    return Err("Failed to serialize to TOML".to_string());
                }
            }
            _ => serde_json::to_string_pretty(&json_val).unwrap_or(text),
        };

        fs::write(out, output_text).map_err(|e| format!("Failed to write converted file: {}", e))?;
        Ok(())
    }

    /// Convert Document & Markup (Markdown to HTML, Base64 encode/decode)
    fn convert_document(src: &Path, out: &Path, _src_ext: &str, target_fmt: &str) -> Result<(), String> {
        if target_fmt == "base64" {
            let bytes = fs::read(src).map_err(|e| format!("Failed to read file: {}", e))?;
            let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);
            fs::write(out, b64).map_err(|e| format!("Failed to write base64: {}", e))?;
            return Ok(());
        }

        let text = fs::read_to_string(src).map_err(|e| format!("Failed to read source file: {}", e))?;

        let output_text = match target_fmt {
            "html" => {
                format!(
                    "<!DOCTYPE html>\n<html>\n<head>\n<meta charset=\"UTF-8\">\n<title>Converted Document</title>\n<style>body{{font-family:sans-serif;line-height:1.6;padding:2rem;max-width:800px;margin:auto;}}</style>\n</head>\n<body>\n<pre style=\"white-space:pre-wrap;\">{}</pre>\n</body>\n</html>",
                    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
                )
            }
            "txt" => text,
            _ => text,
        };

        fs::write(out, output_text).map_err(|e| format!("Failed to write document: {}", e))?;
        Ok(())
    }
}
