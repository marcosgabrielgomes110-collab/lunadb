mod api;
mod auth;
mod bootstrap;
mod config;
mod crypto;
mod engine;
mod error;
mod models;
mod query;
mod schema;
mod state;
mod storage;

use clap::{Parser, Subcommand};
use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "lunadb", version, about = "Banco de dados JSON via API")]
struct Cli {
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    Init,
    Serve,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config_path = resolve_config_path(&cli);
    let created_config = !config_path.exists();
    let config = if created_config {
        let config = config::Config::default();
        config.save(&config_path)?;
        config
    } else {
        config::Config::load(&config_path)?
    };
    let paths = config::Paths::from_config(&config);
    let _log_guard = init_logging(&paths, &config)?;

    if created_config {
        tracing::info!(path = %config_path.display(), "config criado");
    }
    tracing::info!(
        target: "lunadb::ops",
        version = env!("CARGO_PKG_VERSION"),
        storage = %paths.home.display(),
        config = %config_path.display(),
        log_file = %paths.log_file().display(),
        "lunadb iniciando"
    );

    bootstrap::init(&paths, &config).await?;

    match cli.command.unwrap_or(Command::Serve) {
        Command::Init => {
            println!("LunaDB inicializado em {}", paths.home.display());
            println!(
                "root: {} (senha definida em {})",
                config.root.username,
                config_path.display()
            );
            Ok(())
        }
        Command::Serve => serve(paths, config, config_path).await,
    }
}

async fn serve(
    paths: config::Paths,
    config: config::Config,
    config_path: PathBuf,
) -> anyhow::Result<()> {
    let addr = format!("{}:{}", config.server.host, config.server.port);
    let keyring = std::sync::Arc::new(crypto::Keyring::load(&paths, &config)?);
    let state =
        state::AppState::new(paths.clone(), keyring, config.clone(), config_path);
    write_status(&state).await?;

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!(
        target: "lunadb::ops",
        host = %config.server.host,
        port = config.server.port,
        "LunaDB ouvindo em http://{addr}"
    );
    let app = api::router(state);
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    if let Err(err) = tokio::fs::remove_file(paths.status_file()).await {
        tracing::warn!("falha ao remover {}: {err}", paths.status_file().display());
    }
    tracing::info!(target: "lunadb::ops", "LunaDB encerrado");
    Ok(())
}

async fn write_status(state: &state::AppState) -> anyhow::Result<()> {
    let info = serde_json::json!({
        "pid": std::process::id(),
        "version": env!("CARGO_PKG_VERSION"),
        "started_at": state.started_at,
        "host": state.config.server.host,
        "port": state.config.server.port,
        "storage": state.paths.home,
        "config": state.config_path,
        "log_file": state.paths.log_file(),
        "log_rotation": state.config.logging.rotation,
        "log_max_files": state.config.logging.max_files,
    });
    storage::write_json(&state.paths.status_file(), &info).await?;
    tracing::debug!(
        path = %state.paths.status_file().display(),
        "status registrado"
    );
    Ok(())
}

fn init_logging(
    paths: &config::Paths,
    config: &config::Config,
) -> anyhow::Result<Option<tracing_appender::non_blocking::WorkerGuard>> {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let level = config.logging.level.as_str();
    let default_filter = format!("lunadb={level},tower_http={level},axum=warn");
    let filter = std::env::var("RUST_LOG")
        .ok()
        .and_then(|value| tracing_subscriber::EnvFilter::try_new(value).ok())
        .unwrap_or_else(|| {
            tracing_subscriber::EnvFilter::try_new(default_filter)
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("lunadb=info"))
        });

    let console_layer = config
        .logging
        .console
        .then(|| tracing_subscriber::fmt::layer().with_ansi(true));

    let mut guard = None;
    let file_layer = if config.logging.file {
        std::fs::create_dir_all(paths.log_dir())?;
        let rotation = match config.logging.rotation.as_str() {
            "never" => tracing_appender::rolling::Rotation::NEVER,
            "hourly" => tracing_appender::rolling::Rotation::HOURLY,
            _ => tracing_appender::rolling::Rotation::DAILY,
        };
        let appender = tracing_appender::rolling::RollingFileAppender::builder()
            .rotation(rotation)
            .filename_prefix("lunadb.log")
            .max_log_files(config.logging.max_files.max(1))
            .build(paths.log_dir())?;
        let (writer, worker_guard) = tracing_appender::non_blocking(appender);
        guard = Some(worker_guard);
        Some(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(writer),
        )
    } else {
        None
    };

    tracing_subscriber::registry()
        .with(filter)
        .with(console_layer)
        .with(file_layer)
        .init();
    Ok(guard)
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{SignalKind, signal};
        if let Ok(mut stream) = signal(SignalKind::terminate()) {
            stream.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    tracing::info!("sinal de encerramento recebido");
}

fn resolve_config_path(cli: &Cli) -> PathBuf {
    if let Some(path) = &cli.config {
        return path.clone();
    }
    if let Ok(path) = std::env::var("LUNADB_CONFIG") {
        return PathBuf::from(path);
    }
    config::expand_home("~/.lunadb/config.toml")
}
