use axum::Router;
use axum::response::Html;
use axum::routing::get;

use crate::state::SharedState;
use crate::ws;

/// Build the main application router with all routes.
pub fn build_router(state: SharedState) -> Router {
    Router::new()
        .route("/", get(web_chat))
        .route("/health", get(health))
        .route("/ws", get(ws::ws_handler))
        .route("/api/status", get(status))
        .with_state(state)
}

async fn health() -> &'static str {
    "ok"
}

async fn web_chat() -> Html<&'static str> {
    Html(include_str!("webchat.html"))
}

async fn status(
    axum::extract::State(state): axum::extract::State<SharedState>,
) -> axum::Json<serde_json::Value> {
    let channels = state.channels.list();
    axum::Json(serde_json::json!({
        "status": "running",
        "channels": channels,
        "sessions": state.sessions.len(),
    }))
}
