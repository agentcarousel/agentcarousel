mod api;
mod assets;

use std::path::PathBuf;
use std::sync::Arc;

use axum::{routing::get, Router};
use tokio::sync::broadcast;

use api::AppState;

pub use api::ReviewAnnotation;

pub async fn serve(port: u16, _db_path: Option<PathBuf>, dev_mode: bool) -> Result<(), String> {
    let (tx, _rx) = broadcast::channel::<String>(32);
    let tx = Arc::new(tx);

    // Background poller: broadcasts a heartbeat every 5 s so SSE clients refresh.
    {
        let tx = tx.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
            loop {
                interval.tick().await;
                // Send a lightweight ping; clients reload their own data.
                let _ = tx.send("ping".to_string());
            }
        });
    }

    let reviews_path = reviews_path();
    let state = Arc::new(AppState {
        event_tx: tx,
        reviews_path,
        dev_mode,
    });

    let app = Router::new()
        // HTML pages
        .route("/", get(api::serve_index))
        .route("/runs/{id}", get(api::serve_run))
        .route("/compare", get(api::serve_compare))
        .route("/review", get(api::serve_review))
        // JSON API
        .route("/api/runs", get(api::api_list_runs))
        .route("/api/runs/{id}", get(api::api_get_run))
        .route("/api/compare", get(api::api_compare))
        .route("/api/stats", get(api::api_stats))
        .route("/api/events", get(api::api_events))
        .route(
            "/api/reviews",
            get(api::api_get_reviews).post(api::api_save_review),
        )
        .with_state(state);

    let addr = format!("127.0.0.1:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| format!("cannot bind {addr}: {e}"))?;

    let url = format!("http://localhost:{port}");
    eprintln!("agentcarousel dashboard → {url}");

    axum::serve(listener, app)
        .await
        .map_err(|e| format!("server error: {e}"))
}

fn reviews_path() -> PathBuf {
    if let Ok(db) = std::env::var("AGENTCAROUSEL_HISTORY_DB") {
        let p = PathBuf::from(db);
        return p.parent().unwrap_or(&p).join("reviews.jsonl");
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    if cfg!(target_os = "macos") {
        PathBuf::from(home).join("Library/Application Support/agentcarousel/reviews.jsonl")
    } else {
        PathBuf::from(home).join(".local/share/agentcarousel/reviews.jsonl")
    }
}
