use crate::auth::sign_body;
use crate::config::Config;
use crate::error::{ExecutorError, Result};
use crate::types::{ClaimResponse, JobResult};
use reqwest::header::{
    HeaderMap, HeaderName, HeaderValue, AUTHORIZATION, CONTENT_TYPE, USER_AGENT,
};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, warn};

/// Header name used to carry the body HMAC. Mirrors
/// RemoteWorkerHelper::SIGNATURE_HEADER upstream. We send the lowercase
/// form below since reqwest normalises header names that way; this constant
/// is the documented contract value used by the test in `mod tests`.
pub const SIGNATURE_HEADER: &str = "X-PFW-Signature";

/// Thin HTTP client over the upstream REST contract. Handles bearer token,
/// optional HMAC body signing, and JSON encoding. One instance per process is
/// fine (reqwest::Client is internally reference-counted and connection-pooled).
#[derive(Clone)]
pub struct UpstreamClient {
    inner: reqwest::Client,
    cfg: Arc<Config>,
}

impl UpstreamClient {
    pub fn new(cfg: Arc<Config>) -> Result<Self> {
        let inner = reqwest::Client::builder()
            .user_agent(cfg.user_agent())
            .timeout(Duration::from_secs(60))
            .gzip(true)
            .build()?;
        Ok(Self { inner, cfg })
    }

    /// POST /remote/queue/claim
    pub async fn claim(&self) -> Result<ClaimResponse> {
        let body = json!({
            "max": self.cfg.max_jobs_per_claim,
            "leaseSeconds": self.cfg.lease_seconds,
        });
        let resp = self
            .post_json(&self.cfg.endpoint("remote/queue/claim"), body)
            .await?;
        let claim: ClaimResponse = serde_json::from_value(resp)?;
        Ok(claim)
    }

    /// POST /remote/queue/{jobId}/heartbeat
    pub async fn heartbeat(&self, job_id: &str) -> Result<()> {
        let body = json!({ "extendSeconds": self.cfg.lease_seconds });
        let path = format!("remote/queue/{}/heartbeat", job_id);
        let _ = self.post_json(&self.cfg.endpoint(&path), body).await?;
        Ok(())
    }

    /// POST /remote/queue/{jobId}/result
    pub async fn report(&self, job_id: &str, result: &JobResult) -> Result<()> {
        let body = serde_json::to_value(result)?;
        let path = format!("remote/queue/{}/result", job_id);
        let _ = self.post_json(&self.cfg.endpoint(&path), body).await?;
        Ok(())
    }

    /// GET /remote/contract — used by the `probe` subcommand.
    pub async fn fetch_contract(&self) -> Result<Value> {
        let url = self.cfg.endpoint("remote/contract");
        let mut headers = self.base_headers()?;
        // Body is empty for GET; sign empty bytes if signing is on.
        if self.cfg.sign_requests {
            headers.insert(
                HeaderName::from_static("x-pfw-signature"),
                HeaderValue::from_str(&sign_body(&self.cfg.bearer_token, b""))
                    .map_err(|e| ExecutorError::Auth(e.to_string()))?,
            );
        }
        let resp = self.inner.get(&url).headers(headers).send().await?;
        Self::ensure_2xx(resp).await
    }

    async fn post_json(&self, url: &str, body: Value) -> Result<Value> {
        let body_bytes = serde_json::to_vec(&body)?;
        let mut headers = self.base_headers()?;
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if self.cfg.sign_requests {
            let sig = sign_body(&self.cfg.bearer_token, &body_bytes);
            headers.insert(
                HeaderName::from_static("x-pfw-signature"),
                HeaderValue::from_str(&sig).map_err(|e| ExecutorError::Auth(e.to_string()))?,
            );
        }
        debug!(target: "wp_executor::client", url = %url, "POST");
        let resp = self
            .inner
            .post(url)
            .headers(headers)
            .body(body_bytes)
            .send()
            .await?;
        Self::ensure_2xx(resp).await
    }

    fn base_headers(&self) -> Result<HeaderMap> {
        let mut h = HeaderMap::new();
        let bearer = format!("Bearer {}", self.cfg.bearer_token);
        h.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&bearer).map_err(|e| ExecutorError::Auth(e.to_string()))?,
        );
        h.insert(
            USER_AGENT,
            HeaderValue::from_str(&self.cfg.user_agent())
                .map_err(|e| ExecutorError::Auth(e.to_string()))?,
        );
        Ok(h)
    }

    async fn ensure_2xx(resp: reqwest::Response) -> Result<Value> {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            warn!(target: "wp_executor::client", status = %status, body = %body, "non-2xx upstream response");
            return Err(ExecutorError::UpstreamStatus {
                status: status.as_u16(),
                body,
            });
        }
        if body.is_empty() {
            return Ok(Value::Null);
        }
        let value: Value = serde_json::from_str(&body)?;
        Ok(value)
    }
}

/// Verify the signature header on a body using the same algorithm. Used
/// by tests + the wiremock fixture; not used by the executor itself but
/// exposed so tests can assert request signing without re-implementing.
pub fn verify_signature(bearer_token: &str, body: &[u8], header_value: &str) -> bool {
    sign_body(bearer_token, body) == header_value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_helper_round_trips() {
        let body = b"{}";
        let sig = sign_body("tok", body);
        assert!(verify_signature("tok", body, &sig));
        assert!(!verify_signature("other", body, &sig));
    }

    #[test]
    fn signature_header_constant_matches_contract() {
        // The contract publishes this header name in RemoteWorkerHelper::SIGNATURE_HEADER.
        // Keep the case-insensitive equality with the lowercase form used by reqwest.
        assert_eq!(SIGNATURE_HEADER.to_lowercase(), "x-pfw-signature");
    }
}
