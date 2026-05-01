//! Capability dispatch. Each capability is a free async function that takes
//! the JSON payload and returns a `JobResult`. The runtime list of supported
//! capabilities lives in `SUPPORTED_CAPABILITIES`; it is what we register
//! with the upstream when a worker is created and what we reject jobs against
//! at execute time.

use crate::error::{ExecutorError, Result};
use crate::types::JobResult;
use serde_json::Value;
use std::time::Instant;
use tracing::{debug, info};

mod fs_list;
mod fs_read;
mod fs_write;
mod http_request;
mod shell_run;
pub mod system_info;

/// Canonical list of capabilities this executor implements. Mirrors
/// `RemoteContractHelper::capabilities_spec()` upstream and
/// `RemoteWorkerHelper::supported_capabilities()` for worker registration.
pub const SUPPORTED_CAPABILITIES: &[&str] = &[
    "shell.run",
    "fs.read",
    "fs.write",
    "fs.list",
    "http.request",
    "system.info",
];

/// Dispatch a job to the appropriate handler. Wraps the result with timing
/// and translates any handler error into a typed `JobResult` so the worker
/// can always report something coherent to the upstream.
pub async fn dispatch(capability: &str, payload: Value) -> JobResult {
    let started = Instant::now();
    info!(target: "wp_executor::caps", capability = capability, "dispatching");
    let outcome = match capability {
        "shell.run" => shell_run::run(payload).await,
        "fs.read" => fs_read::run(payload).await,
        "fs.write" => fs_write::run(payload).await,
        "fs.list" => fs_list::run(payload).await,
        "http.request" => http_request::run(payload).await,
        "system.info" => system_info::run(payload).await,
        other => Err(ExecutorError::UnsupportedCapability(other.into())),
    };
    let duration_ms = started.elapsed().as_millis() as u64;
    debug!(target: "wp_executor::caps", capability = capability, duration_ms, "dispatched");

    match outcome {
        Ok(mut r) => {
            if r.duration_ms.is_none() {
                r = r.with_duration(duration_ms);
            }
            r
        }
        Err(e) => JobResult {
            exit_code: Some(1),
            error: e.to_string(),
            duration_ms: Some(duration_ms),
            ..Default::default()
        },
    }
}

/// Light input-validation helper used across capability handlers. Pulls a
/// required string field from the payload object.
pub(crate) fn require_string(payload: &Value, field: &str) -> Result<String> {
    payload
        .get(field)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| ExecutorError::InvalidPayload(format!("missing required field '{}'", field)))
}

/// Optional string with default.
pub(crate) fn optional_string(payload: &Value, field: &str, default: &str) -> String {
    payload
        .get(field)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| default.to_string())
}

/// Optional u64 with default.
pub(crate) fn optional_u64(payload: &Value, field: &str, default: u64) -> u64 {
    payload
        .get(field)
        .and_then(|v| v.as_u64())
        .unwrap_or(default)
}

/// Optional bool with default.
pub(crate) fn optional_bool(payload: &Value, field: &str, default: bool) -> bool {
    payload
        .get(field)
        .and_then(|v| v.as_bool())
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn dispatch_unknown_capability_returns_error_result() {
        let r = dispatch("nope.does_not_exist", json!({})).await;
        assert_eq!(r.exit_code, Some(1));
        assert!(r.error.contains("nope.does_not_exist"));
    }

    #[test]
    fn supported_list_is_complete() {
        // If a capability is added or removed, this must be updated; the
        // honest_classification rule requires the executor's catalogue to
        // match the upstream's RemoteWorkerHelper::supported_capabilities().
        assert_eq!(SUPPORTED_CAPABILITIES.len(), 6);
        for k in [
            "shell.run",
            "fs.read",
            "fs.write",
            "fs.list",
            "http.request",
            "system.info",
        ] {
            assert!(SUPPORTED_CAPABILITIES.contains(&k), "missing {}", k);
        }
    }

    #[test]
    fn require_string_returns_field() {
        let p = json!({"path": "/tmp/x"});
        assert_eq!(require_string(&p, "path").unwrap(), "/tmp/x");
    }

    #[test]
    fn require_string_errors_when_missing() {
        let p = json!({});
        let err = require_string(&p, "path").unwrap_err();
        match err {
            ExecutorError::InvalidPayload(msg) => assert!(msg.contains("path")),
            _ => panic!("wrong error type"),
        }
    }

    #[test]
    fn optional_helpers_use_defaults() {
        let p = json!({});
        assert_eq!(optional_string(&p, "x", "y"), "y");
        assert_eq!(optional_u64(&p, "n", 10), 10);
        assert!(optional_bool(&p, "b", true));
    }
}
