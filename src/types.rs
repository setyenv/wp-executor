use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Job claimed from the upstream queue. Mirrors `jobShape` in
/// RemoteContractHelper. Field naming is camelCase on the wire (matches
/// the WordPress REST response from RemoteQueueHelper::format_job_for_response).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub job_id: String,
    pub capability: String,
    #[serde(default)]
    pub payload: Value,
    #[serde(default)]
    pub priority: Option<i64>,
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
    #[serde(default)]
    pub attempt_count: Option<i64>,
    #[serde(default)]
    pub max_attempts: Option<i64>,
    #[serde(default)]
    pub queued_at: Option<String>,
    #[serde(default)]
    pub lease_until: Option<String>,
    /// Upstream stores additional fields like workflowId / executionId / nodeId.
    /// We accept anything extra without failing.
    #[serde(flatten, default)]
    pub extra: std::collections::BTreeMap<String, Value>,
}

/// Server response to POST /remote/queue/claim.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimResponse {
    pub worker_id: i64,
    pub count: i64,
    #[serde(default = "default_lease")]
    pub lease_seconds: u64,
    #[serde(default)]
    pub jobs: Vec<Job>,
}

fn default_lease() -> u64 {
    60
}

/// Body POSTed to /remote/queue/{jobId}/result. Server accepts both snake
/// and camelCase keys; we use snake to match RemoteContractHelper exactly.
#[derive(Debug, Clone, Serialize, Default)]
pub struct JobResult {
    /// 0 = success. Non-zero / `null` = failure reason. `null` allowed for
    /// capabilities where exit code does not apply.
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub stdout: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub stderr: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Non-empty only on infrastructure errors (timeout, IO failure, etc.).
    /// A capability handler that ran the action and got a non-zero exit puts
    /// that in `exit_code`, NOT in `error`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub error: String,
}

impl JobResult {
    pub fn ok() -> Self {
        Self {
            exit_code: Some(0),
            ..Default::default()
        }
    }

    pub fn failed_infra(error: impl Into<String>) -> Self {
        Self {
            exit_code: Some(1),
            error: error.into(),
            ..Default::default()
        }
    }

    pub fn with_output(mut self, output: Value) -> Self {
        self.output = Some(output);
        self
    }

    pub fn with_duration(mut self, duration_ms: u64) -> Self {
        self.duration_ms = Some(duration_ms);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_deserialises_minimum_fields() {
        let raw = r#"{"jobId":"rj_1","capability":"shell.run","payload":{"command":"echo hi"}}"#;
        let job: Job = serde_json::from_str(raw).unwrap();
        assert_eq!(job.job_id, "rj_1");
        assert_eq!(job.capability, "shell.run");
        assert_eq!(job.payload["command"], "echo hi");
    }

    #[test]
    fn job_keeps_extra_camelcase_fields() {
        let raw = r#"{"jobId":"rj_2","capability":"system.info","payload":{},"workflowId":42,"nodeId":"n1"}"#;
        let job: Job = serde_json::from_str(raw).unwrap();
        assert_eq!(job.extra.get("workflowId").unwrap(), &serde_json::json!(42));
        assert_eq!(job.extra.get("nodeId").unwrap(), &serde_json::json!("n1"));
    }

    #[test]
    fn claim_response_parses() {
        let raw = r#"{"workerId":7,"count":1,"leaseSeconds":60,"jobs":[{"jobId":"rj_1","capability":"system.info","payload":{}}]}"#;
        let resp: ClaimResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(resp.worker_id, 7);
        assert_eq!(resp.lease_seconds, 60);
        assert_eq!(resp.jobs.len(), 1);
    }

    #[test]
    fn job_result_omits_empty_strings() {
        let r = JobResult {
            exit_code: Some(0),
            stdout: "out".into(),
            ..Default::default()
        };
        let s = serde_json::to_string(&r).unwrap();
        assert!(s.contains("\"stdout\":\"out\""));
        assert!(!s.contains("\"stderr\""));
        assert!(!s.contains("\"error\""));
    }
}
