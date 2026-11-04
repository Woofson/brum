//! The browser's login lives only in an HttpOnly cookie (#155): page scripts never see it,
//! and everything works with the cookie alone.

use brum::config::AppConfig;
use brum::start_background_server;
use serde_json::{json, Value};
use tempfile::tempdir;

struct Srv {
    base: String,
    port: u16,
    _dir: tempfile::TempDir,
    home: String,
}

async fn start() -> Srv {
    let dir = tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("hello.txt"), "hello").unwrap();
    let mut config = AppConfig::default();
    config.server.host = "127.0.0.1".to_string();
    config.server.port = 0;
    config.server.database_path = dir.path().join("brum_test.db").to_string_lossy().to_string();
    config.server.enable_auth = true;
    config.storage.allow_entire_system = true;
    config.auth.default_admin_user = "admin".to_string();
    config.auth.default_admin_pass = "AdminSecretPass123!".to_string();
    config.plugins.directory = dir.path().join("plugins").to_string_lossy().to_string();
    config.plugins.user_directory = dir.path().join("user_plugins").to_string_lossy().to_string();
    let (port, _) = start_background_server(config).await.unwrap();
    Srv { base: format!("http://127.0.0.1:{port}"), port, home: home.to_string_lossy().to_string(), _dir: dir }
}

fn plain_client() -> reqwest::Client {
    reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap()
}

/// Logs in the way the Brum page does; returns the reply body and the login cookie ("name=value")
async fn page_login(srv: &Srv) -> (Value, String, Vec<String>) {
    let resp = plain_client().post(format!("{}/api/auth/login", srv.base))
        .header("X-Brum-Session", "cookie")
        .json(&json!({ "username": "admin", "password": "AdminSecretPass123!" }))
        .send().await.unwrap();
    assert!(resp.status().is_success());
    let set_cookies: Vec<String> = resp.headers().get_all("set-cookie").iter()
        .map(|v| v.to_str().unwrap().to_string()).collect();
    let own = format!("brum_session_{}=", srv.port);
    let cookie = set_cookies.iter().find(|c| c.starts_with(&own)).expect("login cookie").clone();
    let pair = cookie.split(';').next().unwrap().to_string();
    (resp.json().await.unwrap(), pair, set_cookies)
}

#[tokio::test]
async fn page_login_keeps_the_login_out_of_reach_of_scripts() {
    let srv = start().await;
    let (body, _pair, set_cookies) = page_login(&srv).await;
    assert!(body.get("token").is_none(), "the page must not receive the login in the reply: {body}");
    assert_eq!(body["user"]["username"], "admin");
    let own = set_cookies.iter().find(|c| c.starts_with(&format!("brum_session_{}=", srv.port))).unwrap();
    assert!(own.contains("HttpOnly"), "login cookie must be HttpOnly: {own}");
    assert!(own.contains("SameSite=Lax"));
    assert!(set_cookies.iter().any(|c| c.starts_with("cd_token=;") && c.contains("Max-Age=0")), "old shared cookie is cleared");

    // Scripts and tools that log in themselves still get the login
    let body: Value = plain_client().post(format!("{}/api/auth/login", srv.base))
        .json(&json!({ "username": "admin", "password": "AdminSecretPass123!" }))
        .send().await.unwrap().json().await.unwrap();
    assert!(body["token"].as_str().is_some_and(|t| !t.is_empty()));
}

#[tokio::test]
async fn everything_works_with_the_cookie_alone() {
    let srv = start().await;
    let (_, pair, _) = page_login(&srv).await;
    let c = plain_client();

    // Same-origin page requests carry only the cookie
    let get = |path: String| c.get(format!("{}{}", srv.base, path)).header("cookie", &pair).header("sec-fetch-site", "same-origin");
    assert_eq!(get("/api/auth/me".into()).send().await.unwrap().status().as_u16(), 200);
    let list = get(format!("/api/fs/list?path={}", srv.home)).send().await.unwrap();
    assert_eq!(list.status().as_u16(), 200);
    let download = get(format!("/api/fs/download?path={}/hello.txt", srv.home)).send().await.unwrap();
    assert_eq!(download.status().as_u16(), 200);
    assert_eq!(download.text().await.unwrap(), "hello");
    // Handlers that used to read only the Authorization header
    assert_eq!(get("/api/bookmarks".into()).send().await.unwrap().status().as_u16(), 200);
    assert_eq!(get("/api/auth/tokens".into()).send().await.unwrap().status().as_u16(), 200);

    // Changes from the page work; the same request from another site is blocked
    let mkdir = |site: &'static str| c.post(format!("{}/api/fs/mkdir", srv.base))
        .header("cookie", &pair).header("sec-fetch-site", site)
        .json(&json!({ "path": format!("{}/made-{site}", srv.home) }));
    assert!(mkdir("same-origin").send().await.unwrap().status().is_success());
    assert_eq!(mkdir("cross-site").send().await.unwrap().status().as_u16(), 403);

    // Logging out removes the cookie and ends the login
    let out = c.post(format!("{}/api/auth/logout", srv.base))
        .header("cookie", &pair).header("sec-fetch-site", "same-origin").send().await.unwrap();
    let cleared: Vec<String> = out.headers().get_all("set-cookie").iter().map(|v| v.to_str().unwrap().to_string()).collect();
    assert!(cleared.iter().any(|v| v.starts_with(&format!("brum_session_{}=;", srv.port)) && v.contains("Max-Age=0")));
    assert_eq!(get("/api/auth/me".into()).send().await.unwrap().status().as_u16(), 401);
}

#[tokio::test]
async fn another_brums_cookie_is_ignored_and_the_old_name_still_works() {
    let srv = start().await;
    let (_, pair, _) = page_login(&srv).await;
    let token = pair.split_once('=').unwrap().1.to_string();
    let c = plain_client();
    let me = |cookie: String| c.get(format!("{}/api/auth/me", srv.base)).header("cookie", cookie);
    // A login cookie for another port is not this Brum's
    assert_eq!(me(format!("brum_session_1={token}")).send().await.unwrap().status().as_u16(), 401);
    // Logins from before this change keep working
    assert_eq!(me(format!("cd_token={token}")).send().await.unwrap().status().as_u16(), 200);
    // A stale old cookie does not hide a good new one
    assert_eq!(me(format!("cd_token=stale; {pair}")).send().await.unwrap().status().as_u16(), 200);
}

#[tokio::test]
async fn a_stored_login_from_an_older_version_moves_into_the_cookie() {
    let srv = start().await;
    let body: Value = plain_client().post(format!("{}/api/auth/login", srv.base))
        .json(&json!({ "username": "admin", "password": "AdminSecretPass123!" }))
        .send().await.unwrap().json().await.unwrap();
    let old = body["token"].as_str().unwrap().to_string();

    let resp = plain_client().post(format!("{}/api/auth/session", srv.base))
        .bearer_auth(&old).send().await.unwrap();
    assert!(resp.status().is_success());
    let set: Vec<String> = resp.headers().get_all("set-cookie").iter().map(|v| v.to_str().unwrap().to_string()).collect();
    let own = set.iter().find(|v| v.starts_with(&format!("brum_session_{}={old}", srv.port))).expect("cookie set");
    assert!(own.contains("HttpOnly"));
    assert!(resp.text().await.unwrap().find(&old).is_none(), "the reply must not echo the login");

    // A bad login and an API token are refused
    let bad = plain_client().post(format!("{}/api/auth/session", srv.base)).bearer_auth("nope").send().await.unwrap();
    assert_eq!(bad.status().as_u16(), 401);
    let api: Value = plain_client().post(format!("{}/api/auth/tokens", srv.base))
        .bearer_auth(&old).json(&json!({ "name": "script" })).send().await.unwrap().json().await.unwrap();
    let api_token = api["token"].as_str().or_else(|| api["secret"].as_str()).expect("api token").to_string();
    let refused = plain_client().post(format!("{}/api/auth/session", srv.base)).bearer_auth(&api_token).send().await.unwrap();
    assert_eq!(refused.status().as_u16(), 400);
}

/// The page code never stores the login itself again
#[test]
fn page_code_never_stores_the_login() {
    let app = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/frontend/app.js")).unwrap();
    assert!(!app.contains("localStorage.setItem('cd_token'"), "the login must not go into browser storage");
    assert!(!app.contains("document.cookie = `cd_token="), "the page must not write a login cookie");
    assert!(!app.contains("App.token"), "the page must not keep the login in memory");
}
