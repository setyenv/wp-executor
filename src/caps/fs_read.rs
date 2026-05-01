use crate::caps::{optional_string, optional_u64, require_string};
use crate::error::{ExecutorError, Result};
use crate::types::JobResult;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use serde_json::{json, Value};

pub async fn run(payload: Value) -> Result<JobResult> {
    let path = require_string(&payload, "path")?;
    let encoding = optional_string(&payload, "encoding", "utf8");
    let max_bytes = optional_u64(&payload, "max_bytes", 10 * 1024 * 1024);

    let bytes = tokio::fs::read(&path).await?;
    if bytes.len() as u64 > max_bytes {
        return Err(ExecutorError::InvalidPayload(format!(
            "file size {} exceeds max_bytes {}",
            bytes.len(),
            max_bytes
        )));
    }

    let mime = mime_guess::from_path(&path)
        .first_or_octet_stream()
        .to_string();
    let bytes_read = bytes.len() as u64;

    let content = match encoding.as_str() {
        "utf8" => String::from_utf8(bytes)
            .map_err(|e| ExecutorError::InvalidPayload(format!("invalid utf8: {}", e)))?,
        "base64" => B64.encode(&bytes),
        other => {
            return Err(ExecutorError::InvalidPayload(format!(
                "unsupported encoding '{}'",
                other
            )))
        }
    };

    Ok(JobResult::ok().with_output(json!({
        "content": content,
        "bytes_read": bytes_read,
        "mime_type": mime,
        "encoding": encoding,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[tokio::test]
    async fn reads_utf8_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hello.txt");
        std::fs::File::create(&path)
            .unwrap()
            .write_all(b"hello world")
            .unwrap();
        let r = run(json!({ "path": path.to_str().unwrap() }))
            .await
            .unwrap();
        assert_eq!(r.exit_code, Some(0));
        let out = r.output.unwrap();
        assert_eq!(out["content"], "hello world");
        assert_eq!(out["bytes_read"], 11);
    }

    #[tokio::test]
    async fn reads_binary_as_base64() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("blob.bin");
        std::fs::File::create(&path)
            .unwrap()
            .write_all(&[0xff, 0x00, 0x42])
            .unwrap();
        let r = run(json!({ "path": path.to_str().unwrap(), "encoding": "base64" }))
            .await
            .unwrap();
        let out = r.output.unwrap();
        assert_eq!(out["content"], "/wBC");
        assert_eq!(out["bytes_read"], 3);
    }

    #[tokio::test]
    async fn missing_path_errors() {
        let r = run(json!({})).await;
        assert!(matches!(r, Err(ExecutorError::InvalidPayload(_))));
    }

    #[tokio::test]
    async fn missing_file_returns_io_error() {
        let r = run(json!({ "path": "/no/such/file/should/exist/plz/9876.txt" })).await;
        assert!(matches!(r, Err(ExecutorError::Io(_))));
    }

    #[tokio::test]
    async fn rejects_when_over_max_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.txt");
        std::fs::write(&path, vec![b'a'; 1024]).unwrap();
        let r = run(json!({ "path": path.to_str().unwrap(), "max_bytes": 100 })).await;
        assert!(matches!(r, Err(ExecutorError::InvalidPayload(_))));
    }

    #[tokio::test]
    async fn unknown_encoding_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, b"x").unwrap();
        let r = run(json!({ "path": path.to_str().unwrap(), "encoding": "rot13" })).await;
        assert!(matches!(r, Err(ExecutorError::InvalidPayload(_))));
    }
}
