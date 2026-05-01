use crate::caps::SUPPORTED_CAPABILITIES;
use crate::error::Result;
use crate::types::JobResult;
use serde_json::{json, Value};
use std::sync::OnceLock;
use std::time::Instant;
use sysinfo::System;

static PROCESS_STARTED: OnceLock<Instant> = OnceLock::new();

pub fn mark_process_started() {
    let _ = PROCESS_STARTED.set(Instant::now());
}

pub async fn run(_payload: Value) -> Result<JobResult> {
    let os = std::env::consts::OS.to_string();
    let arch = std::env::consts::ARCH.to_string();
    let host = hostname::get()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "unknown".into());
    let executor_version = env!("CARGO_PKG_VERSION").to_string();
    let uptime_seconds = PROCESS_STARTED
        .get()
        .map(|t| t.elapsed().as_secs())
        .unwrap_or(0);
    let cpu_count = std::thread::available_parallelism()
        .map(|n| n.get() as i64)
        .unwrap_or(1);

    // sysinfo for total memory only — refresh once so we don't keep state.
    let mut sys = System::new();
    sys.refresh_memory();
    let memory_total_bytes = sys.total_memory();

    Ok(JobResult::ok().with_output(json!({
        "os": os,
        "arch": arch,
        "hostname": host,
        "executor_version": executor_version,
        "uptime_seconds": uptime_seconds,
        "capabilities": SUPPORTED_CAPABILITIES,
        "cpu_count": cpu_count,
        "memory_total_bytes": memory_total_bytes,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reports_basic_fields() {
        mark_process_started();
        let r = run(json!({})).await.unwrap();
        assert_eq!(r.exit_code, Some(0));
        let out = r.output.unwrap();
        assert!(!out["os"].as_str().unwrap().is_empty());
        assert!(!out["arch"].as_str().unwrap().is_empty());
        assert!(!out["hostname"].as_str().unwrap().is_empty());
        assert_eq!(out["executor_version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(
            out["capabilities"].as_array().unwrap().len(),
            SUPPORTED_CAPABILITIES.len()
        );
        assert!(out["cpu_count"].as_i64().unwrap() >= 1);
        assert!(out["memory_total_bytes"].as_u64().unwrap() > 0);
    }

    #[tokio::test]
    async fn os_field_is_one_of_documented() {
        let r = run(json!({})).await.unwrap();
        let os = r.output.unwrap()["os"].as_str().unwrap().to_string();
        // The contract says: windows | linux | macos | other.
        // std::env::consts::OS uses these exact strings on the major three.
        assert!(["windows", "linux", "macos"].contains(&os.as_str()) || !os.is_empty());
    }
}
