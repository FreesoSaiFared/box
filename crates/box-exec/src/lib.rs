//! HTTP exec daemon for grok-box (`box-exec`).
//!
//! Shell, workspace files, and Computer Use (CUA) actuators against the
//! box X display. Inference stays in L4. The process-wide heap is mimalloc
//! when the `mimalloc` feature is on (default); see `box_common::GLOBAL_ALLOCATOR`.

mod control;
mod cua;
mod exec;
mod fdcount;
mod files;
mod middleware;
mod program;
mod routes;

#[cfg(test)]
mod program_tests;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use box_common::{cors_layer, BoxConfig, GLOBAL_ALLOCATOR};
use box_cua::CuaConfig;
use tokio::net::TcpListener;
use tokio::sync::{Mutex, Notify, Semaphore};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::TraceLayer;

pub use routes::app;

/// `exec_id` → process group id for every child this daemon currently owns.
///
/// A `std::sync::Mutex` rather than the async one on purpose: the guard that
/// keeps this table honest has to deregister from `Drop`, which cannot await.
pub type ChildGroups = Arc<std::sync::Mutex<HashMap<String, i32>>>;

#[cfg(test)]
mod tests;

#[derive(Clone, Debug)]
pub struct AppState {
    pub workspace: PathBuf,
    pub token: String,
    pub max_file_bytes: u64,
    pub default_timeout: Duration,
    pub max_timeout: Duration,
    pub max_output_bytes: usize,
    pub max_dir_entries: usize,
    pub max_concurrent_execs: usize,
    pub kill_grace: Duration,
    pub cua: CuaConfig,
    pub exec_slots: Arc<Semaphore>,
    pub execs_in_flight: Arc<AtomicUsize>,
    pub execs_started: Arc<AtomicU64>,
    pub shutdown: Arc<Notify>,
    pub shutting_down: Arc<AtomicBool>,
    pub started_at: Instant,
    pub jobs: Arc<Mutex<HashMap<String, DetachedJob>>>,
    pub child_groups: ChildGroups,
}

#[derive(Clone, Debug)]
pub struct DetachedJob {
    pub status: &'static str,
    pub result: Option<exec::ExecResponse>,
}

impl AppState {
    pub fn from_config(config: &BoxConfig) -> Self {
        let workspace = canonicalize_workspace(&config.workspace);
        let max_concurrent_execs = config.max_concurrent_execs;
        Self {
            workspace,
            token: config.token.clone(),
            max_file_bytes: config.max_file_bytes,
            default_timeout: config.default_timeout,
            max_timeout: config.max_timeout,
            max_output_bytes: config.max_output_bytes,
            max_dir_entries: config.max_dir_entries,
            max_concurrent_execs,
            kill_grace: config.kill_grace,
            cua: CuaConfig::from_env(),
            exec_slots: Arc::new(Semaphore::new(max_concurrent_execs)),
            execs_in_flight: Arc::new(AtomicUsize::new(0)),
            execs_started: Arc::new(AtomicU64::new(0)),
            shutdown: Arc::new(Notify::new()),
            shutting_down: Arc::new(AtomicBool::new(false)),
            started_at: Instant::now(),
            jobs: Arc::new(Mutex::new(HashMap::new())),
            child_groups: Arc::new(std::sync::Mutex::new(HashMap::new())),
        }
    }

    #[cfg(test)]
    pub fn for_test(workspace: PathBuf, token: &str) -> Self {
        Self {
            workspace,
            token: token.into(),
            max_file_bytes: 1024 * 1024,
            default_timeout: Duration::from_secs(5),
            max_timeout: Duration::from_secs(10),
            max_output_bytes: 64 * 1024,
            max_dir_entries: 4096,
            max_concurrent_execs: 8,
            kill_grace: Duration::from_millis(400),
            cua: CuaConfig::disabled(),
            exec_slots: Arc::new(Semaphore::new(8)),
            execs_in_flight: Arc::new(AtomicUsize::new(0)),
            execs_started: Arc::new(AtomicU64::new(0)),
            shutdown: Arc::new(Notify::new()),
            shutting_down: Arc::new(AtomicBool::new(false)),
            started_at: Instant::now(),
            jobs: Arc::new(Mutex::new(HashMap::new())),
            child_groups: Arc::new(std::sync::Mutex::new(HashMap::new())),
        }
    }
}

fn canonicalize_workspace(path: &std::path::Path) -> PathBuf {
    let _ = std::fs::create_dir_all(path);
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

pub fn router(state: AppState) -> Router {
    let body_limit = (state.max_file_bytes as usize).saturating_add(256 * 1024);
    app(state)
        .layer(RequestBodyLimitLayer::new(body_limit))
        .layer(TraceLayer::new_for_http())
        .layer(cors_layer())
}

pub async fn serve(config: BoxConfig) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    std::fs::create_dir_all(&config.workspace)?;
    let bind = config.exec_bind;
    let state = AppState::from_config(&config);
    let shutdown = state.shutdown.clone();
    let app = router(state);
    tracing::info!(
        %bind,
        workspace = %config.workspace.display(),
        allocator = GLOBAL_ALLOCATOR,
        "box-exec listening"
    );
    let listener = TcpListener::bind(bind).await?;
    let warmup_cfg = CuaConfig::from_env();
    tokio::spawn(async move {
        box_cua::warmup_pointer(&warmup_cfg).await;
    });
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(shutdown))
        .await?;
    Ok(())
}

async fn shutdown_signal(notify: Arc<Notify>) {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut sig) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            sig.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
        _ = notify.notified() => {}
    }
}
