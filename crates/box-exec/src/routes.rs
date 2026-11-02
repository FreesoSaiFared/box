use axum::extract::DefaultBodyLimit;
use axum::middleware;
use axum::routing::{get, post};
use axum::Json;
use axum::Router;
use serde::Serialize;

use crate::middleware::require_token;
use crate::{control, cua, exec, files, program, AppState};

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
}

pub fn app(state: AppState) -> Router {
    let protected = Router::new()
        .route("/v1/exec", post(exec::handle))
        .route("/v1/exec/stream", post(exec::handle_stream))
        .route("/v1/exec/{id}", get(exec::status).delete(exec::cancel))
        .route(
            "/v1/files",
            get(files::get).put(files::put).delete(files::delete),
        )
        .route("/v1/files/raw", get(files::get_raw).put(files::put_raw))
        .route("/v1/files/mkdir", post(files::mkdir))
        .route("/v1/files/rename", post(files::rename))
        .route("/v1/busy", get(control::busy))
        .route("/v1/shutdown", post(control::shutdown))
        .route("/v1/metrics", get(control::metrics))
        .route("/v1/cua/screenshot", post(cua::screenshot_handler))
        .route("/v1/cua/click", post(cua::click_handler))
        .route("/v1/cua/double-click", post(cua::double_click_handler))
        .route("/v1/cua/move", post(cua::move_handler))
        .route("/v1/cua/drag", post(cua::drag_handler))
        .route("/v1/cua/press", post(cua::press_handler))
        .route("/v1/cua/mousedown", post(cua::press_handler))
        .route("/v1/cua/release", post(cua::release_handler))
        .route("/v1/cua/mouseup", post(cua::release_handler))
        .route("/v1/cua/type", post(cua::type_handler))
        .route("/v1/cua/key", post(cua::key_handler))
        .route("/v1/cua/scroll", post(cua::scroll_handler))
        .route("/v1/cua/recipe", post(cua::recipe_handler))
        .route("/v1/recipe", post(program::handle))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_token));

    Router::new()
        .route("/v1/health", get(health))
        .merge(protected)
        .layer(DefaultBodyLimit::max(
            (state.max_file_bytes as usize).saturating_add(256 * 1024),
        ))
        .layer(middleware::from_fn(box_common::echo_request_id))
        .with_state(state)
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}
