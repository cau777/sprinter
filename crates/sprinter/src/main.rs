use clap::{Parser, Subcommand};
use mimalloc::MiMalloc;
use sprinter::{auth, config, db, generation::Manager, logging, state, web};
use std::{error::Error, net::SocketAddr, sync::Arc};
use tokio::net::TcpListener;
use tracing::{error, info};

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

#[derive(Parser)]
#[command(name = "sprinter", version, about = "Self-hosted AI chat server")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Start the HTTP server (the default command).
    Serve,
    /// Return success when the local SQLite database is reachable.
    Healthcheck,
    /// Export Rust API types to the frontend TypeScript source.
    GenTypes,
}

fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
    if matches!(cli.command, Some(Command::GenTypes)) {
        return sprinter::api_types::export();
    }

    let config = config::Config::from_env()?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(config.worker_threads)
        .enable_all()
        .build()?;
    match cli.command.unwrap_or(Command::Serve) {
        Command::GenTypes => unreachable!(),
        Command::Healthcheck => runtime.block_on(healthcheck(&config)),
        Command::Serve => runtime.block_on(serve(config)),
    }
}

async fn healthcheck(config: &config::Config) -> Result<(), Box<dyn Error>> {
    let pool = db::connect(config).await?;
    sqlx::query_scalar::<_, i64>("SELECT 1")
        .fetch_one(&pool)
        .await?;
    pool.close().await;
    Ok(())
}

async fn serve(config: config::Config) -> Result<(), Box<dyn Error>> {
    let _logging_guard = logging::init(&config);
    if config.master_password.is_none() {
        return Err("SPRINTER_PASSWORD is required".into());
    }
    let pool = db::connect(&config).await?;
    sqlx::query_scalar::<_, i64>("SELECT 1")
        .fetch_one(&pool)
        .await?;

    let address: SocketAddr = config.bind_address;
    let password = config.master_password.as_ref().expect("checked above");
    auth::initialize_password(&pool, password).await?;
    let interrupted = Manager::recover_interrupted(&pool).await?;
    if interrupted > 0 {
        info!(
            count = interrupted,
            "marked unfinished generations as interrupted"
        );
    }
    let config = Arc::new(config);
    let app_state = state::AppState::new(pool, config.clone());
    let listener = TcpListener::bind(address).await?;
    info!(version = env!("CARGO_PKG_VERSION"), bind = %address,
        data_dir = %config.data_dir.display(), workers = config.worker_threads,
        log_keep_days = config.log_keep_days, backup_keep = config.backup_keep,
        insecure_cookies = config.insecure_cookies, openrouter_base_url = %config.openrouter_base_url,
        trusted_proxies = ?config.trusted_proxies,
        key_set = false, "starting");
    let generation_manager = app_state.generation.clone();
    axum::serve(
        listener,
        web::router(app_state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        shutdown_signal().await;
        tokio::spawn(async move {
            generation_manager.shutdown().await;
        });
    })
    .await?;
    info!("stopped");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(err) = tokio::signal::ctrl_c().await {
            error!(error = %err, "failed to listen for Ctrl-C");
        }
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! { _ = ctrl_c => {}, _ = terminate => {} }
}
