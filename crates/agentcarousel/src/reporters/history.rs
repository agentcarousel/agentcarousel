use agentcarousel_core::Run;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use std::env;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Clone, serde::Serialize)]
pub struct RunListing {
    pub id: String,
    pub started_at: DateTime<Utc>,
}

#[derive(Debug, Error)]
pub enum HistoryError {
    #[error("failed to open history db at {path}: {source}")]
    OpenError {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to connect history db at {path}: {source}")]
    ConnectError {
        path: PathBuf,
        source: rusqlite::Error,
    },
    #[error("failed to run history query: {source}")]
    QueryError { source: rusqlite::Error },
    #[error("failed to parse run json: {source}")]
    ParseError { source: serde_json::Error },
}

pub fn persist_run(run: &Run) -> Result<(), HistoryError> {
    let conn = open_connection()?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS runs (
            id TEXT PRIMARY KEY,
            started_at TEXT NOT NULL,
            run_json TEXT NOT NULL
        )",
        [],
    )
    .map_err(|source| HistoryError::QueryError { source })?;

    let json = serde_json::to_string(run).map_err(|source| HistoryError::ParseError { source })?;
    conn.execute(
        "INSERT OR REPLACE INTO runs (id, started_at, run_json) VALUES (?1, ?2, ?3)",
        params![run.id.0, run.started_at.to_rfc3339(), json],
    )
    .map_err(|source| HistoryError::QueryError { source })?;
    Ok(())
}

pub fn list_runs(limit: usize) -> Result<Vec<RunListing>, HistoryError> {
    let conn = open_connection()?;
    ensure_runs_table(&conn)?;
    let mut stmt = conn
        .prepare("SELECT id, started_at FROM runs ORDER BY started_at DESC LIMIT ?1")
        .map_err(|source| HistoryError::QueryError { source })?;
    let rows = stmt
        .query_map([limit as i64], |row| {
            let id: String = row.get(0)?;
            let started_at: String = row.get(1)?;
            let parsed = DateTime::parse_from_rfc3339(&started_at)
                .map_err(|_| rusqlite::Error::InvalidQuery)?;
            Ok(RunListing {
                id,
                started_at: parsed.with_timezone(&Utc),
            })
        })
        .map_err(|source| HistoryError::QueryError { source })?;

    let mut results = Vec::new();
    for row in rows.flatten() {
        results.push(row);
    }
    Ok(results)
}

pub fn list_full_runs(limit: usize) -> Result<Vec<Run>, HistoryError> {
    let conn = open_connection()?;
    ensure_runs_table(&conn)?;
    let mut stmt = conn
        .prepare("SELECT run_json FROM runs ORDER BY started_at DESC LIMIT ?1")
        .map_err(|source| HistoryError::QueryError { source })?;
    let rows = stmt
        .query_map([limit as i64], |row| row.get::<_, String>(0))
        .map_err(|source| HistoryError::QueryError { source })?;
    rows.flatten()
        .map(|json| {
            serde_json::from_str(&json).map_err(|source| HistoryError::ParseError { source })
        })
        .collect()
}

/// Returns the most recent run for `skill_or_agent` that is older than `before_run_id`.
pub fn find_previous_run(
    skill_or_agent: &str,
    before_run_id: &str,
) -> Result<Option<Run>, HistoryError> {
    let conn = open_connection()?;
    ensure_runs_table(&conn)?;
    let anchor_started_at: Option<String> = conn
        .query_row(
            "SELECT started_at FROM runs WHERE id = ?1",
            [before_run_id],
            |row| row.get(0),
        )
        .ok();
    let Some(anchor) = anchor_started_at else {
        return Ok(None);
    };
    let mut stmt = conn
        .prepare(
            "SELECT run_json FROM runs WHERE started_at < ?1 ORDER BY started_at DESC LIMIT 20",
        )
        .map_err(|source| HistoryError::QueryError { source })?;
    let rows = stmt
        .query_map([&anchor], |row| row.get::<_, String>(0))
        .map_err(|source| HistoryError::QueryError { source })?;
    for json in rows.flatten() {
        let run: Run =
            serde_json::from_str(&json).map_err(|source| HistoryError::ParseError { source })?;
        if run.skill_or_agent.as_deref() == Some(skill_or_agent) {
            return Ok(Some(run));
        }
    }
    Ok(None)
}

/// Stores a named tag pointing to `run_id` (upserts — last write wins).
pub fn tag_run(name: &str, run_id: &str) -> Result<(), HistoryError> {
    let conn = open_connection()?;
    ensure_runs_table(&conn)?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS run_tags (name TEXT PRIMARY KEY, run_id TEXT NOT NULL)",
        [],
    )
    .map_err(|source| HistoryError::QueryError { source })?;
    conn.execute(
        "INSERT OR REPLACE INTO run_tags (name, run_id) VALUES (?1, ?2)",
        params![name, run_id],
    )
    .map_err(|source| HistoryError::QueryError { source })?;
    Ok(())
}

/// Returns the run ID stored under `name`, or `None` if the tag does not exist.
pub fn find_tagged_run(name: &str) -> Result<Option<String>, HistoryError> {
    let conn = open_connection()?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS run_tags (name TEXT PRIMARY KEY, run_id TEXT NOT NULL)",
        [],
    )
    .map_err(|source| HistoryError::QueryError { source })?;
    let result = conn
        .query_row(
            "SELECT run_id FROM run_tags WHERE name = ?1",
            [name],
            |row| row.get::<_, String>(0),
        )
        .ok();
    Ok(result)
}

pub fn fetch_run(run_id: &str) -> Result<Run, HistoryError> {
    let conn = open_connection()?;
    ensure_runs_table(&conn)?;
    let mut stmt = conn
        .prepare("SELECT run_json FROM runs WHERE id = ?1")
        .map_err(|source| HistoryError::QueryError { source })?;
    let mut rows = stmt
        .query([run_id])
        .map_err(|source| HistoryError::QueryError { source })?;
    if let Some(row) = rows
        .next()
        .map_err(|source| HistoryError::QueryError { source })?
    {
        let json: String = row
            .get(0)
            .map_err(|source| HistoryError::QueryError { source })?;
        let run: Run =
            serde_json::from_str(&json).map_err(|source| HistoryError::ParseError { source })?;
        Ok(run)
    } else {
        Err(HistoryError::QueryError {
            source: rusqlite::Error::QueryReturnedNoRows,
        })
    }
}

fn open_connection() -> Result<Connection, HistoryError> {
    let path = history_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| HistoryError::OpenError {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    Connection::open(path.clone()).map_err(|source| HistoryError::ConnectError { path, source })
}

fn history_path() -> PathBuf {
    if let Ok(path) = env::var("AGENTCAROUSEL_HISTORY_DB") {
        return PathBuf::from(path);
    }
    let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
    if cfg!(target_os = "macos") {
        PathBuf::from(home).join("Library/Application Support/agentcarousel/history.db")
    } else {
        PathBuf::from(home).join(".local/share/agentcarousel/history.db")
    }
}

fn ensure_runs_table(conn: &Connection) -> Result<(), HistoryError> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS runs (
            id TEXT PRIMARY KEY,
            started_at TEXT NOT NULL,
            run_json TEXT NOT NULL
        )",
        [],
    )
    .map_err(|source| HistoryError::QueryError { source })?;
    Ok(())
}
