use crate::caps::{optional_string, optional_u64, require_string};
use crate::error::{ExecutorError, Result};
use crate::types::JobResult;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, CONTENT_TYPE};
use reqwest::Method;
use serde_json::{json, Value};
use std::str::FromStr;
use std::time::{Duration, Instant};

pub async fn run(payload: Value) -> Result<JobResult> {
    let url = require_string(&payload, "url")?;
    let method = optional_string(&payload, "method", "GET").to_uppercase();
    let timeout_secs = optional_u64(&payload, "timeout_seconds", 30);

    let method = Method::from_str(&method)
        .map_err(|_| ExecutorError::InvalidPayload(format!("invalid HTTP method '{}'", method)))?;

    let headers = build_headers(payload.get("headers"))?;
    let body = payload.get("body");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .gzip(true)
        .build()?;
    let mut req = client.request(method, &url).headers(headers.clone());

    if let Some(body) = body {
        if let Some(s) = body.as_str() {
            req = req.body(s.to_string());
        } else if !body.is_null() {
            // Object/array body -> JSON encode automatically.
            req = req.json(body);
        }
    }

    let started = Instant::now();
    let resp = req.send().await?;
    let status = resp.status().as_u16();

    let resp_headers: serde_json::Map<String, Value> = resp
        .headers()
        .iter()
        .map(|(k, v)| {
            (
                k.as_str().to_string(),
                Value::String(v.to_str().unwrap_or("").to_string()),
            )
        })
        .collect();

    let body_text = resp.text().await.unwrap_or_default();
    let duration_ms = started.elapsed().as_millis() as u64;

    let json: Option<Value> = serde_json::from_str(&body_text).ok();

    // exit_code conventions:
    //   0 = upstream returned a 2xx OR 3xx response (i.e. the request itself succeeded)
    //   non-zero (= status code) = upstream returned 4xx/5xx
    // For network failures we never reach this point — they short-circuit
    // through the `resp.send().await?` ? operator above and become an
    // `ExecutorError::Http`, which the caps dispatcher turns into
    // `JobResult { exit_code: 1, error: ... }`.
    let exit_code = if (200..400).contains(&status) {
        0
    } else {
        status as i32
    };

    Ok(JobResult {
        exit_code: Some(exit_code),
        stdout: String::new(),
        stderr: String::new(),
        output: Some(json!({
            "status_code": status,
            "headers": resp_headers,
            "body": body_text,
            "json": json,
            "duration_ms": duration_ms,
        })),
        duration_ms: Some(duration_ms),
        error: String::new(),
    })
}

fn build_headers(raw: Option<&Value>) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    let map = match raw.and_then(|v| v.as_object()) {
        Some(m) => m,
        None => return Ok(headers),
    };
    for (k, v) in map {
        let value = match v.as_str() {
            Some(s) => s.to_string(),
            None => v.to_string(),
        };
        let name = HeaderName::from_bytes(k.as_bytes()).map_err(|e| {
            ExecutorError::InvalidPayload(format!("invalid header name '{}': {}", k, e))
        })?;
        let val = HeaderValue::from_str(&value).map_err(|e| {
            ExecutorError::InvalidPayload(format!("invalid header value for {}: {}", k, e))
        })?;
        headers.insert(name, val);
    }
    if !headers.contains_key(CONTENT_TYPE) && map.contains_key("content-type") {
        // Already inserted under any case; just an idempotency safeguard.
    }
    Ok(headers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn successful_get_returns_exit_zero_and_body() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/ping"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/json")
                    .set_body_string(r#"{"ok":true}"#),
            )
            .mount(&server)
            .await;

        let url = format!("{}/ping", server.uri());
        let r = run(json!({ "url": url })).await.unwrap();
        assert_eq!(r.exit_code, Some(0));
        let out = r.output.unwrap();
        assert_eq!(out["status_code"], 200);
        assert_eq!(out["json"]["ok"], true);
    }

    #[tokio::test]
    async fn http_error_status_propagates_as_exit_code() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/notfound"))
            .respond_with(ResponseTemplate::new(404).set_body_string("nope"))
            .mount(&server)
            .await;
        let url = format!("{}/notfound", server.uri());
        let r = run(json!({ "url": url })).await.unwrap();
        assert_eq!(r.exit_code, Some(404));
    }

    #[tokio::test]
    async fn post_with_json_body_works() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/echo"))
            .and(header("content-type", "application/json"))
            .respond_with(ResponseTemplate::new(201).set_body_string(r#"{"created":true}"#))
            .mount(&server)
            .await;
        let url = format!("{}/echo", server.uri());
        let r = run(json!({
            "url": url,
            "method": "POST",
            "body": { "name": "alice" },
        }))
        .await
        .unwrap();
        assert_eq!(r.exit_code, Some(0));
        assert_eq!(r.output.unwrap()["status_code"], 201);
    }

    #[tokio::test]
    async fn invalid_method_errors() {
        // HTTP allows custom method tokens (any token chars) so "NOPE" parses.
        // To force a parse error use a method string with invalid token chars
        // such as a space.
        let r = run(json!({ "url": "http://x", "method": "GET BAD" })).await;
        assert!(matches!(r, Err(ExecutorError::InvalidPayload(_))));
    }

    #[tokio::test]
    async fn missing_url_errors() {
        let r = run(json!({})).await;
        assert!(matches!(r, Err(ExecutorError::InvalidPayload(_))));
    }

    #[tokio::test]
    async fn header_round_trip() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/auth"))
            .and(header("authorization", "Bearer abc"))
            .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
            .mount(&server)
            .await;
        let url = format!("{}/auth", server.uri());
        let r = run(json!({
            "url": url,
            "headers": { "Authorization": "Bearer abc" },
        }))
        .await
        .unwrap();
        assert_eq!(r.exit_code, Some(0));
    }
}
