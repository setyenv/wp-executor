use crate::error::{ExecutorError, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Runtime config loaded from a TOML file (default location: platform user
/// config dir) and overridable via env vars / CLI flags.
///
/// Minimum set required to run: `base_url` + `bearer_token`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    /// Site root, e.g. `https://wp.example.com`. The executor appends
    /// `/wp-json/wp-pfworkflow/v1/...` itself; do NOT include the namespace.
    pub base_url: String,

    /// Full bearer token issued by `POST /remote/workers` on the upstream.
    /// Format: `pfw_worker_<id>_<secret>`.
    pub bearer_token: String,

    /// REST namespace path (with version). Default matches the current
    /// upstream contract. Override only if the upstream's
    /// `PFW_REST_NAMESPACE` constant changes (or you add a v2).
    #[serde(default = "default_namespace")]
    pub namespace: String,

    /// Maximum jobs to claim per poll. Server clamps to MAX_CLAIM_BATCH (25).
    #[serde(default = "default_max_jobs")]
    pub max_jobs_per_claim: u32,

    /// Lease duration we ask the server to grant when claiming. Heartbeat
    /// extends this every `heartbeat_interval_seconds`. Defaults match the
    /// contract's `polling.leaseSeconds` (60s).
    #[serde(default = "default_lease")]
    pub lease_seconds: u64,

    /// How often we send a heartbeat while a job is running. Should be
    /// well below `lease_seconds`. Contract recommends 15s.
    #[serde(default = "default_heartbeat_interval")]
    pub heartbeat_interval_seconds: u64,

    /// Sleep duration between poll cycles when the server returns no jobs.
    /// Contract's `polling.recommendedIntervalSeconds` is 5s.
    #[serde(default = "default_idle_poll")]
    pub idle_poll_seconds: u64,

    /// Hard timeout per job execution on the executor side. Capability
    /// payloads can override (e.g. shell.run with timeout_seconds=600).
    #[serde(default = "default_job_timeout")]
    pub default_job_timeout_seconds: u64,

    /// Optional capability allowlist. If set, jobs whose `capability` is not
    /// in this list are rejected with a typed error and the result reports
    /// `pfw_capability_not_allowed`. If `None`, ALL implemented capabilities
    /// are allowed. The allowlist is the executor's safety net; the upstream
    /// also enforces a per-worker capability set when claiming.
    #[serde(default)]
    pub allowed_capabilities: Option<Vec<String>>,

    /// Whether to send the optional X-PFW-Signature HMAC header on every
    /// outgoing request. Defaults to true (defense in depth, recommended
    /// by the contract).
    #[serde(default = "default_true")]
    pub sign_requests: bool,

    /// Outbound egress allowlist for capabilities that make network requests
    /// (currently `http.request`). The SSRF guard is ON by default: requests
    /// to private, loopback, link-local (incl. the cloud metadata endpoint
    /// 169.254.169.254) or otherwise non-global addresses are rejected. Hosts
    /// (or patterns like `*.example.com`) listed here are exempted from that
    /// block, so specific internal destinations you trust remain reachable. A
    /// single entry `"*"` disables the guard entirely. When `None`/empty the
    /// guard blocks all private/internal destinations and allows public ones.
    #[serde(default)]
    pub allowed_egress_hosts: Option<Vec<String>>,

    /// Override the user-agent string. Default identifies the executor +
    /// version + OS so server logs can audit the agent population.
    #[serde(default)]
    pub user_agent: Option<String>,
}

fn default_namespace() -> String {
    "wp-pfworkflow/v1".to_string()
}

fn default_max_jobs() -> u32 {
    5
}

fn default_lease() -> u64 {
    60
}

fn default_heartbeat_interval() -> u64 {
    15
}

fn default_idle_poll() -> u64 {
    5
}

fn default_job_timeout() -> u64 {
    300
}

fn default_true() -> bool {
    true
}

impl Config {
    pub fn endpoint(&self, path: &str) -> String {
        format!(
            "{}/wp-json/{}/{}",
            self.base_url.trim_end_matches('/'),
            self.namespace.trim_matches('/'),
            path.trim_start_matches('/')
        )
    }

    pub fn user_agent(&self) -> String {
        self.user_agent.clone().unwrap_or_else(|| {
            format!(
                "wp-executor/{} ({} {})",
                env!("CARGO_PKG_VERSION"),
                std::env::consts::OS,
                std::env::consts::ARCH
            )
        })
    }

    pub fn capability_allowed(&self, key: &str) -> bool {
        match &self.allowed_capabilities {
            None => true,
            Some(list) => list.iter().any(|c| c == key),
        }
    }

    /// The outbound egress allowlist as a slice (empty when unset — the SSRF
    /// guard then blocks all private/internal destinations and allows public
    /// ones).
    pub fn egress_allowlist(&self) -> &[String] {
        self.allowed_egress_hosts.as_deref().unwrap_or(&[])
    }

    pub fn validate(&self) -> Result<()> {
        if self.base_url.is_empty() {
            return Err(ExecutorError::Config("base_url is empty".into()));
        }
        if !self.base_url.starts_with("http://") && !self.base_url.starts_with("https://") {
            return Err(ExecutorError::Config(
                "base_url must start with http:// or https://".into(),
            ));
        }
        if self.bearer_token.is_empty() {
            return Err(ExecutorError::Config("bearer_token is empty".into()));
        }
        if !self.bearer_token.starts_with("pfw_worker_") {
            return Err(ExecutorError::Config(
                "bearer_token must start with pfw_worker_<id>_<secret>".into(),
            ));
        }
        if self.heartbeat_interval_seconds >= self.lease_seconds {
            return Err(ExecutorError::Config(format!(
                "heartbeat_interval_seconds ({}) must be less than lease_seconds ({})",
                self.heartbeat_interval_seconds, self.lease_seconds
            )));
        }
        Ok(())
    }
}

/// Resolve the default config file path for the current platform.
///
/// - Linux: `$XDG_CONFIG_HOME/wp-executor/config.toml` (or `~/.config/...`)
/// - macOS: `~/Library/Application Support/wp-executor/config.toml`
/// - Windows: `%APPDATA%\wp-executor\config.toml`
pub fn default_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("wp-executor").join("config.toml"))
}

pub fn load_from_file(path: &Path) -> Result<Config> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| ExecutorError::Config(format!("cannot read {}: {}", path.display(), e)))?;
    let cfg: Config = toml::from_str(&raw)?;
    cfg.validate()?;
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Config {
        Config {
            base_url: "https://wp.example.com".into(),
            bearer_token: "pfw_worker_1_secret".into(),
            namespace: default_namespace(),
            max_jobs_per_claim: default_max_jobs(),
            lease_seconds: default_lease(),
            heartbeat_interval_seconds: default_heartbeat_interval(),
            idle_poll_seconds: default_idle_poll(),
            default_job_timeout_seconds: default_job_timeout(),
            allowed_capabilities: None,
            sign_requests: true,
            user_agent: None,
            allowed_egress_hosts: None,
        }
    }

    #[test]
    fn endpoint_assembles_correctly() {
        let cfg = fixture();
        assert_eq!(
            cfg.endpoint("remote/queue/claim"),
            "https://wp.example.com/wp-json/wp-pfworkflow/v1/remote/queue/claim"
        );
        assert_eq!(
            cfg.endpoint("/remote/queue/abc/heartbeat"),
            "https://wp.example.com/wp-json/wp-pfworkflow/v1/remote/queue/abc/heartbeat"
        );
    }

    #[test]
    fn endpoint_handles_trailing_base_slash() {
        let mut cfg = fixture();
        cfg.base_url = "https://wp.example.com/".into();
        assert_eq!(
            cfg.endpoint("remote/contract"),
            "https://wp.example.com/wp-json/wp-pfworkflow/v1/remote/contract"
        );
    }

    #[test]
    fn validate_rejects_empty_token() {
        let mut cfg = fixture();
        cfg.bearer_token = String::new();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_rejects_non_pfw_token() {
        let mut cfg = fixture();
        cfg.bearer_token = "Bearer xxx".into();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_rejects_bad_heartbeat_relation() {
        let mut cfg = fixture();
        cfg.heartbeat_interval_seconds = cfg.lease_seconds;
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn capability_allowed_when_no_allowlist() {
        let cfg = fixture();
        assert!(cfg.capability_allowed("shell.run"));
    }

    #[test]
    fn capability_allowed_only_in_allowlist() {
        let mut cfg = fixture();
        cfg.allowed_capabilities = Some(vec!["fs.read".into(), "system.info".into()]);
        assert!(cfg.capability_allowed("fs.read"));
        assert!(!cfg.capability_allowed("shell.run"));
    }

    #[test]
    fn user_agent_default_contains_version_and_os() {
        let cfg = fixture();
        let ua = cfg.user_agent();
        assert!(ua.starts_with("wp-executor/"));
        assert!(ua.contains(std::env::consts::OS));
    }

    #[test]
    fn loads_from_toml_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"base_url = "https://wp.example.com"
bearer_token = "pfw_worker_42_zzz""#,
        )
        .unwrap();
        let cfg = load_from_file(&path).unwrap();
        assert_eq!(cfg.base_url, "https://wp.example.com");
        assert_eq!(cfg.namespace, "wp-pfworkflow/v1");
        assert_eq!(cfg.lease_seconds, 60);
    }

    #[test]
    fn loads_with_overrides() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"base_url = "https://wp.example.com"
bearer_token = "pfw_worker_1_x"
max_jobs_per_claim = 10
lease_seconds = 120
heartbeat_interval_seconds = 30
allowed_capabilities = ["shell.run", "fs.read"]
sign_requests = false"#,
        )
        .unwrap();
        let cfg = load_from_file(&path).unwrap();
        assert_eq!(cfg.max_jobs_per_claim, 10);
        assert_eq!(cfg.lease_seconds, 120);
        assert_eq!(cfg.allowed_capabilities.as_ref().unwrap().len(), 2);
        assert!(!cfg.sign_requests);
    }
}
