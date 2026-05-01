use crate::caps::{optional_bool, optional_u64, require_string};
use crate::error::Result;
use crate::types::JobResult;
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub async fn run(payload: Value) -> Result<JobResult> {
    let path = require_string(&payload, "path")?;
    let recursive = optional_bool(&payload, "recursive", false);
    let max_entries = optional_u64(&payload, "max_entries", 1000) as usize;
    let include_hidden = optional_bool(&payload, "include_hidden", false);

    let mut entries: Vec<Value> = Vec::new();
    let mut truncated = false;

    let root = PathBuf::from(&path);
    let mut queue: VecDeque<PathBuf> = VecDeque::new();
    queue.push_back(root.clone());

    while let Some(dir) = queue.pop_front() {
        let is_root = dir == root;
        let mut rd = match tokio::fs::read_dir(&dir).await {
            Ok(rd) => rd,
            Err(e) => {
                // Surface the error on the root only; sub-dirs that fail get skipped.
                if is_root {
                    return Err(e.into());
                }
                continue;
            }
        };
        while let Some(entry) = rd.next_entry().await? {
            let name = entry.file_name().to_string_lossy().to_string();
            if !include_hidden && is_hidden_name(&name) {
                continue;
            }
            let full = entry.path();
            let metadata = match entry.metadata().await {
                Ok(m) => m,
                Err(_) => continue,
            };
            let is_dir = metadata.is_dir();
            let size = if metadata.is_file() {
                metadata.len() as i64
            } else {
                0
            };
            let modified = metadata
                .modified()
                .ok()
                .and_then(systime_to_iso8601)
                .unwrap_or_default();
            entries.push(json!({
                "name": name,
                "path": full.display().to_string(),
                "is_dir": is_dir,
                "size": size,
                "modified_at": modified,
            }));
            if entries.len() >= max_entries {
                truncated = true;
                break;
            }
            if recursive && is_dir {
                queue.push_back(full);
            }
        }
        if truncated {
            break;
        }
    }

    Ok(JobResult::ok().with_output(json!({
        "entries": entries,
        "truncated": truncated,
        "count": entries.len(),
    })))
}

fn is_hidden_name(name: &str) -> bool {
    // POSIX-style + Windows-style hidden are detected by the leading dot.
    // True Windows hidden attribute requires a winapi call we deliberately
    // skip for portability; dot-prefix matches the contract's convention.
    name.starts_with('.')
}

fn systime_to_iso8601(t: SystemTime) -> Option<String> {
    let dt: DateTime<Utc> = t.into();
    Some(dt.to_rfc3339())
}

// Suppresses an unused-import warning on Windows-only builds.
#[allow(dead_code)]
fn _path_unused(_: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(path: &Path, contents: &[u8]) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, contents).unwrap();
    }

    #[tokio::test]
    async fn lists_top_level_only_by_default() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("a.txt"), b"a");
        touch(&dir.path().join("sub/b.txt"), b"b");
        let r = run(json!({ "path": dir.path().to_str().unwrap() }))
            .await
            .unwrap();
        let out = r.output.unwrap();
        let entries = out["entries"].as_array().unwrap();
        let names: Vec<String> = entries
            .iter()
            .map(|e| e["name"].as_str().unwrap().to_string())
            .collect();
        assert!(names.contains(&"a.txt".to_string()));
        assert!(names.contains(&"sub".to_string()));
        // Sub-directory contents are not listed.
        assert!(!names.contains(&"b.txt".to_string()));
    }

    #[tokio::test]
    async fn recursive_includes_descendants() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("a.txt"), b"a");
        touch(&dir.path().join("sub/b.txt"), b"b");
        let r = run(json!({
            "path": dir.path().to_str().unwrap(),
            "recursive": true,
        }))
        .await
        .unwrap();
        let entries = r.output.unwrap()["entries"].as_array().unwrap().clone();
        let names: Vec<String> = entries
            .iter()
            .map(|e| e["name"].as_str().unwrap().to_string())
            .collect();
        assert!(names.contains(&"b.txt".to_string()));
    }

    #[tokio::test]
    async fn hides_dot_files_by_default() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join(".hidden"), b"x");
        touch(&dir.path().join("visible.txt"), b"y");
        let r = run(json!({ "path": dir.path().to_str().unwrap() }))
            .await
            .unwrap();
        let names: Vec<String> = r.output.unwrap()["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["name"].as_str().unwrap().to_string())
            .collect();
        assert!(!names.contains(&".hidden".to_string()));
        assert!(names.contains(&"visible.txt".to_string()));
    }

    #[tokio::test]
    async fn include_hidden_shows_dot_files() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join(".hidden"), b"x");
        let r = run(json!({
            "path": dir.path().to_str().unwrap(),
            "include_hidden": true,
        }))
        .await
        .unwrap();
        let names: Vec<String> = r.output.unwrap()["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["name"].as_str().unwrap().to_string())
            .collect();
        assert!(names.contains(&".hidden".to_string()));
    }

    #[tokio::test]
    async fn truncated_when_over_max_entries() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..20 {
            touch(&dir.path().join(format!("f{:02}.txt", i)), b"x");
        }
        let r = run(json!({
            "path": dir.path().to_str().unwrap(),
            "max_entries": 5,
        }))
        .await
        .unwrap();
        let out = r.output.unwrap();
        assert_eq!(out["entries"].as_array().unwrap().len(), 5);
        assert_eq!(out["truncated"], true);
    }

    #[tokio::test]
    async fn missing_path_errors() {
        let r = run(json!({})).await;
        assert!(r.is_err());
    }
}
