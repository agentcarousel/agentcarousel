use std::convert::Infallible;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        Html, IntoResponse, Json,
    },
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{broadcast, mpsc};
use tokio_stream::{wrappers::UnboundedReceiverStream, StreamExt as TokioStreamExt};

use crate::reporters::{fetch_run, list_full_runs};

pub struct AppState {
    pub event_tx: Arc<broadcast::Sender<String>>,
    pub reviews_path: PathBuf,
    pub dev_mode: bool,
}

// ── HTML page handlers ────────────────────────────────────────────────────────

pub async fn serve_index(State(st): State<Arc<AppState>>) -> Html<String> {
    Html(load_asset(
        st.dev_mode,
        "dashboard/index.html",
        super::assets::INDEX_HTML,
    ))
}

pub async fn serve_run(State(st): State<Arc<AppState>>) -> Html<String> {
    Html(load_asset(
        st.dev_mode,
        "dashboard/run.html",
        super::assets::RUN_HTML,
    ))
}

pub async fn serve_compare(State(st): State<Arc<AppState>>) -> Html<String> {
    Html(load_asset(
        st.dev_mode,
        "dashboard/compare.html",
        super::assets::COMPARE_HTML,
    ))
}

pub async fn serve_review(State(st): State<Arc<AppState>>) -> Html<String> {
    Html(load_asset(
        st.dev_mode,
        "dashboard/review.html",
        super::assets::REVIEW_HTML,
    ))
}

fn load_asset(dev_mode: bool, rel_path: &str, embedded: &str) -> String {
    if dev_mode {
        std::fs::read_to_string(rel_path).unwrap_or_else(|_| embedded.to_string())
    } else {
        embedded.to_string()
    }
}

// ── JSON API ──────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct ListRunsQuery {
    limit: Option<usize>,
}

pub async fn api_list_runs(Query(q): Query<ListRunsQuery>) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(50).min(200);
    match list_full_runs(limit) {
        Ok(runs) => Json(json!({ "runs": runs })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub async fn api_get_run(Path(id): Path<String>) -> impl IntoResponse {
    match fetch_run(&id) {
        Ok(run) => Json(serde_json::to_value(run).unwrap_or(Value::Null)).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

pub async fn api_stats() -> impl IntoResponse {
    let runs = match list_full_runs(500) {
        Ok(r) => r,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };

    let total_runs = runs.len();
    let overall_pass_rate = if runs.is_empty() {
        None
    } else {
        let avg = runs.iter().map(|r| r.summary.pass_rate as f64).sum::<f64>() / runs.len() as f64;
        Some(avg)
    };
    let mean_effectiveness = {
        let judged: Vec<f64> = runs
            .iter()
            .filter_map(|r| r.summary.mean_effectiveness_score.map(|s| s as f64))
            .collect();
        if judged.is_empty() {
            None
        } else {
            Some(judged.iter().sum::<f64>() / judged.len() as f64)
        }
    };

    let week_ago = Utc::now() - chrono::Duration::days(7);
    let runs_this_week = runs.iter().filter(|r| r.started_at > week_ago).count();

    Json(json!({
        "total_runs": total_runs,
        "overall_pass_rate": overall_pass_rate,
        "mean_effectiveness": mean_effectiveness,
        "runs_this_week": runs_this_week,
    }))
    .into_response()
}

#[derive(Deserialize)]
pub struct CompareQuery {
    a: String,
    b: String,
}

pub async fn api_compare(Query(q): Query<CompareQuery>) -> impl IntoResponse {
    let baseline = match fetch_run(&q.a) {
        Ok(r) => r,
        Err(_) => {
            return (StatusCode::NOT_FOUND, format!("Run '{}' not found", q.a)).into_response()
        }
    };
    let current = match fetch_run(&q.b) {
        Ok(r) => r,
        Err(_) => {
            return (StatusCode::NOT_FOUND, format!("Run '{}' not found", q.b)).into_response()
        }
    };

    let result = build_compare(&baseline, &current);
    Json(result).into_response()
}

fn build_compare(baseline: &agentcarousel_core::Run, current: &agentcarousel_core::Run) -> Value {
    use std::collections::HashMap;
    let threshold = 0.05_f32;

    let baseline_map: HashMap<&str, &agentcarousel_core::CaseResult> = baseline
        .cases
        .iter()
        .map(|c| (c.case_id.0.as_str(), c))
        .collect();

    let cases: Vec<Value> = current
        .cases
        .iter()
        .map(|c| {
            let b_eff = baseline_map
                .get(c.case_id.0.as_str())
                .and_then(|b| b.eval_scores.as_ref())
                .map(|s| s.effectiveness_score);
            let c_eff = c.eval_scores.as_ref().map(|s| s.effectiveness_score);
            let delta = match (b_eff, c_eff) {
                (Some(b), Some(c)) => Some(c - b),
                _ => None,
            };
            let regression = delta.is_some_and(|d| d < -threshold);
            json!({
                "case_id": c.case_id.0,
                "baseline_effectiveness": b_eff,
                "current_effectiveness": c_eff,
                "delta": delta,
                "regression": regression,
            })
        })
        .collect();

    let b_pass = baseline.summary.pass_rate;
    let c_pass = current.summary.pass_rate;
    let b_eff = baseline.summary.mean_effectiveness_score;
    let c_eff = current.summary.mean_effectiveness_score;
    let eff_delta = match (b_eff, c_eff) {
        (Some(b), Some(c)) => Some(c - b),
        _ => None,
    };
    let regression = eff_delta.is_some_and(|d| d < -threshold)
        || cases
            .iter()
            .any(|c| c["regression"].as_bool().unwrap_or(false));

    json!({
        "baseline_run_id": baseline.id.0,
        "current_run_id": current.id.0,
        "skill_or_agent": current.skill_or_agent,
        "overall_effectiveness_delta": eff_delta,
        "pass_rate_delta": c_pass - b_pass,
        "regression": regression,
        "threshold": threshold,
        "cases": cases,
    })
}

// ── SSE live stream ───────────────────────────────────────────────────────────

pub async fn api_events(
    State(st): State<Arc<AppState>>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let (tx, rx) = mpsc::unbounded_channel::<String>();
    let mut broadcast_rx = st.event_tx.subscribe();
    tokio::spawn(async move {
        while let Ok(msg) = broadcast_rx.recv().await {
            if tx.send(msg).is_err() {
                break;
            }
        }
    });
    let stream = TokioStreamExt::map(UnboundedReceiverStream::new(rx), |data| {
        Ok::<Event, Infallible>(Event::default().data(data))
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

// ── Reviews ───────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct ReviewAnnotation {
    pub run_id: String,
    pub case_id: String,
    pub verdict: String,
    pub note: Option<String>,
    pub created_at: String,
}

#[derive(Deserialize)]
pub struct SaveReviewBody {
    pub run_id: String,
    pub case_id: String,
    pub verdict: String,
    pub note: Option<String>,
}

pub async fn api_save_review(
    State(st): State<Arc<AppState>>,
    Json(body): Json<SaveReviewBody>,
) -> impl IntoResponse {
    let annotation = ReviewAnnotation {
        run_id: body.run_id,
        case_id: body.case_id,
        verdict: body.verdict,
        note: body.note,
        created_at: Utc::now().to_rfc3339(),
    };
    let line = match serde_json::to_string(&annotation) {
        Ok(l) => l,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    let result = (|| -> std::io::Result<()> {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&st.reviews_path)?;
        writeln!(f, "{line}")
    })();
    match result {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[derive(Deserialize)]
pub struct GetReviewsQuery {
    run: String,
}

pub async fn api_get_reviews(
    State(st): State<Arc<AppState>>,
    Query(q): Query<GetReviewsQuery>,
) -> impl IntoResponse {
    let text = std::fs::read_to_string(&st.reviews_path).unwrap_or_default();
    let reviews: Vec<ReviewAnnotation> = text
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .filter(|r: &ReviewAnnotation| r.run_id == q.run)
        .collect();
    Json(reviews).into_response()
}
