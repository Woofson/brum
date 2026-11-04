//! The browser's login cookie.
//!
//! The login lives only in an `HttpOnly` cookie, so scripts on the page can never read it.
//! Cookies are shared by every port on the same host, so each Brum names its cookie after the
//! port the browser uses (`brum_session_3140`); two Brums on one computer then keep separate
//! logins. The older shared name `cd_token` is still read so existing logins keep working.

use axum::http::{header, HeaderMap, HeaderValue};

/// Older, shared cookie name (read, and cleared when a new login is set)
pub const LEGACY_COOKIE: &str = "cd_token";

/// Request header the Brum page sends to say "keep the login in the cookie, not in the reply"
pub const COOKIE_SESSION_HEADER: &str = "x-brum-session";

/// The port the browser used to reach this Brum, from the `Host` header
fn browser_port(headers: &HeaderMap) -> String {
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("").trim();
    // "[::1]:3140", "example.com:8443", "example.com"
    let after_bracket = host.rsplit_once(']').map(|(_, rest)| rest).unwrap_or(host);
    match after_bracket.rsplit_once(':') {
        Some((_, port)) if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => port.to_string(),
        _ => "default".to_string(),
    }
}

/// This Brum's login cookie name, as seen by the browser
pub fn cookie_name(headers: &HeaderMap) -> String {
    format!("brum_session_{}", browser_port(headers))
}

/// Whether the Brum page asked for a cookie-only login (no login in the reply body)
pub fn wants_cookie_only(headers: &HeaderMap) -> bool {
    headers.get(COOKIE_SESSION_HEADER)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("cookie"))
}

/// `Set-Cookie` value that stores the login
pub fn set_cookie_value(headers: &HeaderMap, token: &str, max_age_secs: u64, secure: bool) -> String {
    format!(
        "{}={}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}{}",
        cookie_name(headers),
        token,
        max_age_secs,
        if secure { "; Secure" } else { "" }
    )
}

/// `Set-Cookie` value that removes a cookie
fn clear_cookie_value(name: &str) -> String {
    format!("{}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0; Expires=Thu, 01 Jan 1970 00:00:00 GMT", name)
}

/// Adds the login cookie to a response, and removes the older shared cookie
pub fn attach_login_cookie(resp_headers: &mut HeaderMap, req_headers: &HeaderMap, token: &str, max_age_secs: u64, secure: bool) {
    if let Ok(hv) = HeaderValue::from_str(&set_cookie_value(req_headers, token, max_age_secs, secure)) {
        resp_headers.append(header::SET_COOKIE, hv);
    }
    if let Ok(hv) = HeaderValue::from_str(&clear_cookie_value(LEGACY_COOKIE)) {
        resp_headers.append(header::SET_COOKIE, hv);
    }
}

/// Removes the login cookie (and the older shared one) from the browser
pub fn attach_logout_cookies(resp_headers: &mut HeaderMap, req_headers: &HeaderMap) {
    for name in [cookie_name(req_headers), LEGACY_COOKIE.to_string()] {
        if let Ok(hv) = HeaderValue::from_str(&clear_cookie_value(&name)) {
            resp_headers.append(header::SET_COOKIE, hv);
        }
    }
}

/// Login tokens found in the request's cookies: this Brum's own cookie first, then the older names
pub fn tokens_from_cookies(headers: &HeaderMap) -> Vec<String> {
    let own = format!("{}=", cookie_name(headers));
    let mut own_tokens = Vec::new();
    let mut legacy_tokens = Vec::new();
    for value in headers.get_all(header::COOKIE).iter().filter_map(|v| v.to_str().ok()) {
        for pair in value.split(';') {
            let pair = pair.trim();
            let (bucket, tok) = if let Some(t) = pair.strip_prefix(own.as_str()) {
                (&mut own_tokens, t)
            } else if let Some(t) = pair.strip_prefix("cd_token=").or_else(|| pair.strip_prefix("token=")) {
                (&mut legacy_tokens, t)
            } else {
                continue;
            };
            let tok = tok.trim();
            if tok.is_empty() || tok == "undefined" || tok == "null" {
                continue;
            }
            let decoded = crate::server::middleware::percent_decode(tok);
            if decoded != tok {
                bucket.push(decoded);
            }
            bucket.push(tok.to_string());
        }
    }
    own_tokens.extend(legacy_tokens);
    own_tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut m = HeaderMap::new();
        for (k, v) in pairs {
            m.append(axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(), HeaderValue::from_str(v).unwrap());
        }
        m
    }

    #[test]
    fn cookie_name_follows_the_browser_port() {
        assert_eq!(cookie_name(&h(&[("host", "127.0.0.1:3140")])), "brum_session_3140");
        assert_eq!(cookie_name(&h(&[("host", "[::1]:8443")])), "brum_session_8443");
        assert_eq!(cookie_name(&h(&[("host", "brum.example.com")])), "brum_session_default");
        assert_eq!(cookie_name(&h(&[("host", "[::1]")])), "brum_session_default");
    }

    #[test]
    fn own_cookie_comes_before_legacy_and_other_ports_are_ignored() {
        let headers = h(&[("host", "localhost:3140"), ("cookie", "cd_token=old; brum_session_3195=other; brum_session_3140=mine")]);
        assert_eq!(tokens_from_cookies(&headers), vec!["mine".to_string(), "old".to_string()]);
    }

    #[test]
    fn login_cookie_is_http_only() {
        let v = set_cookie_value(&h(&[("host", "localhost:3140")]), "abc", 60, true);
        assert!(v.starts_with("brum_session_3140=abc;"));
        assert!(v.contains("HttpOnly") && v.contains("SameSite=Lax") && v.contains("Secure"));
    }
}
