use crate::caps::{system_info, SUPPORTED_CAPABILITIES};
use crate::client::UpstreamClient;
use crate::config::{default_config_path, load_from_file, Config};
use crate::error::{ExecutorError, Result};
use crate::worker::Worker;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::signal;
use tokio::sync::watch;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(
    name = "wp-executor",
    version,
    about = "Remote command executor for the wp-pfworkflow plugin's remote queue.",
    long_about = None,
)]
pub struct Cli {
    /// Path to the TOML config file. Defaults to the platform user-config
    /// dir (Linux: ~/.config/wp-executor/config.toml, macOS: ~/Library/...,
    /// Windows: %APPDATA%\wp-executor\config.toml).
    #[arg(long, env = "WP_EXECUTOR_CONFIG", global = true)]
    pub config: Option<PathBuf>,

    /// Override the upstream base URL (e.g. https://wp.example.com).
    #[arg(long, env = "WP_EXECUTOR_BASE_URL", global = true)]
    pub base_url: Option<String>,

    /// Override the bearer token (`pfw_worker_<id>_<secret>`). Prefer the
    /// config file or env var; passing on the CLI exposes the secret to ps.
    #[arg(long, env = "WP_EXECUTOR_TOKEN", global = true)]
    pub token: Option<String>,

    /// Tracing filter (e.g. `info`, `wp_executor=debug`). Defaults to
    /// `wp_executor=info` if unset.
    #[arg(long, env = "RUST_LOG", global = true)]
    pub log: Option<String>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run the worker loop until SIGINT / SIGTERM / Ctrl+C.
    Run,

    /// One-shot: hit `/remote/contract` and print the response. Verifies
    /// network reach + token validity without claiming any jobs.
    Probe,

    /// Print the resolved configuration (with the token redacted) and exit.
    /// Useful for debugging the env / file precedence.
    ShowConfig,

    /// Print this executor's `system.info` payload to stdout. Equivalent
    /// to running the system.info capability locally; no upstream call.
    SystemInfo,

    /// List the capabilities this binary implements.
    Capabilities,
}

pub async fn run() -> Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.log.as_deref());
    system_info::mark_process_started();

    let cfg = build_config(&cli)?;
    let cfg = Arc::new(cfg);

    match cli.command {
        Command::Capabilities => {
            for c in SUPPORTED_CAPABILITIES {
                println!("{}", c);
            }
            Ok(())
        }
        Command::SystemInfo => {
            let r = system_info::run(serde_json::Value::Null).await?;
            println!("{}", serde_json::to_string_pretty(&r.output.unwrap())?);
            Ok(())
        }
        Command::ShowConfig => {
            let mut redacted = (*cfg).clone();
            redacted.bearer_token = redact_token(&redacted.bearer_token);
            println!(
                "{}",
                toml::to_string_pretty(&redacted)
                    .map_err(|e| ExecutorError::Other(e.to_string()))?
            );
            Ok(())
        }
        Command::Probe => {
            cfg.validate()?;
            let client = UpstreamClient::new(cfg.clone())?;
            let contract = client.fetch_contract().await?;
            println!("{}", serde_json::to_string_pretty(&contract)?);
            Ok(())
        }
        Command::Run => {
            cfg.validate()?;
            let client = UpstreamClient::new(cfg.clone())?;
            let worker = Worker::new(cfg.clone(), client);
            let (tx, rx) = watch::channel(false);
            tokio::spawn(async move {
                if let Err(e) = wait_for_shutdown().await {
                    error!(target: "wp_executor::cli", error = %e, "shutdown signal listener failed");
                }
                let _ = tx.send(true);
            });
            worker.run_forever(rx).await;
            info!(target: "wp_executor::cli", "worker loop ended");
            Ok(())
        }
    }
}

fn build_config(cli: &Cli) -> Result<Config> {
    let path = cli
        .config
        .clone()
        .or_else(default_config_path)
        .ok_or_else(|| ExecutorError::Config("cannot resolve a default config path".into()))?;

    // Load file if it exists; otherwise build a minimal in-memory config from
    // CLI/env overrides only (useful for `wp-executor probe --base-url=... --token=...`
    // without writing a file).
    let mut cfg = if path.exists() {
        load_from_file(&path)?
    } else {
        Config {
            base_url: String::new(),
            bearer_token: String::new(),
            namespace: "wp-pfworkflow/v1".into(),
            max_jobs_per_claim: 5,
            lease_seconds: 60,
            heartbeat_interval_seconds: 15,
            idle_poll_seconds: 5,
            default_job_timeout_seconds: 300,
            allowed_capabilities: None,
            sign_requests: true,
            user_agent: None,
        }
    };

    if let Some(url) = &cli.base_url {
        cfg.base_url = url.clone();
    }
    if let Some(t) = &cli.token {
        cfg.bearer_token = t.clone();
    }
    Ok(cfg)
}

fn redact_token(token: &str) -> String {
    // Token format: pfw_worker_<id>_<secret>. Show id, mask secret.
    let parts: Vec<&str> = token.splitn(4, '_').collect();
    if parts.len() == 4 {
        format!("{}_{}_{}_<redacted>", parts[0], parts[1], parts[2])
    } else if !token.is_empty() {
        "<redacted>".into()
    } else {
        String::new()
    }
}

fn init_tracing(log_filter: Option<&str>) {
    let filter = log_filter
        .map(|s| EnvFilter::try_new(s).unwrap_or_else(|_| EnvFilter::new("wp_executor=info")))
        .unwrap_or_else(|| {
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("wp_executor=info"))
        });
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .try_init();
}

#[cfg(unix)]
async fn wait_for_shutdown() -> Result<()> {
    use signal::unix::{signal as unix_signal, SignalKind};
    let mut sigterm = unix_signal(SignalKind::terminate())
        .map_err(|e| ExecutorError::Other(format!("install SIGTERM listener: {}", e)))?;
    let mut sigint = unix_signal(SignalKind::interrupt())
        .map_err(|e| ExecutorError::Other(format!("install SIGINT listener: {}", e)))?;
    tokio::select! {
        _ = sigterm.recv() => info!(target: "wp_executor::cli", "received SIGTERM"),
        _ = sigint.recv() => info!(target: "wp_executor::cli", "received SIGINT"),
        _ = signal::ctrl_c() => info!(target: "wp_executor::cli", "received Ctrl+C"),
    }
    Ok(())
}

#[cfg(not(unix))]
async fn wait_for_shutdown() -> Result<()> {
    signal::ctrl_c()
        .await
        .map_err(|e| ExecutorError::Other(format!("ctrl-c listener: {}", e)))?;
    info!(target: "wp_executor::cli", "received Ctrl+C");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_token_id_visible() {
        let r = redact_token("pfw_worker_42_abcdefghi");
        assert_eq!(r, "pfw_worker_42_<redacted>");
    }

    #[test]
    fn redacts_unparseable_token_fully() {
        assert_eq!(redact_token("nope"), "<redacted>");
        assert_eq!(redact_token(""), "");
    }

    #[test]
    fn cli_parses_run_command() {
        let cli = Cli::parse_from(["wp-executor", "run"]);
        matches!(cli.command, Command::Run);
    }

    #[test]
    fn cli_parses_probe_with_overrides() {
        let cli = Cli::parse_from([
            "wp-executor",
            "--base-url=https://wp.example.com",
            "--token=pfw_worker_1_secret",
            "probe",
        ]);
        assert_eq!(cli.base_url.as_deref(), Some("https://wp.example.com"));
        assert!(matches!(cli.command, Command::Probe));
    }
}
