// main.rs — amity-service binary entrypoint.
//
// Responsibilities:
//   1. Initialise structured logging (tracing-subscriber).
//   2. Load configuration from the platform config file (or defaults).
//   3. Open (and migrate) the database.
//   4. Build the axum application.
//   5. Bind to the configured address and start serving.
//
// Everything except the bind/serve step lives in the library crate so that
// integration tests can reuse the same application setup without a network.

// `build_app` owns the axum router; `load_config` reads the config file.
// `open_database` is imported here rather than re-exported by amity-service
// to keep the binary responsible for its own startup sequence.
use amity_service::{build_app, config::load_config};
use amity_storage::connection::open_database;
// `ProjectDirs` resolves the platform data directory for the auto-create step.
use directories::ProjectDirs;
// `EnvFilter` drives `RUST_LOG` environment variable parsing for tracing.
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialise structured logging. The `RUST_LOG` environment variable
    // controls the filter (e.g. `RUST_LOG=amity_service=debug`).
    // Default to `info` so the service is observable without extra config.
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    // Ensure the data directory exists before loading config or opening the DB.
    // On first run `~/.local/share/amity/` (XDG) / `~/Library/Application Support/amity/`
    // (macOS) / `%APPDATA%\amity\` (Windows) may not exist; `create_dir_all`
    // is idempotent so it is safe to call on every startup.
    if let Some(dirs) = ProjectDirs::from("", "amity", "amity") {
        let data_dir = dirs.data_dir();
        std::fs::create_dir_all(data_dir).map_err(|e| {
            anyhow::anyhow!(
                "failed to create data directory {}: {e}",
                data_dir.display()
            )
        })?;
        tracing::debug!(path = %data_dir.display(), "data directory ready");
    }

    // Errors here are configuration parse failures — non-recoverable at startup.
    let config = load_config()?;

    tracing::info!(
        bind = %config.server.bind_address,
        port = config.server.port,
        db   = %config.database.url,
        "amity-service starting"
    );

    // Open the database and apply any pending migrations.
    // Migrations are embedded at compile time; a fresh database is fully set up
    // on first run without any manual migration step.
    let db = open_database(&config.database.url).await?;

    // `build_app` wires the axum router with the database pool as shared state.
    // The pool is Arc-wrapped inside SqlitePool so cloning it shares the connection.
    let app = build_app(db);

    // Construct the bind address from config. Loopback-only by default
    // so the service is not reachable from other machines without explicit
    // reconfiguration — appropriate for a household hub device.
    let bind_addr = format!("{}:{}", config.server.bind_address, config.server.port);
    // Bind the TCP socket; fails fast if the port is already in use.
    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;

    tracing::info!(address = %bind_addr, "listening");

    // `serve` blocks until the process exits or an unrecoverable I/O error occurs.
    // On normal shutdown (SIGTERM / SIGINT) the OS cleans up the socket.
    axum::serve(listener, app).await?;

    // Return Ok to satisfy anyhow::Result; unreachable in practice since
    // `serve` only returns on I/O error, which propagates via `?` above.
    Ok(())
}
