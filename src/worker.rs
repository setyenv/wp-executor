use crate::caps;
use crate::client::UpstreamClient;
use crate::config::Config;
use crate::types::{Job, JobResult};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

/// Main worker loop. Polls the upstream queue, dispatches each claimed job
/// concurrently, sends heartbeats while a job runs, reports the result.
///
/// The loop runs until `shutdown_signal` resolves OR a fatal error happens
/// inside the claim call (in which case it backs off and retries).
pub struct Worker {
    cfg: Arc<Config>,
    client: UpstreamClient,
}

impl Worker {
    pub fn new(cfg: Arc<Config>, client: UpstreamClient) -> Self {
        Self { cfg, client }
    }

    /// Run a single poll cycle: claim, dispatch all jobs concurrently,
    /// wait for all to complete. Returns the number of jobs processed.
    /// Used by the integration test so it can drive one cycle without a loop.
    pub async fn process_once(&self) -> u64 {
        let claim = match self.client.claim().await {
            Ok(c) => c,
            Err(e) => {
                warn!(target: "wp_executor::worker", error = %e, "claim failed");
                return 0;
            }
        };
        if claim.jobs.is_empty() {
            return 0;
        }
        let count = claim.jobs.len() as u64;
        let mut handles: Vec<JoinHandle<()>> = Vec::with_capacity(claim.jobs.len());
        for job in claim.jobs {
            let cfg = self.cfg.clone();
            let client = self.client.clone();
            handles.push(tokio::spawn(async move {
                process_one(cfg, client, job).await;
            }));
        }
        for h in handles {
            let _ = h.await;
        }
        count
    }

    /// Run forever. `shutdown_signal` should fire on Ctrl+C / SIGTERM.
    pub async fn run_forever(self, mut shutdown_signal: watch::Receiver<bool>) {
        info!(target: "wp_executor::worker",
            base_url = %self.cfg.base_url,
            namespace = %self.cfg.namespace,
            max_jobs = self.cfg.max_jobs_per_claim,
            "starting worker loop"
        );
        loop {
            if *shutdown_signal.borrow() {
                info!(target: "wp_executor::worker", "shutdown requested, exiting loop");
                break;
            }
            let processed = self.process_once().await;
            if processed == 0 {
                let interval = self.cfg.idle_poll_seconds;
                tokio::select! {
                    _ = sleep(Duration::from_secs(interval)) => {}
                    _ = shutdown_signal.changed() => {
                        info!(target: "wp_executor::worker", "shutdown received during idle poll");
                        break;
                    }
                }
            } else {
                debug!(target: "wp_executor::worker", processed, "cycle done");
            }
        }
    }
}

async fn process_one(cfg: Arc<Config>, client: UpstreamClient, job: Job) {
    let job_id = job.job_id.clone();
    let capability = job.capability.clone();
    info!(target: "wp_executor::worker", %job_id, %capability, "executing job");

    // Reject capabilities outside the executor allowlist (defense in depth
    // — server also restricts on claim).
    if !cfg.capability_allowed(&capability) {
        let r = JobResult {
            exit_code: Some(1),
            error: format!("capability '{}' not allowed by this executor", capability),
            ..Default::default()
        };
        if let Err(e) = client.report(&job_id, &r).await {
            error!(target: "wp_executor::worker", %job_id, error = %e, "failed to report rejection");
        }
        return;
    }

    // Spawn heartbeat in background; cancel when the job finishes.
    let (tx, mut rx) = watch::channel(false);
    let heartbeat_client = client.clone();
    let heartbeat_id = job_id.clone();
    let heartbeat_interval = cfg.heartbeat_interval_seconds;
    let heartbeat_handle: JoinHandle<()> = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = sleep(Duration::from_secs(heartbeat_interval)) => {
                    match heartbeat_client.heartbeat(&heartbeat_id).await {
                        Ok(()) => debug!(target: "wp_executor::worker", job_id = %heartbeat_id, "heartbeat ok"),
                        Err(e) => warn!(target: "wp_executor::worker", job_id = %heartbeat_id, error = %e, "heartbeat failed"),
                    }
                }
                _ = rx.changed() => {
                    if *rx.borrow() {
                        break;
                    }
                }
            }
        }
    });

    // Resolve a hard timeout: payload override > job-level > executor default.
    let timeout_secs = job
        .payload
        .get("timeout_seconds")
        .and_then(|v| v.as_u64())
        .or(job.timeout_seconds)
        .unwrap_or(cfg.default_job_timeout_seconds);

    let result = match tokio::time::timeout(
        Duration::from_secs(timeout_secs),
        caps::dispatch(&capability, job.payload.clone()),
    )
    .await
    {
        Ok(r) => r,
        Err(_) => JobResult {
            exit_code: Some(1),
            error: format!("job exceeded executor timeout of {}s", timeout_secs),
            ..Default::default()
        },
    };

    // Stop heartbeat before reporting (so the lease isn't extended after we're done).
    let _ = tx.send(true);
    let _ = heartbeat_handle.await;

    if let Err(e) = client.report(&job_id, &result).await {
        error!(target: "wp_executor::worker", %job_id, error = %e, "failed to report result");
    } else {
        info!(target: "wp_executor::worker", %job_id, exit_code = ?result.exit_code, "job reported");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn cfg() -> Arc<Config> {
        Arc::new(Config {
            base_url: "https://wp.example.com".into(),
            bearer_token: "pfw_worker_1_x".into(),
            namespace: "wp-pfworkflow/v1".into(),
            max_jobs_per_claim: 1,
            lease_seconds: 60,
            heartbeat_interval_seconds: 15,
            idle_poll_seconds: 1,
            default_job_timeout_seconds: 5,
            allowed_capabilities: Some(vec!["fs.read".into()]),
            sign_requests: false,
            user_agent: None,
        })
    }

    #[test]
    fn allowlist_rejects_outside_capabilities() {
        let c = cfg();
        assert!(c.capability_allowed("fs.read"));
        assert!(!c.capability_allowed("shell.run"));
    }
}
