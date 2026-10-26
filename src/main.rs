use anyhow::bail;
use clap::{Parser, Subcommand};
use lunadb::{
    api, auth, bootstrap, client as http_client, config, crypto, engine, models, shell, state, storage,
};
use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "lunadb", version, about = "JSON/MessagePack database via API")]
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
    Backup {
        #[arg(long, short = 'o')]
        out: Option<PathBuf>,
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
            runtime()?
                .block_on(async { bootstrap::init(&paths, &mut config, &config_path).await })?;
            println!("LunaDB initialized at {}", paths.home.display());
            println!(
                "root: {} (password set in {})",
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
                anyhow::anyhow!("provide --key or set LUNADB_API_KEY to manage api keys")
            })?;
            run_apikey(base_url, key, action)
        }
        Command::Backup { out } => {
            let base_url = server_override.unwrap_or_else(|| default_base_url(&config));
            backup_command(&paths, &config, base_url, key_override, out)
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
        tracing::info!(path = %config_path.display(), "config created");
    }
    tracing::info!(
        target: "lunadb::ops",
        version = env!("CARGO_PKG_VERSION"),
        storage = %paths.home.display(),
        config = %config_path.display(),
        log_file = %paths.log_file().display(),
        "lunadb starting"
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
    let state = state::AppState::new(paths.clone(), keyring, config.clone(), config_path);
    write_status(&state).await?;

    let (scheme, _tls_ready) = if config.server.tls.enabled {
        match (&config.server.tls.cert_file, &config.server.tls.key_file) {
            (Some(cert), Some(key)) => {
                let cert_p = config::expand_home(cert);
                let key_p = config::expand_home(key);
                if !cert_p.exists() {
                    anyhow::bail!("TLS cert_file not found: {}", cert_p.display());
                }
                if !key_p.exists() {
                    anyhow::bail!("TLS key_file not found: {}", key_p.display());
                }
                ("https", true)
            }
            _ => anyhow::bail!("TLS is enabled but cert_file and/or key_file are not specified"),
        }
    } else {
        ("http", false)
    };

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!(
        target: "lunadb::ops",
        host = %config.server.host,
        port = config.server.port,
        tls = config.server.tls.enabled,
        "LunaDB listening on {scheme}://{addr}"
    );
    let app = api::router(state);
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    if let Err(err) = tokio::fs::remove_file(paths.status_file()).await {
        tracing::warn!("failed to remove {}: {err}", paths.status_file().display());
    }
    tracing::info!(target: "lunadb::ops", "LunaDB stopped");
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
        "status recorded"
    );
    Ok(())
}

fn migrate_command(paths: &config::Paths, to: &str, force: bool) -> anyhow::Result<()> {
    let target = match to {
        "json" => storage::Format::Json,
        "msgpack" => storage::Format::Msgpack,
        other => bail!("invalid format: '{other}' (use json or msgpack)"),
    };
    if !force && server_running(paths) {
        bail!(
            "server appears to be running ({}); stop it before migrating or use --force",
            paths.status_file().display()
        );
    }
    let report = runtime()?.block_on(engine::migrate_tables(paths, target))?;
    println!(
        "converted tables: {} | already in format {}: {}",
        report.converted.len(),
        target.extension(),
        report.skipped.len()
    );
    for label in &report.converted {
        println!("  converted: {label}");
    }
    Ok(())
}

fn backup_command(
    paths: &config::Paths,
    config: &config::Config,
    base_url: String,
    key_override: Option<String>,
    out: Option<PathBuf>,
) -> anyhow::Result<()> {
    let dest_dir = match out {
        Some(dir) => dir,
        None => {
            let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
            paths.home.join("backups").join(format!("snapshot_{timestamp}"))
        }
    };

    if server_running(paths) {
        // Online backup via HTTP API
        let key = key_override.ok_or_else(|| {
            anyhow::anyhow!("server is running; provide --key or set LUNADB_API_KEY with admin access for online backup")
        })?;
        let client = http_client::Client::new(base_url, Some(key));
        let body = serde_json::json!({
            "destination": dest_dir.display().to_string(),
        });
        let res = client.post("/snapshot", body)?;
        println!("online backup created successfully:");
        println!("{}", serde_json::to_string_pretty(&res)?);
    } else {
        // Offline consistent backup
        let keyring = std::sync::Arc::new(crypto::Keyring::load(paths, config)?);
        let config_path = paths.home.join("config.toml");
        let state = state::AppState::new(paths.clone(), keyring, config.clone(), config_path);
        let dummy_admin = models::User {
            id: "root".to_string(),
            username: "root".to_string(),
            password_hash: String::new(),
            api_key_hash: None,
            api_keys: Vec::new(),
            role: models::ROLE_ADMIN.to_string(),
            grants: Vec::new(),
            created_at: chrono::Utc::now(),
        };
        let auth_user = auth::AuthUser(dummy_admin);
        let report = runtime()?.block_on(engine::create_snapshot(&state, &auth_user, &dest_dir))?;
        println!("offline backup created successfully:");
        println!("{}", serde_json::to_string_pretty(&report)?);
    }
    Ok(())
}

fn server_running(paths: &config::Paths) -> bool {
    let Some(status) = read_status(paths) else {
        return false;
    };
    let Some(pid) = status.get("pid").and_then(|pid| pid.as_u64()) else {
        return false;
    };
    process_alive(pid)
}

fn read_status(paths: &config::Paths) -> Option<serde_json::Value> {
    let raw = std::fs::read_to_string(paths.status_file()).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Whether `pid` is a live process. The status file can outlive a crashed server,
/// so a bare file check is not enough.
#[cfg(target_os = "linux")]
fn process_alive(pid: u64) -> bool {
    std::path::Path::new(&format!("/proc/{pid}")).exists()
}

#[cfg(target_os = "macos")]
fn process_alive(pid: u64) -> bool {
    // kill(pid, 0) succeeds for live processes and fails with ESRCH otherwise;
    // EPERM means the process exists but belongs to another user.
    let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(windows)]
fn process_alive(pid: u64) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid as u32);
        if handle.is_null() {
            return false;
        }
        let mut code = 0u32;
        let ok = GetExitCodeProcess(handle, &mut code) != 0;
        CloseHandle(handle);
        // STILL_ACTIVE is an NTSTATUS (i32) with the value 259, so compare as i32.
        ok && code as i32 == STILL_ACTIVE
    }
}

/// Fallback for other Unix platforms (and for builds without extra dependencies):
/// the process is considered alive only while its own status file is present.
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn process_alive(_pid: u64) -> bool {
    true
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
        .ok_or_else(|| anyhow::anyhow!("user '{username}' not found"))
}

fn default_base_url(config: &config::Config) -> String {
    let scheme = if config.server.tls.enabled {
        "https"
    } else {
        "http"
    };
    format!("{scheme}://{}:{}", config.server.host, config.server.port)
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
    tracing::info!("shutdown signal received");
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

#[cfg(test)]
mod tests {
    use super::*;

    fn paths_in(dir: &std::path::Path) -> config::Paths {
        let mut config = config::Config::default();
        config.storage.path = dir.to_string_lossy().to_string();
        config::Paths::from_config(&config)
    }

    /// A pid that cannot be running, chosen within the range each platform
    /// accepts. Linux pids are bounded by `pid_max` (4194304 by default) and
    /// macOS/BSD use a signed `pid_t`, where a value above `i32::MAX` would
    /// wrap to a negative number and hit the `kill` special cases.
    fn dead_pid() -> u64 {
        if cfg!(target_os = "linux") {
            // beyond the default pid_max, so /proc/<pid> cannot exist
            u32::MAX as u64
        } else {
            // valid positive range, but no process will ever hold these
            (i32::MAX - 1) as u64
        }
    }

    #[test]
    fn own_process_is_alive() {
        assert!(process_alive(std::process::id() as u64));
    }

    #[test]
    fn missing_status_file_is_not_running() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!server_running(&paths_in(dir.path())));
    }

    #[test]
    fn stale_status_file_does_not_report_running() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_in(dir.path());
        runtime().unwrap().block_on(paths.ensure()).unwrap();
        runtime()
            .unwrap()
            .block_on(storage::write_json(
                &paths.status_file(),
                &serde_json::json!({"pid": dead_pid(), "version": "0"}),
            ))
            .unwrap();
        assert!(
            !server_running(&paths),
            "a leftover status file must not block migrate"
        );
    }

    #[test]
    fn malformed_status_file_is_not_running() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_in(dir.path());
        runtime().unwrap().block_on(paths.ensure()).unwrap();
        std::fs::write(paths.status_file(), b"not json").unwrap();
        assert!(!server_running(&paths));
    }
}
