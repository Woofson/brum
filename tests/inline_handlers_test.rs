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
    ("frontend/index.html", 0),
    ("frontend/app.js", 0),
];

fn count_inline_handlers(text: &str) -> usize {
    // `onclick="`, `onchange='`, and `onclick=\"` inside JS strings; whitespace before `on`
    // keeps names like `data-on-click=` and `el.onclick =` out of the count.
    let re = Regex::new(r#"(?i)\son[a-z]+\s*=\s*\\?["']"#).unwrap();
    // handlers written from code: el.setAttribute('onclick', ...)
    let set_attr = Regex::new(r#"(?i)setAttribute\(\s*['"]on[a-z]+['"]"#).unwrap();
    re.find_iter(text).count() + set_attr.find_iter(text).count()
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

fn read(file: &str) -> String {
    let root = env!("CARGO_MANIFEST_DIR");
    fs::read_to_string(format!("{root}/{file}")).unwrap_or_else(|e| panic!("cannot read {file}: {e}"))
}

/// Quoted names inside a JS array literal that starts with `start` in app.js
fn js_list(app: &str, start: &str) -> Vec<String> {
    let a = app.find(start).unwrap_or_else(|| panic!("{start} not found in app.js"));
    // a list ends at `];` (array) or `]);` (Set), whichever comes first
    let rest = &app[a..];
    let end = [rest.find("];"), rest.find("]);")].into_iter().flatten().min().expect("unterminated list");
    let b = a + end;
    Regex::new(r"'(\w+)'").unwrap()
        .captures_iter(&app[a..b]).map(|c| c[1].to_string()).collect()
}

/// Every event named in the markup must be one the connector listens for, or the
/// converted handler silently never runs.
#[test]
fn every_wired_event_type_is_listened_for() {
    let app = read("frontend/app.js");
    let page = read("frontend/index.html");
    let events = js_list(&app, "const BRUM_ACTION_EVENTS = [");
    let re = Regex::new(r"data-(?:on-([a-z]+)=|([a-z]+)-(?:args|steps|after|stop|prevent|key|unless|self)\b)").unwrap();
    let mut missing = Vec::new();
    for text in [&app, &page] {
        for c in re.captures_iter(text) {
            let t = c.get(1).or(c.get(2)).unwrap().as_str();
            // `data-error-args` etc. name the event; skip unrelated data attributes
            if !events.iter().any(|e| e == t) && !missing.contains(&t.to_string()) && is_event_like(t) {
                missing.push(t.to_string());
            }
        }
    }
    assert!(missing.is_empty(), "add these to BRUM_ACTION_EVENTS: {missing:?}");
}

fn is_event_like(t: &str) -> bool {
    // data-click-args, data-dragover-prevent ... but not e.g. data-fallback-used
    const KNOWN_NON_EVENTS: &[&str] = &["fallback"];
    !KNOWN_NON_EVENTS.contains(&t)
}

/// Every action named in the markup must be on the allowed list, or clicking it does nothing.
#[test]
fn every_named_action_is_allowed() {
    let app = read("frontend/app.js");
    let page = read("frontend/index.html");
    let allowed = js_list(&app, "const BRUM_ACTIONS = new Set([");
    let mut names = Vec::new();
    let attr = Regex::new(r#"data-(?:on-[a-z]+|[a-z]+-after)="(\w+)""#).unwrap();
    // first name of each step: ['fn', ...] in actionSteps(...), ["fn", ...] in static JSON
    let step = Regex::new(r#"(?:actionSteps\(|\],\s*|\[\[)\s*\[?\s*['"](\w+)['"]"#).unwrap();
    for text in [&app, &page] {
        names.extend(attr.captures_iter(text).map(|c| c[1].to_string()));
    }
    for c in Regex::new(r"actionSteps\(([^\n]*)\)").unwrap().captures_iter(&app) {
        names.extend(step.captures_iter(&format!("actionSteps({}", &c[1])).map(|s| s[1].to_string()));
    }
    for c in Regex::new(r"data-[a-z]+-steps='([^']*)'").unwrap().captures_iter(&page) {
        let json = c[1].replace("&#39;", "'").replace("&amp;", "&");
        let steps: Vec<Vec<serde_json::Value>> = serde_json::from_str(&json).expect("bad static steps JSON");
        names.extend(steps.iter().filter_map(|s| s.first()?.as_str().map(String::from)));
    }
    let mut missing: Vec<String> = names.into_iter().filter(|n| !allowed.contains(n)).collect();
    missing.sort();
    missing.dedup();
    assert!(missing.is_empty(), "add these to BRUM_ACTIONS: {missing:?}");
}

/// The connector finds actions on `window`, which holds top-level `function` declarations but
/// not `const`/`let` aliases. Every allowed action must be a real function declaration.
#[test]
fn every_allowed_action_is_a_global_function() {
    let app = read("frontend/app.js");
    let allowed = js_list(&app, "const BRUM_ACTIONS = new Set([");
    let missing: Vec<&String> = allowed
        .iter()
        .filter(|n| {
            let decl = Regex::new(&format!(r"(?m)^(?:async )?function {}\(", regex::escape(n))).unwrap();
            !decl.is_match(&app)
        })
        .collect();
    assert!(missing.is_empty(), "not top-level function declarations (aliases or missing): {missing:?}");
}

#[test]
fn counter_ignores_data_attributes_and_properties() {
    assert_eq!(count_inline_handlers(r#"<b data-on-click="x">"#), 0);
    assert_eq!(count_inline_handlers("el.onclick = fn;"), 0);
    assert_eq!(count_inline_handlers(r#"<b onclick="x()">"#), 1);
    assert_eq!(count_inline_handlers(r#"html += '<b onchange=\"x()\">';"#), 1);
    assert_eq!(count_inline_handlers(r#"btn.setAttribute('onclick', `x()`);"#), 1);
    assert_eq!(count_inline_handlers(r#"btn.setAttribute('data-on-click', 'x');"#), 0);
}
