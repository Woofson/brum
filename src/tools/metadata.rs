use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct AudioMetadata {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub year: Option<String>,
    pub track_number: Option<u32>,
    pub track_total: Option<u32>,
    pub disc_number: Option<u32>,
    pub genre: Option<String>,
    pub comment: Option<String>,
    pub duration_seconds: Option<f64>,
    pub bitrate_kbps: Option<u32>,
    pub sample_rate: Option<u32>,
    pub has_cover_art: bool,
    pub cover_art_mime: Option<String>,
    pub cover_art_base64: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct PhotoMetadata {
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    pub lens_model: Option<String>,
    pub date_taken: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub iso: Option<u32>,
    pub exposure_time: Option<String>,
    pub f_number: Option<String>,
    pub focal_length: Option<String>,
    pub gps_latitude: Option<f64>,
    pub gps_longitude: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FileMetadataResponse {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub file_type: String, // "audio", "image", "other"
    pub audio: Option<AudioMetadata>,
    pub photo: Option<PhotoMetadata>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateMetadataRequest {
    pub path: String,
    pub audio: Option<AudioMetadata>,
    pub photo: Option<PhotoMetadata>,
}

#[derive(Debug, Deserialize)]
pub struct BatchMetadataRequest {
    pub files: Vec<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub year: Option<String>,
    pub genre: Option<String>,
    pub auto_track_numbers: Option<bool>,
    pub start_track_number: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct BatchMetadataResponse {
    pub updated_count: usize,
    pub failed_files: Vec<String>,
}

/// Reads ID3v1 tags from the last 128 bytes of an MP3 file
fn read_id3v1(file: &mut File, size: u64) -> Option<AudioMetadata> {
    if size < 128 {
        return None;
    }
    file.seek(SeekFrom::End(-128)).ok()?;
    let mut buf = [0u8; 128];
    file.read_exact(&mut buf).ok()?;

    if &buf[0..3] != b"TAG" {
        return None;
    }

    let parse_str = |slice: &[u8]| -> Option<String> {
        let s = String::from_utf8_lossy(slice).trim().trim_matches('\0').to_string();
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    };

    let title = parse_str(&buf[3..33]);
    let artist = parse_str(&buf[33..63]);
    let album = parse_str(&buf[63..93]);
    let year = parse_str(&buf[93..97]);
    let comment = parse_str(&buf[97..127]);
    let track_number = if buf[125] == 0 && buf[126] != 0 {
        Some(buf[126] as u32)
    } else {
        None
    };

    Some(AudioMetadata {
        title,
        artist,
        album,
        year,
        comment,
        track_number,
        ..Default::default()
    })
}

/// Helper to decode syncsafe integers used in ID3v2 headers
fn syncsafe_to_u32(b: &[u8]) -> u32 {
    ((b[0] as u32 & 0x7F) << 21)
        | ((b[1] as u32 & 0x7F) << 14)
        | ((b[2] as u32 & 0x7F) << 7)
        | (b[3] as u32 & 0x7F)
}

/// Reads ID3v2 tags from an MP3 file
fn read_id3v2(file: &mut File) -> Option<AudioMetadata> {
    file.seek(SeekFrom::Start(0)).ok()?;
    let mut header = [0u8; 10];
    file.read_exact(&mut header).ok()?;

    if &header[0..3] != b"ID3" {
        return None;
    }

    let version_major = header[3];
    let tag_size = syncsafe_to_u32(&header[6..10]) as usize;

    if tag_size > 10 * 1024 * 1024 {
        return None; // Guard against malformed tags > 10MB
    }

    let mut tag_data = vec![0u8; tag_size];
    file.read_exact(&mut tag_data).ok()?;

    let mut meta = AudioMetadata::default();
    let mut pos = 0;

    while pos + 10 <= tag_data.len() {
        let frame_id = match std::str::from_utf8(&tag_data[pos..pos + 4]) {
            Ok(id) if id.chars().all(|c| c.is_ascii_alphanumeric()) => id,
            _ => break,
        };

        let frame_size = if version_major == 4 {
            syncsafe_to_u32(&tag_data[pos + 4..pos + 8]) as usize
        } else {
            u32::from_be_bytes([
                tag_data[pos + 4],
                tag_data[pos + 5],
                tag_data[pos + 6],
                tag_data[pos + 7],
            ]) as usize
        };

        pos += 10;
        if pos + frame_size > tag_data.len() {
            break;
        }

        let frame_body = &tag_data[pos..pos + frame_size];
        pos += frame_size;

        if frame_body.is_empty() {
            continue;
        }

        let parse_text_frame = |body: &[u8]| -> Option<String> {
            if body.len() <= 1 {
                return None;
            }
            let encoding = body[0];
            let text_bytes = &body[1..];
            let raw = match encoding {
                1 | 2 => {
                    // UTF-16
                    let u16_vec: Vec<u16> = text_bytes
                        .chunks_exact(2)
                        .map(|c| u16::from_le_bytes([c[0], c[1]]))
                        .collect();
                    String::from_utf16_lossy(&u16_vec)
                }
                3 => String::from_utf8_lossy(text_bytes).to_string(), // UTF-8
                _ => String::from_utf8_lossy(text_bytes).to_string(), // ISO-8859-1
            };
            let clean = raw.trim().trim_matches('\0').to_string();
            if clean.is_empty() {
                None
            } else {
                Some(clean)
            }
        };

        match frame_id {
            "TIT2" => meta.title = parse_text_frame(frame_body),
            "TPE1" => meta.artist = parse_text_frame(frame_body),
            "TALB" => meta.album = parse_text_frame(frame_body),
            "TPE2" => meta.album_artist = parse_text_frame(frame_body),
            "TYER" | "TDRC" => meta.year = parse_text_frame(frame_body),
            "TCON" => meta.genre = parse_text_frame(frame_body),
            "COMM" => {
                if frame_body.len() > 4 {
                    meta.comment = parse_text_frame(&frame_body[4..]);
                }
            }
            "TRCK" => {
                if let Some(t_str) = parse_text_frame(frame_body) {
                    if let Some((num, tot)) = t_str.split_once('/') {
                        meta.track_number = num.trim().parse().ok();
                        meta.track_total = tot.trim().parse().ok();
                    } else {
                        meta.track_number = t_str.trim().parse().ok();
                    }
                }
            }
            "APIC" => {
                // Attached picture (Cover Art)
                if frame_body.len() > 10 {
                    let mime_end = frame_body[1..]
                        .iter()
                        .position(|&b| b == 0)
                        .unwrap_or(0);
                    let mime = String::from_utf8_lossy(&frame_body[1..1 + mime_end]).to_string();
                    let desc_start = 1 + mime_end + 2;
                    if desc_start < frame_body.len() {
                        let img_start = desc_start
                            + frame_body[desc_start..]
                                .iter()
                                .position(|&b| b == 0)
                                .unwrap_or(0)
                            + 1;
                        if img_start < frame_body.len() {
                            let img_bytes = &frame_body[img_start..];
                            use base64::Engine;
                            meta.has_cover_art = true;
                            meta.cover_art_mime = Some(if mime.is_empty() { "image/jpeg".to_string() } else { mime });
                            meta.cover_art_base64 = Some(base64::engine::general_purpose::STANDARD.encode(img_bytes));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    Some(meta)
}

/// Reads photo EXIF and basic dimensions from JPEG / PNG
fn read_photo_metadata(path: &Path) -> PhotoMetadata {
    let mut photo = PhotoMetadata::default();
    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return photo,
    };

    let mut buf = vec![0u8; 65536];
    let n = file.read(&mut buf).unwrap_or(0);
    buf.truncate(n);

    // PNG dimension
    if buf.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) && buf.len() >= 24 {
        photo.width = Some(u32::from_be_bytes([buf[16], buf[17], buf[18], buf[19]]));
        photo.height = Some(u32::from_be_bytes([buf[20], buf[21], buf[22], buf[23]]));
        return photo;
    }

    // JPEG dimension & EXIF scan
    if buf.starts_with(&[0xFF, 0xD8]) {
        let mut idx = 2;
        while idx + 4 < buf.len() {
            if buf[idx] != 0xFF {
                idx += 1;
                continue;
            }
            let marker = buf[idx + 1];
            let len = u16::from_be_bytes([buf[idx + 2], buf[idx + 3]]) as usize;
            
            // SOF0 / SOF2 (Baseline / Progressive DCT)
            if (marker == 0xC0 || marker == 0xC2) && idx + 9 < buf.len() {
                photo.height = Some(u16::from_be_bytes([buf[idx + 5], buf[idx + 6]]) as u32);
                photo.width = Some(u16::from_be_bytes([buf[idx + 7], buf[idx + 8]]) as u32);
            }

            // APP1 EXIF
            if marker == 0xE1 && idx + 10 < buf.len() && &buf[idx + 4..idx + 10] == b"Exif\0\0" {
                let exif_slice = &buf[idx + 10..idx + 2 + len.min(buf.len() - idx - 2)];
                parse_exif_tags(exif_slice, &mut photo);
            }

            idx += 2 + len;
        }
    }

    photo
}

/// Simple parser for key EXIF tags (TIFF Header & IFD0)
fn parse_exif_tags(data: &[u8], photo: &mut PhotoMetadata) {
    if data.len() < 8 {
        return;
    }
    let is_le = &data[0..2] == b"II";
    let read_u16 = |offset: usize| -> Option<u16> {
        if offset + 2 > data.len() {
            return None;
        }
        Some(if is_le {
            u16::from_le_bytes([data[offset], data[offset + 1]])
        } else {
            u16::from_be_bytes([data[offset], data[offset + 1]])
        })
    };
    let read_u32 = |offset: usize| -> Option<u32> {
        if offset + 4 > data.len() {
            return None;
        }
        Some(if is_le {
            u32::from_le_bytes([data[offset], data[offset + 1], data[offset + 2], data[offset + 3]])
        } else {
            u32::from_be_bytes([data[offset], data[offset + 1], data[offset + 2], data[offset + 3]])
        })
    };

    let ifd0_offset = read_u32(4).unwrap_or(8) as usize;
    if ifd0_offset + 2 > data.len() {
        return;
    }

    let num_entries = read_u16(ifd0_offset).unwrap_or(0) as usize;
    let mut pos = ifd0_offset + 2;

    for _ in 0..num_entries {
        if pos + 12 > data.len() {
            break;
        }
        let tag = read_u16(pos).unwrap_or(0);
        let count = read_u32(pos + 4).unwrap_or(0) as usize;
        let val_offset = read_u32(pos + 8).unwrap_or(0) as usize;

        let get_string = || -> Option<String> {
            if count == 0 {
                return None;
            }
            let str_bytes = if count <= 4 {
                &data[pos + 8..pos + 8 + count]
            } else if val_offset + count <= data.len() {
                &data[val_offset..val_offset + count]
            } else {
                return None;
            };
            Some(String::from_utf8_lossy(str_bytes).trim().trim_matches('\0').to_string())
        };

        match tag {
            0x010F => photo.camera_make = get_string(),
            0x0110 => photo.camera_model = get_string(),
            0x0132 | 0x9003 => photo.date_taken = get_string(),
            0xA434 => photo.lens_model = get_string(),
            _ => {}
        }
        pos += 12;
    }
}

/// Reads all available metadata for a file
pub fn read_file_metadata(file_path: &str) -> Result<FileMetadataResponse, String> {
    let path = Path::new(file_path);
    if !path.exists() {
        return Err("File does not exist".to_string());
    }

    let meta = fs::metadata(path).map_err(|e| e.to_string())?;
    let size = meta.len();
    let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    let mut file_type = "other".to_string();
    let mut audio = None;
    let mut photo = None;

    if ["mp3", "flac", "ogg", "wav", "m4a", "aac"].contains(&ext.as_str()) {
        file_type = "audio".to_string();
        if let Ok(mut f) = File::open(path) {
            let mut a = read_id3v2(&mut f).unwrap_or_default();
            if a.title.is_none() && a.artist.is_none() {
                if let Some(v1) = read_id3v1(&mut f, size) {
                    a = v1;
                }
            }
            audio = Some(a);
        }
    } else if ["jpg", "jpeg", "png", "webp", "bmp"].contains(&ext.as_str()) {
        file_type = "image".to_string();
        photo = Some(read_photo_metadata(path));
    }

    Ok(FileMetadataResponse {
        path: file_path.to_string(),
        name,
        size,
        file_type,
        audio,
        photo,
    })
}

/// Writes updated ID3v2 tags to an MP3 file
pub fn update_file_metadata(req: UpdateMetadataRequest) -> Result<(), String> {
    let path = Path::new(&req.path);
    if !path.exists() {
        return Err("File does not exist".to_string());
    }

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    if ext != "mp3" {
        return Ok(()); // Basic update handles MP3 ID3 tags
    }

    let audio = match req.audio {
        Some(a) => a,
        None => return Ok(()),
    };

    let file_content = fs::read(path).map_err(|e| e.to_string())?;

    // Determine if file starts with ID3v2 header
    let mut audio_start = 0;
    if file_content.starts_with(b"ID3") && file_content.len() >= 10 {
        let tag_size = syncsafe_to_u32(&file_content[6..10]) as usize;
        audio_start = 10 + tag_size;
    }

    // Build new ID3v2.4 frame bytes
    let mut frames = Vec::new();

    let mut add_text_frame = |frame_id: &[u8; 4], text: &Option<String>| {
        if let Some(t) = text {
            if !t.trim().is_empty() {
                let mut body = vec![3u8]; // UTF-8 encoding
                body.extend_from_slice(t.as_bytes());
                let frame_len = body.len() as u32;
                frames.extend_from_slice(frame_id);
                // syncsafe frame size
                frames.push(((frame_len >> 21) & 0x7F) as u8);
                frames.push(((frame_len >> 14) & 0x7F) as u8);
                frames.push(((frame_len >> 7) & 0x7F) as u8);
                frames.push((frame_len & 0x7F) as u8);
                frames.extend_from_slice(&[0, 0]); // Flags
                frames.extend_from_slice(&body);
            }
        }
    };

    add_text_frame(b"TIT2", &audio.title);
    add_text_frame(b"TPE1", &audio.artist);
    add_text_frame(b"TALB", &audio.album);
    add_text_frame(b"TPE2", &audio.album_artist);
    add_text_frame(b"TDRC", &audio.year);
    add_text_frame(b"TCON", &audio.genre);
    if let Some(track) = audio.track_number {
        let track_str = if let Some(tot) = audio.track_total {
            format!("{}/{}", track, tot)
        } else {
            track.to_string()
        };
        add_text_frame(b"TRCK", &Some(track_str));
    }
    if let Some(comm) = audio.comment {
        if !comm.trim().is_empty() {
            let mut body = vec![3u8, b'e', b'n', b'g', 0]; // UTF-8, lang: eng, short desc: empty
            body.extend_from_slice(comm.as_bytes());
            let frame_len = body.len() as u32;
            frames.extend_from_slice(b"COMM");
            frames.push(((frame_len >> 21) & 0x7F) as u8);
            frames.push(((frame_len >> 14) & 0x7F) as u8);
            frames.push(((frame_len >> 7) & 0x7F) as u8);
            frames.push((frame_len & 0x7F) as u8);
            frames.extend_from_slice(&[0, 0]);
            frames.extend_from_slice(&body);
        }
    }

    // Embed album artwork if provided in base64
    if let Some(art_b64) = audio.cover_art_base64 {
        use base64::Engine;
        if let Ok(img_bytes) = base64::engine::general_purpose::STANDARD.decode(art_b64) {
            let mime = audio.cover_art_mime.unwrap_or_else(|| "image/jpeg".to_string());
            let mut body = vec![0u8]; // ISO-8859-1 for mime
            body.extend_from_slice(mime.as_bytes());
            body.push(0); // Null terminator
            body.push(3); // Picture type 3 = Cover (front)
            body.push(0); // Empty description null terminator
            body.extend_from_slice(&img_bytes);

            let frame_len = body.len() as u32;
            frames.extend_from_slice(b"APIC");
            frames.push(((frame_len >> 21) & 0x7F) as u8);
            frames.push(((frame_len >> 14) & 0x7F) as u8);
            frames.push(((frame_len >> 7) & 0x7F) as u8);
            frames.push((frame_len & 0x7F) as u8);
            frames.extend_from_slice(&[0, 0]);
            frames.extend_from_slice(&body);
        }
    }

    let tag_len = frames.len() as u32;
    let mut new_id3 = Vec::new();
    new_id3.extend_from_slice(b"ID3");
    new_id3.push(4); // ID3v2.4
    new_id3.push(0); // Revision
    new_id3.push(0); // Flags
    new_id3.push(((tag_len >> 21) & 0x7F) as u8);
    new_id3.push(((tag_len >> 14) & 0x7F) as u8);
    new_id3.push(((tag_len >> 7) & 0x7F) as u8);
    new_id3.push((tag_len & 0x7F) as u8);
    new_id3.extend_from_slice(&frames);

    let raw_audio = &file_content[audio_start.min(file_content.len())..];
    let mut output = OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(path)
        .map_err(|e| e.to_string())?;

    output.write_all(&new_id3).map_err(|e| e.to_string())?;
    output.write_all(raw_audio).map_err(|e| e.to_string())?;

    Ok(())
}

/// Bulk updates tags across a list of audio files
pub fn batch_update_metadata(req: BatchMetadataRequest) -> BatchMetadataResponse {
    let mut updated_count = 0;
    let mut failed_files = Vec::new();
    let auto_num = req.auto_track_numbers.unwrap_or(false);
    let mut track_counter = req.start_track_number.unwrap_or(1);

    for file_str in req.files {
        let path = Path::new(&file_str);
        if !path.exists() {
            failed_files.push(file_str);
            continue;
        }

        let existing = read_file_metadata(&file_str).ok();
        let mut audio = existing.and_then(|e| e.audio).unwrap_or_default();

        if let Some(ref art) = req.artist {
            audio.artist = Some(art.clone());
        }
        if let Some(ref alb) = req.album {
            audio.album = Some(alb.clone());
        }
        if let Some(ref aa) = req.album_artist {
            audio.album_artist = Some(aa.clone());
        }
        if let Some(ref yr) = req.year {
            audio.year = Some(yr.clone());
        }
        if let Some(ref gn) = req.genre {
            audio.genre = Some(gn.clone());
        }
        if auto_num {
            audio.track_number = Some(track_counter);
            track_counter += 1;
        }

        match update_file_metadata(UpdateMetadataRequest {
            path: file_str.clone(),
            audio: Some(audio),
            photo: None,
        }) {
            Ok(_) => updated_count += 1,
            Err(_) => failed_files.push(file_str),
        }
    }

    BatchMetadataResponse {
        updated_count,
        failed_files,
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_id3v1_metadata_cycle() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("test.mp3");

        // Create a dummy MP3 file with 128-byte ID3v1 tag structure
        let content = vec![0u8; 1024];
        fs::write(&file, &content).unwrap();

        let req = UpdateMetadataRequest {
            path: file.to_string_lossy().to_string(),
            audio: Some(AudioMetadata {
                title: Some("Brum Song".to_string()),
                artist: Some("Bolt J Woofson".to_string()),
                album: Some("Woofsons Hits".to_string()),
                album_artist: None,
                year: Some("2026".to_string()),
                genre: Some("Electronic".to_string()),
                track_number: Some(7),
                track_total: None,
                comment: Some("Test comment".to_string()),
                ..Default::default()
            }),
            photo: None,
        };

        update_file_metadata(req).unwrap();

        let read_res = read_file_metadata(&file.to_string_lossy()).unwrap();
        assert!(read_res.audio.is_some());
        let audio = read_res.audio.unwrap();
        assert_eq!(audio.title.as_deref(), Some("Brum Song"));
        assert_eq!(audio.artist.as_deref(), Some("Bolt J Woofson"));
        assert_eq!(audio.album.as_deref(), Some("Woofsons Hits"));
        assert_eq!(audio.track_number, Some(7));
    }
}

