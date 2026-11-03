//! Guards the move away from inline event handlers (#152).
//!
//! Inline handlers (`onclick="..."` and friends) force the main app's content
//! security policy to allow inline script. Each conversion batch lowers the
//! ceilings below; new inline handlers make this test fail.
//! Convert them with `data-on-click` (see the BUTTON WIRING section at the top of `frontend/app.js`).

use regex::Regex;
use std::fs;

/// (file, highest allowed number of inline handlers)
const CEILINGS: &[(&str, usize)] = &[
    ("frontend/index.html", 1164),
    ("frontend/app.js", 268),
];

fn count_inline_handlers(text: &str) -> usize {
    // `onclick="`, `onchange='`, and `onclick=\"` inside JS strings; whitespace before `on`
    // keeps names like `data-on-click=` and `el.onclick =` out of the count.
    let re = Regex::new(r#"(?i)\son[a-z]+\s*=\s*\\?["']"#).unwrap();
    re.find_iter(text).count()
}

#[test]
fn inline_handler_count_does_not_grow() {
    let root = env!("CARGO_MANIFEST_DIR");
    let mut failures = Vec::new();
    for (file, ceiling) in CEILINGS {
        let text = fs::read_to_string(format!("{root}/{file}"))
            .unwrap_or_else(|e| panic!("cannot read {file}: {e}"));
        let count = count_inline_handlers(&text);
        if count > *ceiling {
            failures.push(format!(
                "{file}: {count} inline handlers, ceiling is {ceiling} (use data-on-click instead)"
            ));
        } else if count < *ceiling {
            println!("{file}: {count} inline handlers; ceiling {ceiling} can be lowered");
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn counter_ignores_data_attributes_and_properties() {
    assert_eq!(count_inline_handlers(r#"<b data-on-click="x">"#), 0);
    assert_eq!(count_inline_handlers("el.onclick = fn;"), 0);
    assert_eq!(count_inline_handlers(r#"<b onclick="x()">"#), 1);
    assert_eq!(count_inline_handlers(r#"html += '<b onchange=\"x()\">';"#), 1);
}
