//! End-to-end integration test for the worker loop.
//!
//! Spins up wiremock impersonating the upstream wp-pfworkflow REST API,
//! drives one `process_once()` cycle, and asserts:
//!   - the worker called `/remote/queue/claim` with the expected body
//!   - the worker reported a `system.info` result back via `/remote/queue/<id>/result`
//!   - the X-PFW-Signature header matched HMAC-SHA256(body, bearer_token)
//!
//! This is the contract test that detects upstream-side breaking changes
//! (the executor will fail to talk to a server that drifts from the contract).

use serde_json::{json, Value};
use std::sync::Arc;
use wiremock::matchers::{header, header_exists, method, path, path_regex};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};
use wp_executor::auth::sign_body;
use wp_executor::client::UpstreamClient;
use wp_executor::config::Config;
use wp_executor::worker::Worker;

const TOKEN: &str = "pfw_worker_42_secretsecret";

fn cfg(base_url: String) -> Arc<Config> {
    Arc::new(Config {
        base_url,
        bearer_token: TOKEN.into(),
        namespace: "wp-pfworkflow/v1".into(),
        max_jobs_per_claim: 5,
        lease_seconds: 60,
        heartbeat_interval_seconds: 15,
        idle_poll_seconds: 5,
        default_job_timeout_seconds: 30,
        allowed_capabilities: None,
        sign_requests: true,
        user_agent: Some("wp-executor-test/0.1".into()),
        allowed_egress_hosts: None,
    })
}

#[tokio::test]
async fn worker_processes_a_system_info_job_end_to_end() {
    let server = MockServer::start().await;
    let cfg = cfg(server.uri());

    // Mock: claim returns one system.info job.
    let claim_response = json!({
        "workerId": 42,
        "count": 1,
        "leaseSeconds": 60,
        "jobs": [{
            "jobId": "rj_test_001",
            "capability": "system.info",
            "payload": {},
            "priority": 5,
            "timeoutSeconds": 30,
            "attemptCount": 1,
            "maxAttempts": 3,
            "queuedAt": "2026-05-01T12:00:00Z",
            "leaseUntil": "2026-05-01T12:01:00Z",
            "workflowId": 123,
            "executionId": "exec_abc",
            "nodeId": "n1"
        }]
    });

    Mock::given(method("POST"))
        .and(path("/wp-json/wp-pfworkflow/v1/remote/queue/claim"))
        .and(header(
            "authorization",
            format!("Bearer {}", TOKEN).as_str(),
        ))
        .and(header_exists("x-pfw-signature"))
        .respond_with(ResponseTemplate::new(200).set_body_json(claim_response))
        .expect(1)
        .mount(&server)
        .await;

    // Mock: result POST. Verify the body shape after the test by inspecting recorded requests.
    Mock::given(method("POST"))
        .and(path_regex(
            r"^/wp-json/wp-pfworkflow/v1/remote/queue/.+/result$",
        ))
        .and(header(
            "authorization",
            format!("Bearer {}", TOKEN).as_str(),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "status": "completed"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = UpstreamClient::new(cfg.clone()).expect("client");
    let worker = Worker::new(cfg.clone(), client);
    let processed = worker.process_once().await;
    assert_eq!(processed, 1, "expected to process exactly one job");

    // Inspect requests to validate signature + body.
    let received = server.received_requests().await.unwrap();
    assert!(
        received.len() >= 2,
        "claim + result expected, got {}",
        received.len()
    );

    // Find the result POST. Verify the body parses, has system.info-shaped output.
    let result_req = received
        .iter()
        .find(|r| r.url.path().ends_with("/result"))
        .expect("result POST not found");
    verify_signed_request(result_req);
    let body: Value = serde_json::from_slice(&result_req.body).expect("result body json");
    assert_eq!(body["exit_code"], 0);
    assert!(body["output"]["os"].is_string());
    assert_eq!(
        body["output"]["capabilities"]
            .as_array()
            .map(|a| a.len())
            .unwrap_or(0),
        6
    );
    assert!(body["output"]["memory_total_bytes"].as_u64().unwrap() > 0);

    // Find the claim POST. Verify body has max + leaseSeconds.
    let claim_req = received
        .iter()
        .find(|r| r.url.path().ends_with("/claim"))
        .expect("claim POST not found");
    verify_signed_request(claim_req);
    let claim_body: Value = serde_json::from_slice(&claim_req.body).expect("claim body json");
    assert_eq!(claim_body["max"], 5);
    assert_eq!(claim_body["leaseSeconds"], 60);
}

#[tokio::test]
async fn worker_returns_zero_when_queue_empty() {
    let server = MockServer::start().await;
    let cfg = cfg(server.uri());

    Mock::given(method("POST"))
        .and(path("/wp-json/wp-pfworkflow/v1/remote/queue/claim"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "workerId": 42,
            "count": 0,
            "leaseSeconds": 60,
            "jobs": []
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = UpstreamClient::new(cfg.clone()).expect("client");
    let worker = Worker::new(cfg, client);
    let processed = worker.process_once().await;
    assert_eq!(processed, 0);
}

#[tokio::test]
async fn worker_reports_failure_when_capability_disallowed() {
    let server = MockServer::start().await;
    let mut c = cfg(server.uri());
    Arc::get_mut(&mut c).unwrap().allowed_capabilities = Some(vec!["fs.read".into()]);

    Mock::given(method("POST"))
        .and(path("/wp-json/wp-pfworkflow/v1/remote/queue/claim"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "workerId": 42,
            "count": 1,
            "leaseSeconds": 60,
            "jobs": [{
                "jobId": "rj_blocked",
                "capability": "shell.run",
                "payload": {"command": "echo hi"}
            }]
        })))
        .expect(1)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path(
            "/wp-json/wp-pfworkflow/v1/remote/queue/rj_blocked/result",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .expect(1)
        .mount(&server)
        .await;

    let client = UpstreamClient::new(c.clone()).expect("client");
    let worker = Worker::new(c, client);
    worker.process_once().await;

    let reqs = server.received_requests().await.unwrap();
    let result_req = reqs
        .iter()
        .find(|r| r.url.path().ends_with("/rj_blocked/result"))
        .expect("result POST");
    let body: Value = serde_json::from_slice(&result_req.body).unwrap();
    assert_eq!(body["exit_code"], 1);
    assert!(
        body["error"].as_str().unwrap().contains("not allowed"),
        "error should mention disallowed: {:?}",
        body["error"]
    );
}

#[tokio::test]
async fn fs_capabilities_round_trip_via_worker() {
    use std::io::Write;

    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("worker_io.txt");
    std::fs::File::create(&target)
        .unwrap()
        .write_all(b"hi")
        .unwrap();

    let server = MockServer::start().await;
    let cfg = cfg(server.uri());

    Mock::given(method("POST"))
        .and(path("/wp-json/wp-pfworkflow/v1/remote/queue/claim"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "workerId": 42,
            "count": 1,
            "leaseSeconds": 60,
            "jobs": [{
                "jobId": "rj_fs_read",
                "capability": "fs.read",
                "payload": {"path": target.to_str().unwrap()}
            }]
        })))
        .expect(1)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path(
            "/wp-json/wp-pfworkflow/v1/remote/queue/rj_fs_read/result",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .expect(1)
        .mount(&server)
        .await;

    let client = UpstreamClient::new(cfg.clone()).expect("client");
    let worker = Worker::new(cfg, client);
    worker.process_once().await;

    let reqs = server.received_requests().await.unwrap();
    let result_req = reqs
        .iter()
        .find(|r| r.url.path().ends_with("/rj_fs_read/result"))
        .unwrap();
    let body: Value = serde_json::from_slice(&result_req.body).unwrap();
    assert_eq!(body["exit_code"], 0);
    assert_eq!(body["output"]["content"], "hi");
    assert_eq!(body["output"]["bytes_read"], 2);
}

fn verify_signed_request(req: &Request) {
    let sig_header = req
        .headers
        .get("x-pfw-signature")
        .expect("x-pfw-signature header present")
        .to_str()
        .expect("signature header is utf8");
    let expected = sign_body(TOKEN, &req.body);
    assert_eq!(
        sig_header, expected,
        "signature mismatch: header={:?} expected={:?}",
        sig_header, expected
    );
}
