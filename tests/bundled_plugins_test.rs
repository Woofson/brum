//! The packed add-ons Brum ships (`plugins/*.grr`) must match their source folders.
//! When this fails, repack the add-on from `plugins/<id>/` into `plugins/<id>.grr`.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::Path;

fn folder_files(dir: &Path, base: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
    for entry in fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            folder_files(&path, base, out);
        } else {
            let rel = path.strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/");
            out.insert(rel, fs::read(&path).unwrap());
        }
    }
}

#[test]
fn packed_plugins_match_their_source() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("plugins");
    let mut checked = 0;
    for entry in fs::read_dir(&root).unwrap().flatten() {
        let grr = entry.path();
        if grr.extension().and_then(|e| e.to_str()) != Some("grr") {
            continue;
        }
        let src = root.join(grr.file_stem().unwrap());
        assert!(src.is_dir(), "{} has no source folder", grr.display());

        let mut expected = BTreeMap::new();
        folder_files(&src, &src, &mut expected);

        let mut packed = BTreeMap::new();
        let mut zip = zip::ZipArchive::new(fs::File::open(&grr).unwrap()).unwrap();
        for i in 0..zip.len() {
            let mut f = zip.by_index(i).unwrap();
            if f.is_dir() {
                continue;
            }
            let name = f.name().unwrap().to_string();
            let mut buf = Vec::new();
            f.read_to_end(&mut buf).unwrap();
            packed.insert(name, buf);
        }

        let differs: Vec<&String> = expected
            .keys()
            .chain(packed.keys())
            .filter(|k| expected.get(*k) != packed.get(*k))
            .collect();
        assert!(differs.is_empty(), "{} is out of date with plugins/{}/: {:?}", grr.display(), src.file_name().unwrap().to_string_lossy(), differs);
        checked += 1;
    }
    assert!(checked >= 2, "expected the bundled add-ons");
}

/// Chewtoys run in a sandbox without their own origin: browser storage and cookies throw there
/// and stop the script. Bundled Chewtoys must use the host bridge instead (`storage.*`).
#[test]
fn bundled_plugins_do_not_use_sandbox_blocked_storage() {
    let roots = [Path::new(env!("CARGO_MANIFEST_DIR")).join("plugins"), Path::new(env!("CARGO_MANIFEST_DIR")).join("examples")];
    let blocked = regex::Regex::new(r"\b(localStorage|sessionStorage|indexedDB)\s*[.\[]|document\.cookie|window\.parent\.(Brum|App|document)\b").unwrap();
    let mut found = Vec::new();
    for entry in roots.iter().flat_map(|root| fs::read_dir(root).unwrap().flatten()) {
        if !entry.path().is_dir() {
            continue;
        }
        let mut files = BTreeMap::new();
        folder_files(&entry.path(), &entry.path(), &mut files);
        for (name, bytes) in files {
            if !(name.ends_with(".js") || name.ends_with(".html")) {
                continue;
            }
            let text = String::from_utf8_lossy(&bytes);
            for (i, line) in text.lines().enumerate() {
                if blocked.is_match(line) {
                    found.push(format!("{}/{}:{}", entry.file_name().to_string_lossy(), name, i + 1));
                }
            }
        }
    }
    assert!(found.is_empty(), "sandbox-blocked storage used in bundled Chewtoys: {found:?}");
}
