use crate::caps::{optional_bool, optional_string, require_string};
use crate::error::{ExecutorError, Result};
use crate::types::JobResult;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use serde_json::{json, Value};
use std::path::PathBuf;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;

pub async fn run(payload: Value) -> Result<JobResult> {
    let path = require_string(&payload, "path")?;
    let content = require_string(&payload, "content")?;
    let encoding = optional_string(&payload, "encoding", "utf8");
    let mode = optional_string(&payload, "mode", "overwrite");
    let create_dirs = optional_bool(&payload, "create_dirs", true);

    let bytes: Vec<u8> = match encoding.as_str() {
        "utf8" => content.into_bytes(),
        "base64" => B64
            .decode(content.as_bytes())
            .map_err(|e| ExecutorError::InvalidPayload(format!("invalid base64: {}", e)))?,
        other => {
            return Err(ExecutorError::InvalidPayload(format!(
                "unsupported encoding '{}'",
                other
            )))
        }
    };

    let path_buf = PathBuf::from(&path);
    if create_dirs {
        if let Some(parent) = path_buf.parent() {
            if !parent.as_os_str().is_empty() {
                tokio::fs::create_dir_all(parent).await?;
            }
        }
    }

    let mut opts = OpenOptions::new();
    match mode.as_str() {
        "overwrite" => {
            opts.write(true).create(true).truncate(true);
        }
        "append" => {
            opts.write(true).create(true).append(true);
        }
        "create_only" => {
            opts.write(true).create_new(true);
        }
        other => {
            return Err(ExecutorError::InvalidPayload(format!(
                "unsupported mode '{}'",
                other
            )))
        }
    }

    let mut file = opts.open(&path_buf).await?;
    file.write_all(&bytes).await?;
    file.flush().await?;

    let absolute = std::fs::canonicalize(&path_buf)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| path_buf.display().to_string());

    Ok(JobResult::ok().with_output(json!({
        "bytes_written": bytes.len(),
        "path": absolute,
        "mode": mode,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn writes_utf8_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.txt");
        let r = run(json!({ "path": path.to_str().unwrap(), "content": "hi" }))
            .await
            .unwrap();
        assert_eq!(r.exit_code, Some(0));
        assert_eq!(r.output.unwrap()["bytes_written"], 2);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hi");
    }

    #[tokio::test]
    async fn appends_when_mode_append() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log.txt");
        std::fs::write(&path, "line1\n").unwrap();
        run(json!({
            "path": path.to_str().unwrap(),
            "content": "line2\n",
            "mode": "append",
        }))
        .await
        .unwrap();
        let body = std::fs::read_to_string(&path).unwrap();
        assert_eq!(body, "line1\nline2\n");
    }

    #[tokio::test]
    async fn create_only_refuses_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("locked.txt");
        std::fs::write(&path, "x").unwrap();
        let r = run(json!({
            "path": path.to_str().unwrap(),
            "content": "y",
            "mode": "create_only",
        }))
        .await;
        assert!(matches!(r, Err(ExecutorError::Io(_))));
    }

    #[tokio::test]
    async fn writes_base64_decoded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("blob.bin");
        run(json!({
            "path": path.to_str().unwrap(),
            "content": "/wBC",
            "encoding": "base64",
        }))
        .await
        .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), vec![0xff, 0x00, 0x42]);
    }

    #[tokio::test]
    async fn create_dirs_makes_parents() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a/b/c/file.txt");
        run(json!({
            "path": nested.to_str().unwrap(),
            "content": "ok",
        }))
        .await
        .unwrap();
        assert_eq!(std::fs::read_to_string(&nested).unwrap(), "ok");
    }

    #[tokio::test]
    async fn missing_required_fields_error() {
        let r = run(json!({ "path": "/tmp/x" })).await;
        assert!(matches!(r, Err(ExecutorError::InvalidPayload(_))));
        let r = run(json!({ "content": "hi" })).await;
        assert!(matches!(r, Err(ExecutorError::InvalidPayload(_))));
    }

    #[tokio::test]
    async fn invalid_base64_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.bin");
        let r = run(json!({
            "path": path.to_str().unwrap(),
            "content": "not_valid_base64!!!",
            "encoding": "base64",
        }))
        .await;
        assert!(matches!(r, Err(ExecutorError::InvalidPayload(_))));
    }
}
