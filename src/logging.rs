//! Dedicated file-based logging for this MCP server.
//!
//! stdout is reserved exclusively for JSON-RPC responses — nothing here
//! ever writes there. This module additionally captures failures to a
//! persistent log file under `~/.config/mcp-for-page-builders/logs/`,
//! independent of stderr, so a crashed/killed process (e.g. the client
//! tearing down the transport) still leaves a diagnosable trail.
//!
//! Call [`init`] once at startup. It returns a guard that must be kept
//! alive for the process lifetime (dropping it stops the background
//! writer thread and log lines may be lost).

use std::panic;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::prelude::*;

/// Initializes logging to both stderr (human-readable, when attached to a
/// terminal) and a rolling daily file under `log_dir()`. Also installs a
/// panic hook that records panics to the file log before the default hook
/// runs, so panics inside spawned tasks (which would otherwise only ever
/// reach stderr, easily lost when the MCP client owns the pipe) are
/// captured durably.
///
/// Returns a guard: keep it alive (e.g. bind to a variable in `main`) for
/// the lifetime of the process.
pub fn init() -> anyhow::Result<WorkerGuard> {
    let dir = crate::util::log_dir();
    std::fs::create_dir_all(&dir)?;

    let file_appender = tracing_appender::rolling::daily(&dir, "mcp-for-page-builders.log");
    let (file_writer, guard) = tracing_appender::non_blocking(file_appender);

    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("mcp_for_page_builders=info"));

    let stderr_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_target(false);

    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(file_writer)
        .with_ansi(false)
        .with_target(true)
        .with_thread_ids(true);

    tracing_subscriber::registry()
        .with(env_filter)
        .with(stderr_layer)
        .with(file_layer)
        .init();

    install_panic_hook();

    tracing::info!(
        log_dir = %dir.display(),
        pid = std::process::id(),
        "Logging initialized"
    );

    Ok(guard)
}

/// Wraps the default panic hook so every panic (main thread or any spawned
/// task/thread) is also written to `tracing::error!`, which routes into the
/// file log above. Without this, a panic in a background CDP task can be
/// invisible: tokio prints it to stderr once and the task quietly dies,
/// which from the MCP client's perspective looks identical to "the
/// connection closed" with zero durable evidence of why.
fn install_panic_hook() {
    let default_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map_or_else(|| "<unknown location>".to_string(), |l| format!("{}:{}:{}", l.file(), l.line(), l.column()));
        let payload = info.payload();
        let message = payload
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "<non-string panic payload>".to_string());

        tracing::error!(
            target: "panic",
            location = %location,
            "PANIC: {message}"
        );

        // Still call the default hook so stderr behavior (and any test
        // harness expectations) is unchanged.
        default_hook(info);
    }));
}
