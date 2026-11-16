//! #195: Brum's real start-up reads the settings saved in Brum and puts config.toml on top.
//! Its own test program, because it points Brum at a config file with an environment variable.

use brum::config::ConfigManager;

#[test]
fn start_up_uses_saved_settings_under_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("brum.db");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute("CREATE TABLE server_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)", []).unwrap();
    for (k, v) in [
        ("config:fleet.transfer_jobs", "7"),
        ("config:server.server_name", "\"Saved name\""),
        ("config:server.database_path", "\"/elsewhere.db\""), // never allowed: ignored
        ("image_cache_retention_days", "3"),                   // another kind of setting: ignored
    ] {
        conn.execute("INSERT INTO server_settings (key, value) VALUES (?1, ?2)", [k, v]).unwrap();
    }
    drop(conn);

    let file = dir.path().join("config.toml");
    let db_text = db.to_string_lossy().replace('\\', "/");
    std::fs::write(&file, format!("[server]\ndatabase_path = \"{db_text}\"\nserver_name = \"File name\"\n")).unwrap();
    std::env::set_var("BRUM_CONFIG", &file);

    let cfg = ConfigManager::load_all();
    assert_eq!(cfg.fleet.transfer_jobs, 7, "a saved setting is used");
    assert_eq!(cfg.server.server_name, "File name", "the file wins");
    assert_eq!(cfg.server.database_path.replace('\\', "/"), db_text, "the database location only comes from the file");
    assert_eq!(cfg.server.config_file.as_deref(), Some(file.as_path()));
    assert!(ConfigManager::settings_location().contains("+ 2 settings saved in Brum"), "{}", ConfigManager::settings_location());
}
