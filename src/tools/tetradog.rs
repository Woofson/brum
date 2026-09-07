use chrono::Utc;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TetraScore {
    pub id: i64,
    pub rank: usize,
    pub username: String,
    pub player_name: String,
    pub score: u64,
    pub lines_cleared: u32,
    pub level: u32,
    pub duration_seconds: u32,
    pub mode: String,
    pub created_at: String,
    pub is_current_user: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TetraPersonalBest {
    pub high_score: u64,
    pub max_level: u32,
    pub total_lines: u64,
    pub games_played: u64,
    pub best_rank: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TetraLeaderboardResponse {
    pub leaderboard: Vec<TetraScore>,
    pub user_best: Option<TetraScore>,
    pub user_stats: TetraPersonalBest,
    pub total_entries: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SubmitScoreRequest {
    pub score: u64,
    pub lines_cleared: u32,
    pub level: u32,
    #[serde(default)]
    pub duration_seconds: Option<u32>,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub player_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitScoreResponse {
    pub status: String,
    pub id: i64,
    pub rank: usize,
    pub is_personal_best: bool,
    pub is_global_high_score: bool,
    pub leaderboard: Vec<TetraScore>,
}

pub fn get_leaderboard(
    conn: &Connection,
    current_user: &str,
    limit: usize,
    mode_filter: Option<&str>,
    user_only: bool,
) -> Result<TetraLeaderboardResponse, String> {
    let limit_clamped = limit.clamp(1, 100);
    let target_mode = mode_filter.unwrap_or("marathon");

    let mut scores = Vec::new();
    let query = if user_only {
        "SELECT id, username, player_name, score, lines_cleared, level, duration_seconds, mode, created_at
         FROM tetradog_scores
         WHERE username = ?1 AND mode = ?2
         ORDER BY score DESC, lines_cleared DESC, id ASC
         LIMIT ?3"
    } else {
        "SELECT id, username, player_name, score, lines_cleared, level, duration_seconds, mode, created_at
         FROM tetradog_scores
         WHERE mode = ?1
         ORDER BY score DESC, lines_cleared DESC, id ASC
         LIMIT ?2"
    };

    if user_only {
        let mut stmt = conn.prepare(query).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![current_user, target_mode, limit_clamped as i64], |row| {
                let id: i64 = row.get(0)?;
                let username: String = row.get(1)?;
                let player_name: String = row.get(2)?;
                let score: i64 = row.get(3)?;
                let lines_cleared: i64 = row.get(4)?;
                let level: i64 = row.get(5)?;
                let duration_seconds: i64 = row.get(6)?;
                let mode: String = row.get(7)?;
                let created_at: String = row.get(8)?;

                Ok((id, username, player_name, score, lines_cleared, level, duration_seconds, mode, created_at))
            })
            .map_err(|e| e.to_string())?;

        for (idx, row) in rows.enumerate() {
            if let Ok((id, username, player_name, score, lines_cleared, level, duration_seconds, mode, created_at)) = row {
                let is_current_user = username == current_user;
                scores.push(TetraScore {
                    id,
                    rank: idx + 1,
                    username,
                    player_name,
                    score: score.max(0) as u64,
                    lines_cleared: lines_cleared.max(0) as u32,
                    level: level.max(1) as u32,
                    duration_seconds: duration_seconds.max(0) as u32,
                    mode,
                    created_at,
                    is_current_user,
                });
            }
        }
    } else {
        let mut stmt = conn.prepare(query).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![target_mode, limit_clamped as i64], |row| {
                let id: i64 = row.get(0)?;
                let username: String = row.get(1)?;
                let player_name: String = row.get(2)?;
                let score: i64 = row.get(3)?;
                let lines_cleared: i64 = row.get(4)?;
                let level: i64 = row.get(5)?;
                let duration_seconds: i64 = row.get(6)?;
                let mode: String = row.get(7)?;
                let created_at: String = row.get(8)?;

                Ok((id, username, player_name, score, lines_cleared, level, duration_seconds, mode, created_at))
            })
            .map_err(|e| e.to_string())?;

        for (idx, row) in rows.enumerate() {
            if let Ok((id, username, player_name, score, lines_cleared, level, duration_seconds, mode, created_at)) = row {
                let is_current_user = username == current_user;
                scores.push(TetraScore {
                    id,
                    rank: idx + 1,
                    username,
                    player_name,
                    score: score.max(0) as u64,
                    lines_cleared: lines_cleared.max(0) as u32,
                    level: level.max(1) as u32,
                    duration_seconds: duration_seconds.max(0) as u32,
                    mode,
                    created_at,
                    is_current_user,
                });
            }
        }
    }

    // User's personal best for this mode
    let mut user_best: Option<TetraScore> = None;
    let mut user_stats = TetraPersonalBest::default();

    if let Ok(mut stmt) = conn.prepare(
        "SELECT id, username, player_name, score, lines_cleared, level, duration_seconds, mode, created_at
         FROM tetradog_scores
         WHERE username = ?1 AND mode = ?2
         ORDER BY score DESC, lines_cleared DESC, id ASC
         LIMIT 1"
    ) {
        if let Ok(best_row) = stmt.query_row(params![current_user, target_mode], |row| {
            let id: i64 = row.get(0)?;
            let username: String = row.get(1)?;
            let player_name: String = row.get(2)?;
            let score: i64 = row.get(3)?;
            let lines_cleared: i64 = row.get(4)?;
            let level: i64 = row.get(5)?;
            let duration_seconds: i64 = row.get(6)?;
            let mode: String = row.get(7)?;
            let created_at: String = row.get(8)?;

            Ok((id, username, player_name, score, lines_cleared, level, duration_seconds, mode, created_at))
        }) {
            user_best = Some(TetraScore {
                id: best_row.0,
                rank: 0, // Calculated below
                username: best_row.1,
                player_name: best_row.2,
                score: best_row.3.max(0) as u64,
                lines_cleared: best_row.4.max(0) as u32,
                level: best_row.5.max(1) as u32,
                duration_seconds: best_row.6.max(0) as u32,
                mode: best_row.7,
                created_at: best_row.8,
                is_current_user: true,
            });
        }
    }

    // Aggregate user stats
    if let Ok(mut stmt) = conn.prepare(
        "SELECT MAX(score), MAX(level), SUM(lines_cleared), COUNT(*)
         FROM tetradog_scores
         WHERE username = ?1"
    ) {
        if let Ok(stats) = stmt.query_row(params![current_user], |row| {
            let max_score: Option<i64> = row.get(0)?;
            let max_lvl: Option<i64> = row.get(1)?;
            let total_ln: Option<i64> = row.get(2)?;
            let total_cnt: i64 = row.get(3)?;
            Ok((max_score, max_lvl, total_ln, total_cnt))
        }) {
            user_stats.high_score = stats.0.unwrap_or(0).max(0) as u64;
            user_stats.max_level = stats.1.unwrap_or(1).max(1) as u32;
            user_stats.total_lines = stats.2.unwrap_or(0).max(0) as u64;
            user_stats.games_played = stats.3.max(0) as u64;
        }
    }

    // Determine rank of user's best score in global ranking
    if let Some(ref mut best) = user_best {
        if let Ok(mut stmt) = conn.prepare(
            "SELECT COUNT(*) FROM tetradog_scores WHERE mode = ?1 AND (score > ?2 OR (score = ?2 AND id < ?3))"
        ) {
            let higher_count: i64 = stmt.query_row(params![target_mode, best.score as i64, best.id], |r| r.get(0)).unwrap_or(0);
            let rank = (higher_count + 1) as usize;
            best.rank = rank;
            user_stats.best_rank = Some(rank);
        }
    }

    let total_entries: i64 = conn
        .query_row("SELECT COUNT(*) FROM tetradog_scores WHERE mode = ?1", params![target_mode], |r| r.get(0))
        .unwrap_or(0);

    Ok(TetraLeaderboardResponse {
        leaderboard: scores,
        user_best,
        user_stats,
        total_entries: total_entries as usize,
    })
}

pub fn submit_score(
    conn: &Connection,
    username: &str,
    req: SubmitScoreRequest,
) -> Result<SubmitScoreResponse, String> {
    let mode = req.mode.unwrap_or_else(|| "marathon".to_string());
    let duration = req.duration_seconds.unwrap_or(0);
    let player_name = req
        .player_name
        .filter(|p| !p.trim().is_empty())
        .unwrap_or_else(|| username.to_string());

    let score = req.score;
    let lines = req.lines_cleared;
    let level = req.level.max(1);
    let now = Utc::now().to_rfc3339();

    // Check existing personal best before insert
    let prev_personal_best: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(score), 0) FROM tetradog_scores WHERE username = ?1 AND mode = ?2",
            params![username, mode],
            |r| r.get(0),
        )
        .unwrap_or(0);

    // Check existing global top score
    let prev_global_best: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(score), 0) FROM tetradog_scores WHERE mode = ?1",
            params![mode],
            |r| r.get(0),
        )
        .unwrap_or(0);

    // Insert new score record
    conn.execute(
        "INSERT INTO tetradog_scores (username, player_name, score, lines_cleared, level, duration_seconds, mode, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            username,
            player_name,
            score as i64,
            lines as i64,
            level as i64,
            duration as i64,
            mode,
            now,
        ],
    )
    .map_err(|e| format!("Failed to record TetraDog score: {}", e))?;

    let inserted_id = conn.last_insert_rowid();

    // Calculate newly achieved rank
    let higher_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM tetradog_scores WHERE mode = ?1 AND (score > ?2 OR (score = ?2 AND id < ?3))",
            params![mode, score as i64, inserted_id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let rank = (higher_count + 1) as usize;

    let is_personal_best = (score as i64) > prev_personal_best;
    let is_global_high_score = (score as i64) > prev_global_best;

    // Fetch refreshed leaderboard (Top 25)
    let lb_resp = get_leaderboard(conn, username, 25, Some(&mode), false)?;

    Ok(SubmitScoreResponse {
        status: "ok".to_string(),
        id: inserted_id,
        rank,
        is_personal_best,
        is_global_high_score,
        leaderboard: lb_resp.leaderboard,
    })
}

pub fn clear_scores(conn: &Connection, username_filter: Option<&str>) -> Result<usize, String> {
    let rows_affected = if let Some(user) = username_filter {
        conn.execute("DELETE FROM tetradog_scores WHERE username = ?1", params![user])
            .map_err(|e| e.to_string())?
    } else {
        conn.execute("DELETE FROM tetradog_scores", [])
            .map_err(|e| e.to_string())?
    };
    Ok(rows_affected)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute(
            "CREATE TABLE tetradog_scores (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                username TEXT NOT NULL,
                player_name TEXT NOT NULL,
                score INTEGER NOT NULL,
                lines_cleared INTEGER NOT NULL,
                level INTEGER NOT NULL,
                duration_seconds INTEGER NOT NULL DEFAULT 0,
                mode TEXT NOT NULL DEFAULT 'marathon',
                created_at TEXT NOT NULL
            )",
            [],
        )
        .unwrap();
        conn
    }

    #[test]
    fn test_tetradog_scoring_and_leaderboard_multiuser() {
        let conn = setup_test_db();

        // 1. Bolt plays marathon
        let res1 = submit_score(
            &conn,
            "bolt",
            SubmitScoreRequest {
                score: 15400,
                lines_cleared: 24,
                level: 3,
                duration_seconds: Some(120),
                mode: Some("marathon".to_string()),
                player_name: Some("Bolt Woofson".to_string()),
            },
        )
        .unwrap();

        assert_eq!(res1.rank, 1);
        assert!(res1.is_personal_best);
        assert!(res1.is_global_high_score);

        // 2. Alice plays and gets higher score
        let res2 = submit_score(
            &conn,
            "alice",
            SubmitScoreRequest {
                score: 28900,
                lines_cleared: 42,
                level: 5,
                duration_seconds: Some(210),
                mode: Some("marathon".to_string()),
                player_name: Some("Alice Doggo".to_string()),
            },
        )
        .unwrap();

        assert_eq!(res2.rank, 1);
        assert!(res2.is_global_high_score);

        // 3. Bob plays and gets score between Alice and Bolt
        let res3 = submit_score(
            &conn,
            "bob",
            SubmitScoreRequest {
                score: 20000,
                lines_cleared: 30,
                level: 4,
                duration_seconds: Some(150),
                mode: Some("marathon".to_string()),
                player_name: Some("Bob Barker".to_string()),
            },
        )
        .unwrap();

        assert_eq!(res3.rank, 2);

        // 4. Bolt plays again and sets new all-time high score!
        let res4 = submit_score(
            &conn,
            "bolt",
            SubmitScoreRequest {
                score: 50200,
                lines_cleared: 68,
                level: 7,
                duration_seconds: Some(340),
                mode: Some("marathon".to_string()),
                player_name: Some("Bolt Woofson".to_string()),
            },
        )
        .unwrap();

        assert_eq!(res4.rank, 1);
        assert!(res4.is_personal_best);
        assert!(res4.is_global_high_score);

        // 5. Query full leaderboard as Bob
        let lb = get_leaderboard(&conn, "bob", 10, Some("marathon"), false).unwrap();
        assert_eq!(lb.leaderboard.len(), 4);
        assert_eq!(lb.leaderboard[0].username, "bolt");
        assert_eq!(lb.leaderboard[0].score, 50200);
        assert_eq!(lb.leaderboard[1].username, "alice");
        assert_eq!(lb.leaderboard[1].score, 28900);
        assert_eq!(lb.leaderboard[2].username, "bob");
        assert_eq!(lb.leaderboard[2].score, 20000);
        assert_eq!(lb.leaderboard[3].username, "bolt");
        assert_eq!(lb.leaderboard[3].score, 15400);

        // Verify Bob's personal stats
        assert_eq!(lb.user_stats.high_score, 20000);
        assert_eq!(lb.user_stats.max_level, 4);
        assert_eq!(lb.user_stats.total_lines, 30);
        assert_eq!(lb.user_stats.games_played, 1);
        assert_eq!(lb.user_stats.best_rank, Some(3));

        // 6. Test mode separation
        let sprint_res = submit_score(
            &conn,
            "bolt",
            SubmitScoreRequest {
                score: 4000,
                lines_cleared: 40,
                level: 4,
                duration_seconds: Some(85),
                mode: Some("sprint".to_string()),
                player_name: None,
            },
        )
        .unwrap();
        assert_eq!(sprint_res.rank, 1);

        let sprint_lb = get_leaderboard(&conn, "bolt", 10, Some("sprint"), false).unwrap();
        assert_eq!(sprint_lb.leaderboard.len(), 1);
        assert_eq!(sprint_lb.leaderboard[0].mode, "sprint");

        // Marathon leaderboard still has 4
        let marathon_lb = get_leaderboard(&conn, "bolt", 10, Some("marathon"), false).unwrap();
        assert_eq!(marathon_lb.leaderboard.len(), 4);
    }
}

