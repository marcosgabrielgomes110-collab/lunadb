use anyhow::bail;
use clap::{Parser, Subcommand};
use lunadb::{
    api, bootstrap, client as http_client, config, crypto, engine, shell, state, storage,
};
use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "lunadb", version, about = "Banco de dados JSON/MessagePack via API")]
struct Cli {
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[arg(long, global = true)]
    server: Option<String>,
    #[arg(long, global = true)]
    key: Option<String>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    Init,
    Serve,
    Shell,
    #[command(name = "apikey")]
    ApiKey {
        #[command(subcommand)]
        action: ApiKeyAction,
    },
    Migrate {
        #[arg(long, default_value = "msgpack")]
        to: String,
        #[arg(long)]
        force: bool,
    },
}

#[derive(Debug, Subcommand)]
enum ApiKeyAction {
    Create {
        user: String,
        #[arg(long)]
        name: Option<String>,
    },
    List {
        user: Option<String>,
    },
    Revoke {
        user: String,
        #[arg(value_name = "KEY")]
        key_ref: String,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let server_override = cli.server.clone();
    let key_override = resolve_key(&cli);
    let config_path = resolve_config_path(&cli);
    let created_config = !config_path.exists();
    let mut config = if created_config {
        let config = config::Config::default();
        config.save(&config_path)?;
        config
    } else {
        config::Config::load(&config_path)?
    };
    let paths = config::Paths::from_config(&config);

    match cli.command.unwrap_or(Command::Serve) {
        Command::Serve => {
            let _guard = init_logging(&paths, &config)?;
            log_startup(&paths, &config, &config_path, created_config);
            runtime()?.block_on(async {
                bootstrap::init(&paths, &mut config, &config_path).await?;
                serve(paths, config, config_path).await
            })
        }
        Command::Init => {
            let _guard = init_logging(&paths, &config)?;
            runtime()?.block_on(async {
                bootstrap::init(&paths, &mut config, &config_path).await
            })?;
            println!("LunaDB inicializado em {}", paths.home.display());
            println!(
                "root: {} (senha definida em {})",
                config.root.username,
                config_path.display()
            );
            Ok(())
        }
        Command::Migrate { to, force } => migrate_command(&paths, &to, force),
        Command::Shell => {
            let base_url = server_override.unwrap_or_else(|| default_base_url(&config));
            shell::run(&paths, base_url, key_override)
        }
        Command::ApiKey { action } => {
            let base_url = server_override.unwrap_or_else(|| default_base_url(&config));
            let key = key_override.ok_or_else(|| {
                anyhow::anyhow!("informe --key ou defina LUNADB_API_KEY para gerenciar api keys")
            })?;
            run_apikey(base_url, key, action)
        }
    }
}

fn log_startup(
    paths: &config::Paths,
    _config: &config::Config,
    config_path: &std::path::Path,
    created_config: bool,
) {
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
}

fn runtime() -> anyhow::Result<tokio::runtime::Runtime> {
    Ok(tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?)
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

fn migrate_command(paths: &config::Paths, to: &str, force: bool) -> anyhow::Result<()> {
    let target = match to {
        "json" => storage::Format::Json,
        "msgpack" => storage::Format::Msgpack,
        other => bail!("formato invalido: '{other}' (use json ou msgpack)"),
    };
    if !force && server_running(paths) {
        bail!(
            "servidor parece estar rodando ({}); pare antes de migrar ou use --force",
            paths.status_file().display()
        );
    }
    let report = runtime()?.block_on(engine::migrate_tables(paths, target))?;
    println!(
        "tabelas convertidas: {} | ja no formato {}: {}",
        report.converted.len(),
        target.extension(),
        report.skipped.len()
    );
    for label in &report.converted {
        println!("  convertida: {label}");
    }
    Ok(())
}

fn server_running(paths: &config::Paths) -> bool {
    let Ok(raw) = std::fs::read_to_string(paths.status_file()) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return false;
    };
    let Some(pid) = value.get("pid").and_then(|pid| pid.as_u64()) else {
        return false;
    };
    std::path::Path::new(&format!("/proc/{pid}")).exists()
}

fn run_apikey(base_url: String, api_key: String, action: ApiKeyAction) -> anyhow::Result<()> {
    let mut client = http_client::Client::new(base_url, Some(api_key));
    match action {
        ApiKeyAction::Create { user, name } => {
            let id = resolve_user_id(&mut client, &user)?;
            let value = client.post(
                &format!("/users/{id}/api-keys"),
                serde_json::json!({ "name": name }),
            )?;
            match value["key"].as_str() {
                Some(key) => println!("{key}"),
                None => println!("{}", serde_json::to_string_pretty(&value)?),
            }
        }
        ApiKeyAction::List { user } => {
            let path = match user {
                Some(user) => {
                    let id = resolve_user_id(&mut client, &user)?;
                    format!("/users/{id}/api-keys")
                }
                None => "/auth/api-keys".to_string(),
            };
            let keys = client.get(&path)?;
            println!("{}", serde_json::to_string_pretty(&keys)?);
        }
        ApiKeyAction::Revoke { user, key_ref } => {
            let id = resolve_user_id(&mut client, &user)?;
            let value = client.delete(&format!("/users/{id}/api-keys/{key_ref}"))?;
            println!("{}", serde_json::to_string_pretty(&value)?);
        }
    }
    Ok(())
}

fn resolve_user_id(client: &mut http_client::Client, username: &str) -> anyhow::Result<String> {
    if username == "self" {
        let me = client.get("/me")?;
        return Ok(me["id"].as_str().unwrap_or_default().to_string());
    }
    let users = client.get("/users")?;
    users
        .as_array()
        .and_then(|users| {
            users
                .iter()
                .find(|user| user["username"] == username)
                .and_then(|user| user["id"].as_str())
        })
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("usuario '{username}' nao encontrado"))
}

fn default_base_url(config: &config::Config) -> String {
    format!("http://{}:{}", config.server.host, config.server.port)
}

fn resolve_key(cli: &Cli) -> Option<String> {
    cli.key
        .clone()
        .or_else(|| std::env::var("LUNADB_API_KEY").ok())
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
