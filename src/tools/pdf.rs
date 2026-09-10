// Pure-Rust PDF Split & Merger Tool - Brum
use lopdf::{Document, Object, ObjectId, dictionary};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::info;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdfInfo {
    pub path: String,
    pub page_count: usize,
    pub version: String,
    pub title: Option<String>,
    pub author: Option<String>,
    pub file_size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageSpec {
    pub page_num: u32,
    pub rotation: i32, // 0, 90, 180, 270
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeRequest {
    pub sources: Vec<String>,
    pub destination: String,
    pub add_bookmarks: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SplitRequest {
    pub source: String,
    pub destination_dir: String,
    pub split_mode: String, // "ranges", "each_page", "page_count"
    pub page_ranges: Option<String>, // "1-3, 5, 7-10"
    pub chunk_size: Option<usize>, // e.g. 2 pages per bundle
    pub output_prefix: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageReorderRequest {
    pub source: String,
    pub destination: String,
    pub pages: Vec<PageSpec>,
}

pub struct PdfEngine;

impl PdfEngine {
    /// Inspects PDF metadata and total page count
    pub fn get_info(path: &Path) -> Result<PdfInfo, String> {
        let doc = Document::load(path)
            .map_err(|e| format!("Failed to read PDF document ({}): {}", path.display(), e))?;

        let page_count = {
            let pages = doc.get_pages();
            if pages.is_empty() {
                doc.page_iter().count()
            } else {
                pages.len()
            }
        };
        let version = doc.version.clone();
        let metadata = fs::metadata(path).map_err(|e| e.to_string())?;

        let mut title = None;
        let mut author = None;

        let info_dict = if let Ok(info_ref) = doc.trailer.get(b"Info").and_then(|o| o.as_reference()) {
            doc.get_object(info_ref).ok().and_then(|o| o.as_dict().ok())
        } else {
            doc.trailer.get(b"Info").ok().and_then(|o| o.as_dict().ok())
        };

        if let Some(info_dict) = info_dict {
            if let Ok(t) = info_dict.get(b"Title").and_then(|o| o.as_str()) {
                title = Some(String::from_utf8_lossy(t).to_string());
            }
            if let Ok(a) = info_dict.get(b"Author").and_then(|o| o.as_str()) {
                author = Some(String::from_utf8_lossy(a).to_string());
            }
        }

        Ok(PdfInfo {
            path: path.to_string_lossy().to_string(),
            page_count,
            version,
            title,
            author,
            file_size_bytes: metadata.len(),
        })
    }

    /// Merges multiple PDF files in order into a single PDF output
    pub fn merge(sources: &[PathBuf], destination: &Path, _add_bookmarks: bool) -> Result<(), String> {
        if sources.is_empty() {
            return Err("No source PDF files provided for merging".to_string());
        }

        if let Some(parent) = destination.parent() {
            let _ = fs::create_dir_all(parent);
        }

        let mut target_doc = Document::with_version("1.5");
        let mut documents_pages = BTreeMap::new();
        let mut documents_objects = BTreeMap::new();
        let mut doc_names = Vec::new();

        let mut max_id = 1;

        for (doc_idx, src) in sources.iter().enumerate() {
            let mut doc = Document::load(src)
                .map_err(|e| format!("Failed to load PDF {}: {}", src.display(), e))?;
            
            let file_stem = src.file_stem().unwrap_or_default().to_string_lossy().to_string();
            doc_names.push((doc_idx, file_stem));

            // Renumber objects to avoid collisions
            doc.renumber_objects_with(max_id);
            max_id = doc.max_id + 1;

            let mut pages = doc.get_pages();
            if pages.is_empty() {
                let mut pnum = 1;
                for pid in doc.page_iter() {
                    pages.insert(pnum, pid);
                    pnum += 1;
                }
            }

            documents_pages.insert(doc_idx, pages);
            documents_objects.insert(doc_idx, doc.objects);
        }

        let mut catalog_object: Option<(ObjectId, Object)> = None;
        let mut pages_object_id: Option<ObjectId> = None;
        let mut pages_kids: Vec<Object> = Vec::new();
        let mut page_count: i64 = 0;

        for (doc_idx, pages) in documents_pages {
            let objects = documents_objects.remove(&doc_idx).unwrap_or_default();
            for (obj_id, obj_val) in objects {
                if let Ok(dict) = obj_val.as_dict() {
                    let is_catalog = dict.get(b"Type").and_then(|t| t.as_name_str()).map_or(false, |n| n == "Catalog");
                    let is_pages = dict.get(b"Type").and_then(|t| t.as_name_str()).map_or(false, |n| n == "Pages");

                    if is_catalog {
                        if catalog_object.is_none() {
                            catalog_object = Some((obj_id, obj_val.clone()));
                        }
                        continue;
                    }

                    if is_pages {
                        if pages_object_id.is_none() {
                            pages_object_id = Some(obj_id);
                        }
                        continue;
                    }
                }

                target_doc.objects.insert(obj_id, obj_val);
            }

            for (_page_num, page_id) in pages {
                pages_kids.push(Object::Reference(page_id));
                page_count += 1;
            }
        }

        let pages_id = pages_object_id.unwrap_or((max_id, 0));
        max_id += 1;

        let pages_dict = dictionary! {
            "Type" => Object::Name(b"Pages".to_vec()),
            "Count" => Object::Integer(page_count),
            "Kids" => Object::Array(pages_kids),
        };

        target_doc.objects.insert(pages_id, Object::Dictionary(pages_dict.clone()));

        // Point all page objects to the new parent Pages node
        if let Ok(kids_array) = pages_dict.get(b"Kids").and_then(|k| k.as_array()) {
            for kid in kids_array {
                if let Ok(page_ref) = kid.as_reference() {
                    if let Ok(page_obj) = target_doc.get_object_mut(page_ref) {
                        if let Ok(pdict) = page_obj.as_dict_mut() {
                            pdict.set("Parent", Object::Reference(pages_id));
                        }
                    }
                }
            }
        }

        // Establish root catalog
        let catalog_id = (max_id, 0);
        let root_catalog = dictionary! {
            "Type" => Object::Name(b"Catalog".to_vec()),
            "Pages" => Object::Reference(pages_id),
        };

        target_doc.objects.insert(catalog_id, Object::Dictionary(root_catalog));
        target_doc.trailer.set("Root", catalog_id);
        target_doc.max_id = target_doc.objects.keys().map(|(id, _)| *id).max().unwrap_or(0);

        target_doc.save(destination)
            .map_err(|e| format!("Failed to save merged PDF to {}: {}", destination.display(), e))?;

        info!("Successfully merged {} PDFs into {}", sources.len(), destination.display());
        Ok(())
    }

    /// Splits a PDF into multiple documents according to ranges or page counts
    pub fn split(
        source: &Path,
        destination_dir: &Path,
        split_mode: &str,
        page_ranges_str: Option<&str>,
        chunk_size: Option<usize>,
        output_prefix: Option<&str>,
    ) -> Result<Vec<PathBuf>, String> {
        let doc = Document::load(source)
            .map_err(|e| format!("Failed to load PDF {}: {}", source.display(), e))?;

        let pages_map = doc.get_pages();
        let total_pages = pages_map.len();
        if total_pages == 0 {
            return Err("PDF has no pages to split".to_string());
        }

        fs::create_dir_all(destination_dir)
            .map_err(|e| format!("Failed to create output directory: {}", e))?;

        let stem = output_prefix
            .map(|s| s.to_string())
            .unwrap_or_else(|| source.file_stem().unwrap_or_default().to_string_lossy().to_string());

        let mut output_files = Vec::new();

        match split_mode {
            "each_page" => {
                for page_num in 1..=total_pages {
                    let out_path = destination_dir.join(format!("{}_page_{:03}.pdf", stem, page_num));
                    Self::extract_pages(&doc, &[page_num as u32], &out_path)?;
                    output_files.push(out_path);
                }
            }
            "page_count" => {
                let size = chunk_size.unwrap_or(1).max(1);
                let mut part = 1;
                let mut current_bundle = Vec::new();

                for page_num in 1..=total_pages {
                    current_bundle.push(page_num as u32);
                    if current_bundle.len() >= size || page_num == total_pages {
                        let out_path = destination_dir.join(format!("{}_part_{:02}.pdf", stem, part));
                        Self::extract_pages(&doc, &current_bundle, &out_path)?;
                        output_files.push(out_path);
                        current_bundle.clear();
                        part += 1;
                    }
                }
            }
            _ => {
                // "ranges" - parse comma-separated ranges e.g. "1-3, 5, 7-10"
                let ranges_spec = page_ranges_str.unwrap_or("1");
                let parsed_ranges = Self::parse_page_ranges(ranges_spec, total_pages)?;

                for (idx, page_group) in parsed_ranges.iter().enumerate() {
                    let out_path = destination_dir.join(format!("{}_range_{:02}.pdf", stem, idx + 1));
                    Self::extract_pages(&doc, page_group, &out_path)?;
                    output_files.push(out_path);
                }
            }
        }

        info!("Successfully split PDF {} into {} parts in {}", source.display(), output_files.len(), destination_dir.display());
        Ok(output_files)
    }

    /// Reorders pages, rotates specific pages, and outputs to target PDF
    pub fn reorder_and_rotate(
        source: &Path,
        pages_spec: &[PageSpec],
        destination: &Path,
    ) -> Result<(), String> {
        let mut doc = Document::load(source)
            .map_err(|e| format!("Failed to load PDF {}: {}", source.display(), e))?;

        let orig_pages = doc.get_pages();
        let total_pages = orig_pages.len();

        let mut ordered_page_nums = Vec::new();
        let mut rotations = BTreeMap::new();

        for spec in pages_spec {
            if spec.page_num >= 1 && (spec.page_num as usize) <= total_pages {
                ordered_page_nums.push(spec.page_num);
                if spec.rotation % 360 != 0 {
                    rotations.insert(spec.page_num, spec.rotation);
                }
            }
        }

        if ordered_page_nums.is_empty() {
            return Err("No valid pages specified in reorder request".to_string());
        }

        if let Some(parent) = destination.parent() {
            let _ = fs::create_dir_all(parent);
        }

        // Apply rotation to page objects
        for (&pnum, &rot) in &rotations {
            if let Some(&page_id) = orig_pages.get(&pnum) {
                if let Ok(page_obj) = doc.get_object_mut(page_id) {
                    if let Ok(dict) = page_obj.as_dict_mut() {
                        let existing_rot = dict.get(b"Rotate").and_then(|r| r.as_i64()).unwrap_or(0);
                        let new_rot = (existing_rot + rot as i64).rem_euclid(360);
                        dict.set("Rotate", Object::Integer(new_rot));
                    }
                }
            }
        }

        Self::extract_pages(&doc, &ordered_page_nums, destination)?;
        info!("Successfully reordered/rotated PDF to {}", destination.display());
        Ok(())
    }

    /// Extracts a subset of pages from a document into a new destination PDF
    fn extract_pages(source_doc: &Document, page_numbers: &[u32], destination: &Path) -> Result<(), String> {
        let mut new_doc = source_doc.clone();
        let all_pages = new_doc.get_pages();
        let target_set: std::collections::HashSet<u32> = page_numbers.iter().copied().collect();

        // Identify pages to delete
        let mut pages_to_delete = Vec::new();
        for page_num in all_pages.keys() {
            if !target_set.contains(page_num) {
                pages_to_delete.push(*page_num);
            }
        }

        new_doc.delete_pages(&pages_to_delete);

        // If ordering changed, reorganize the Kids array in Pages dict
        let remaining_pages = new_doc.get_pages();
        let mut reordered_kids = Vec::new();

        for &requested_page in page_numbers {
            if let Some(&page_id) = remaining_pages.get(&requested_page) {
                reordered_kids.push(Object::Reference(page_id));
            }
        }

        if !reordered_kids.is_empty() {
            // Find Pages dictionary and update Kids & Count
            for (_id, obj) in new_doc.objects.iter_mut() {
                if let Ok(dict) = obj.as_dict_mut() {
                    let is_pages = dict.get(b"Type").and_then(|t| t.as_name()).map_or(false, |n| n == b"Pages");
                    if is_pages {
                        dict.set("Count", Object::Integer(reordered_kids.len() as i64));
                        dict.set("Kids", Object::Array(reordered_kids.clone()));
                        break;
                    }
                }
            }
        }

        new_doc.save(destination)
            .map_err(|e| format!("Failed to save extracted PDF {}: {}", destination.display(), e))?;

        Ok(())
    }

    /// Parses string like "1-3, 5, 8-10" into groups of page numbers
    pub fn parse_page_ranges(spec: &str, max_pages: usize) -> Result<Vec<Vec<u32>>, String> {
        let mut result = Vec::new();

        for chunk in spec.split(',') {
            let trimmed = chunk.trim();
            if trimmed.is_empty() {
                continue;
            }

            if let Some((start_s, end_s)) = trimmed.split_once('-') {
                let start: usize = start_s.trim().parse().map_err(|_| format!("Invalid range start: '{}'", start_s))?;
                let end: usize = end_s.trim().parse().map_err(|_| format!("Invalid range end: '{}'", end_s))?;
                if start == 0 || end == 0 || start > end {
                    return Err(format!("Invalid page range: {}-{}", start, end));
                }
                let valid_end = end.min(max_pages);
                let pages: Vec<u32> = (start..=valid_end).map(|p| p as u32).collect();
                if !pages.is_empty() {
                    result.push(pages);
                }
            } else {
                let single: usize = trimmed.parse().map_err(|_| format!("Invalid page number: '{}'", trimmed))?;
                if single >= 1 && single <= max_pages {
                    result.push(vec![single as u32]);
                }
            }
        }

        if result.is_empty() {
            return Err("No valid page ranges found".to_string());
        }

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_dummy_pdf(page_count: u32, path: &Path) {
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let mut kids = Vec::new();

        for _ in 0..page_count {
            let page_id = doc.add_object(dictionary! {
                "Type" => "Page",
                "Parent" => pages_id,
                "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            });
            kids.push(page_id.into());
        }

        let pages_dict = dictionary! {
            "Type" => "Pages",
            "Kids" => kids,
            "Count" => page_count as i64,
        };
        doc.objects.insert(pages_id, Object::Dictionary(pages_dict));

        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        doc.trailer.set("Root", catalog_id);

        doc.save(path).expect("Failed to write dummy PDF");
    }

    #[test]
    fn test_pdf_info_and_split() {
        let tmp = tempfile::tempdir().unwrap();
        let pdf_path = tmp.path().join("test_sample.pdf");
        create_dummy_pdf(5, &pdf_path);

        let info = PdfEngine::get_info(&pdf_path).expect("Failed to get info");
        assert_eq!(info.page_count, 5);

        // Test split each page
        let out_dir = tmp.path().join("split_out");
        let parts = PdfEngine::split(&pdf_path, &out_dir, "each_page", None, None, None).expect("Failed to split");
        assert_eq!(parts.len(), 5);

        // Test split by ranges "1-2, 4-5"
        let range_dir = tmp.path().join("range_out");
        let range_parts = PdfEngine::split(&pdf_path, &range_dir, "ranges", Some("1-2, 4-5"), None, None).expect("Failed range split");
        assert_eq!(range_parts.len(), 2);
    }

    #[test]
    fn test_pdf_merge() {
        let tmp = tempfile::tempdir().unwrap();
        let doc1 = tmp.path().join("doc1.pdf");
        let doc2 = tmp.path().join("doc2.pdf");
        let merged = tmp.path().join("merged.pdf");

        create_dummy_pdf(2, &doc1);
        create_dummy_pdf(3, &doc2);

        PdfEngine::merge(&[doc1, doc2], &merged, true).expect("Merge failed");

        let info = PdfEngine::get_info(&merged).expect("Failed to get merged info");
        assert_eq!(info.page_count, 5);
    }
}
