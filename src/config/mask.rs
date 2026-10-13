// ==============================================================================
// Brum - Configuration Secrets Masking & Non-Destructive Merge Engine
// Protects sensitive cryptographic keys and passwords in web-facing config views.
// ==============================================================================

use toml_edit::{DocumentMut, Formatted, Item, Value};

/// Fixed sentinel string used to mask sensitive credentials in the web UI.
pub const MASKED_SECRET_SENTINEL: &str = "********";

/// Checks if a TOML key name represents a sensitive credential that should be masked.
pub fn is_sensitive_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    lower == "jwt_secret"
        || lower == "client_secret"
        || lower == "default_admin_pass"
        || lower.contains("secret")
        || lower.contains("password")
        || lower.contains("passphrase")
        || lower.contains("api_key")
        || lower.contains("private_key")
        || lower == "token"
        || lower == "auth_token"
}

/// Redacts sensitive values in a TOML string, replacing non-empty values with `********`.
/// Preserves all TOML comments, whitespace, indentation, and structure.
pub fn redact_sensitive_config(raw_toml: &str) -> Result<String, String> {
    let mut doc: DocumentMut = raw_toml
        .parse()
        .map_err(|e| format!("Failed to parse TOML for redaction: {}", e))?;

    redact_table_items(doc.as_table_mut());
    Ok(doc.to_string())
}

fn redact_table_items(table: &mut toml_edit::Table) {
    for (k, item) in table.iter_mut() {
        let key_name = k.get().to_string();
        redact_item(&key_name, item);
    }
}

fn redact_item(key: &str, item: &mut Item) {
    match item {
        Item::Value(val) => {
            if is_sensitive_key(key) {
                if let Value::String(s) = val {
                    if !s.value().trim().is_empty() && s.value() != MASKED_SECRET_SENTINEL {
                        *s = Formatted::new(MASKED_SECRET_SENTINEL.to_string());
                    }
                }
            } else if let Value::InlineTable(inline) = val {
                for (k, v) in inline.iter_mut() {
                    if is_sensitive_key(k.get()) {
                        if let Value::String(s) = v {
                            if !s.value().trim().is_empty() && s.value() != MASKED_SECRET_SENTINEL {
                                *s = Formatted::new(MASKED_SECRET_SENTINEL.to_string());
                            }
                        }
                    }
                }
            }
        }
        Item::Table(t) => {
            redact_table_items(t);
        }
        Item::ArrayOfTables(arr) => {
            for t in arr.iter_mut() {
                redact_table_items(t);
            }
        }
        _ => {}
    }
}

#[derive(Debug, Clone)]
enum PathSegment {
    Key(String),
    Index(usize),
}

/// Merges incoming TOML (which may contain `********` placeholders) with the existing
/// on-disk configuration. If an incoming value is `********`, the pre-existing secret
/// value from disk is restored. If a user provided a new real value, the new value is preserved.
pub fn merge_masked_config(incoming_toml: &str, existing_toml: &str) -> Result<String, String> {
    let mut incoming_doc: DocumentMut = incoming_toml
        .parse()
        .map_err(|e| format!("Invalid incoming TOML: {}", e))?;

    let existing_doc: DocumentMut = match existing_toml.parse() {
        Ok(d) => d,
        Err(_) => return Ok(incoming_doc.to_string()),
    };

    let mut path = Vec::new();
    merge_table_items(incoming_doc.as_table_mut(), &existing_doc, &mut path);

    Ok(incoming_doc.to_string())
}

fn merge_table_items(
    table: &mut toml_edit::Table,
    existing_doc: &DocumentMut,
    path: &mut Vec<PathSegment>,
) {
    for (k, item) in table.iter_mut() {
        let key_name = k.get().to_string();
        path.push(PathSegment::Key(key_name.clone()));
        merge_item(&key_name, item, existing_doc, path);
        path.pop();
    }
}

fn merge_item(
    key: &str,
    item: &mut Item,
    existing_doc: &DocumentMut,
    path: &mut Vec<PathSegment>,
) {
    match item {
        Item::Value(val) => {
            if is_sensitive_key(key) {
                if let Value::String(s) = val {
                    if s.value() == MASKED_SECRET_SENTINEL {
                        if let Some(original_val) = lookup_value_at_path(existing_doc, path) {
                            *s = Formatted::new(original_val);
                        }
                    }
                }
            } else if let Value::InlineTable(inline) = val {
                for (k, v) in inline.iter_mut() {
                    let sub_key = k.get().to_string();
                    if is_sensitive_key(&sub_key) {
                        if let Value::String(s) = v {
                            if s.value() == MASKED_SECRET_SENTINEL {
                                path.push(PathSegment::Key(sub_key.clone()));
                                if let Some(original_val) = lookup_value_at_path(existing_doc, path) {
                                    *s = Formatted::new(original_val);
                                }
                                path.pop();
                            }
                        }
                    }
                }
            }
        }
        Item::Table(t) => {
            merge_table_items(t, existing_doc, path);
        }
        Item::ArrayOfTables(arr) => {
            for (idx, t) in arr.iter_mut().enumerate() {
                path.push(PathSegment::Index(idx));
                merge_table_items(t, existing_doc, path);
                path.pop();
            }
        }
        _ => {}
    }
}

fn lookup_value_at_path(doc: &DocumentMut, path: &[PathSegment]) -> Option<String> {
    if path.is_empty() {
        return None;
    }

    let mut current: &Item = doc.as_item();
    for (i, segment) in path.iter().enumerate() {
        let is_last = i == path.len() - 1;
        match segment {
            PathSegment::Key(k) => match current {
                Item::Table(t) => {
                    let next = t.get(k)?;
                    if is_last {
                        return next.as_value().and_then(|v| v.as_str()).map(|s| s.to_string());
                    }
                    current = next;
                }
                Item::Value(Value::InlineTable(it)) => {
                    let next = it.get(k)?;
                    if is_last {
                        return next.as_str().map(|s| s.to_string());
                    }
                    return None;
                }
                _ => return None,
            },
            PathSegment::Index(idx) => match current {
                Item::ArrayOfTables(arr) => {
                    let t = arr.get(*idx)?;
                    if is_last {
                        return None;
                    }
                    // For the next iteration, we treat the Table as current
                    // Since arr.get() returns &Table, we need to inspect the remaining path
                    return lookup_in_table(t, &path[i + 1..]);
                }
                _ => return None,
            },
        }
    }

    None
}

fn lookup_in_table(table: &toml_edit::Table, remaining_path: &[PathSegment]) -> Option<String> {
    if remaining_path.is_empty() {
        return None;
    }

    let mut current_table = table;
    for (i, segment) in remaining_path.iter().enumerate() {
        let is_last = i == remaining_path.len() - 1;
        match segment {
            PathSegment::Key(k) => {
                let next = current_table.get(k)?;
                if is_last {
                    return next.as_value().and_then(|v| v.as_str()).map(|s| s.to_string());
                }
                match next {
                    Item::Table(t) => {
                        current_table = t;
                    }
                    _ => return None,
                }
            }
            PathSegment::Index(_) => return None,
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_sensitive_key() {
        assert!(is_sensitive_key("jwt_secret"));
        assert!(is_sensitive_key("client_secret"));
        assert!(is_sensitive_key("default_admin_pass"));
        assert!(is_sensitive_key("database_password"));
        assert!(is_sensitive_key("vault_passphrase"));
        assert!(is_sensitive_key("token"));
        assert!(is_sensitive_key("auth_token"));
        assert!(is_sensitive_key("api_key"));
        assert!(is_sensitive_key("ssh_private_key"));

        assert!(!is_sensitive_key("server_name"));
        assert!(!is_sensitive_key("port"));
        assert!(!is_sensitive_key("allow_guest"));
        assert!(!is_sensitive_key("session_duration_hours"));
        assert!(!is_sensitive_key("allow_web_config_edit"));
    }

    #[test]
    fn test_redact_sensitive_config() {
        let input = r#"
# Server configuration
[server]
port = 3140
jwt_secret = "super_duper_secret_key_123"

[auth]
default_admin_user = "admin"
default_admin_pass = "topsecret"

[auth.oidc]
client_id = "brum-client"
client_secret = "oidc-secret-456"

[[fleet.nodes]]
name = "node-1"
token = "fleet-token-789"
"#;

        let redacted = redact_sensitive_config(input).unwrap();

        assert!(!redacted.contains("super_duper_secret_key_123"));
        assert!(!redacted.contains("topsecret"));
        assert!(!redacted.contains("oidc-secret-456"));
        assert!(!redacted.contains("fleet-token-789"));

        assert!(redacted.contains("jwt_secret = \"********\""));
        assert!(redacted.contains("default_admin_pass = \"********\""));
        assert!(redacted.contains("client_secret = \"********\""));
        assert!(redacted.contains("token = \"********\""));

        // Non-sensitive fields and comments preserved
        assert!(redacted.contains("# Server configuration"));
        assert!(redacted.contains("port = 3140"));
        assert!(redacted.contains("default_admin_user = \"admin\""));
        assert!(redacted.contains("client_id = \"brum-client\""));
        assert!(redacted.contains("name = \"node-1\""));
    }

    #[test]
    fn test_merge_masked_config_retains_existing_secret() {
        let original_disk = r#"
[server]
port = 3140
jwt_secret = "real_production_jwt_secret_999"

[auth]
default_admin_pass = "admin_pw_111"
"#;

        // User edited the port to 3141 in web UI, left secrets as ********
        let web_edited = r#"
[server]
port = 3141
jwt_secret = "********"

[auth]
default_admin_pass = "********"
"#;

        let merged = merge_masked_config(web_edited, original_disk).unwrap();

        assert!(merged.contains("port = 3141"));
        assert!(merged.contains("jwt_secret = \"real_production_jwt_secret_999\""));
        assert!(merged.contains("default_admin_pass = \"admin_pw_111\""));
        assert!(!merged.contains("********"));
    }

    #[test]
    fn test_merge_masked_config_allows_secret_rotation() {
        let original_disk = r#"
[server]
jwt_secret = "old_secret_123"
"#;

        // User intentionally changed jwt_secret in web UI to a new secret
        let web_edited = r#"
[server]
jwt_secret = "new_rotated_secret_456"
"#;

        let merged = merge_masked_config(web_edited, original_disk).unwrap();
        assert!(merged.contains("jwt_secret = \"new_rotated_secret_456\""));
        assert!(!merged.contains("old_secret_123"));
    }
}
