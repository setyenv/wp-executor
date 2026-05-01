use crate::caps::{optional_string, optional_u64, require_string};
use crate::error::{ExecutorError, Result};
use crate::types::JobResult;
use serde_json::{json, Value};
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::time::timeout;

pub async fn run(payload: Value) -> Result<JobResult> {
    let command = require_string(&payload, "command")?;
    let shell = optional_string(&payload, "shell", "auto");
    let cwd = payload
        .get("cwd")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let timeout_secs = optional_u64(&payload, "timeout_seconds", 60);
    let stdin_str = payload
        .get("stdin")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let env_overrides = payload.get("env").and_then(|v| v.as_object()).cloned();

    let (program, args) = pick_shell(&shell, &command)?;
    let mut cmd = Command::new(program);
    cmd.args(&args);
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.kill_on_drop(true);
    if let Some(dir) = &cwd {
        cmd.current_dir(dir);
    }
    if let Some(env) = env_overrides {
        for (k, v) in env {
            if let Some(value) = v.as_str() {
                cmd.env(k, value);
            }
        }
    }

    let started = Instant::now();
    let mut child = cmd
        .spawn()
        .map_err(|e| ExecutorError::Other(format!("failed to spawn process: {}", e)))?;

    if let Some(input) = stdin_str {
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(input.as_bytes()).await?;
            stdin.shutdown().await.ok();
        }
    }

    let wait = child.wait_with_output();
    let output = match timeout(Duration::from_secs(timeout_secs), wait).await {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => return Err(ExecutorError::Other(format!("wait failed: {}", e))),
        Err(_) => return Err(ExecutorError::Timeout(timeout_secs)),
    };
    let duration_ms = started.elapsed().as_millis() as u64;

    let exit_code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    Ok(JobResult {
        exit_code: Some(exit_code),
        stdout,
        stderr,
        output: Some(json!({ "duration_ms": duration_ms })),
        duration_ms: Some(duration_ms),
        error: String::new(),
    })
}

fn pick_shell(shell: &str, command: &str) -> Result<(String, Vec<String>)> {
    let resolved = if shell == "auto" {
        if cfg!(windows) {
            "cmd"
        } else {
            "bash"
        }
    } else {
        shell
    };
    match resolved {
        "cmd" => Ok(("cmd".into(), vec!["/C".into(), command.into()])),
        "powershell" => Ok((
            "powershell".into(),
            vec!["-NoProfile".into(), "-Command".into(), command.into()],
        )),
        "pwsh" => Ok((
            "pwsh".into(),
            vec!["-NoProfile".into(), "-Command".into(), command.into()],
        )),
        "bash" => Ok(("bash".into(), vec!["-c".into(), command.into()])),
        "sh" => Ok(("sh".into(), vec!["-c".into(), command.into()])),
        other => Err(ExecutorError::InvalidPayload(format!(
            "unsupported shell '{}'",
            other
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn echo_command_succeeds() {
        let payload = if cfg!(windows) {
            json!({ "command": "echo hello", "shell": "cmd" })
        } else {
            json!({ "command": "echo hello", "shell": "sh" })
        };
        let r = run(payload).await.unwrap();
        assert_eq!(r.exit_code, Some(0));
        assert!(r.stdout.contains("hello"), "stdout was: {:?}", r.stdout);
    }

    #[tokio::test]
    async fn nonzero_exit_is_reported() {
        let payload = if cfg!(windows) {
            json!({ "command": "exit 7", "shell": "cmd" })
        } else {
            json!({ "command": "exit 7", "shell": "sh" })
        };
        let r = run(payload).await.unwrap();
        assert_eq!(r.exit_code, Some(7));
    }

    #[tokio::test]
    async fn timeout_kills_long_running_command() {
        let payload = if cfg!(windows) {
            // ping is the canonical Windows sleep equivalent.
            json!({ "command": "ping -n 60 127.0.0.1", "shell": "cmd", "timeout_seconds": 1 })
        } else {
            json!({ "command": "sleep 60", "shell": "sh", "timeout_seconds": 1 })
        };
        let err = run(payload).await.unwrap_err();
        match err {
            ExecutorError::Timeout(s) => assert_eq!(s, 1),
            other => panic!("expected Timeout, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn missing_command_field_errors() {
        let r = run(json!({})).await;
        assert!(matches!(r, Err(ExecutorError::InvalidPayload(_))));
    }

    #[tokio::test]
    async fn stdin_is_piped_in() {
        let payload = if cfg!(windows) {
            // findstr returns the lines from stdin that contain the pattern.
            json!({ "command": "findstr hello", "shell": "cmd", "stdin": "hello world\nbye\n" })
        } else {
            json!({ "command": "grep hello", "shell": "sh", "stdin": "hello world\nbye\n" })
        };
        let r = run(payload).await.unwrap();
        assert_eq!(r.exit_code, Some(0));
        assert!(r.stdout.contains("hello"));
    }

    #[test]
    fn auto_shell_picks_cmd_on_windows_else_bash() {
        let (program, _) = pick_shell("auto", "echo x").unwrap();
        if cfg!(windows) {
            assert_eq!(program, "cmd");
        } else {
            assert_eq!(program, "bash");
        }
    }

    #[test]
    fn unknown_shell_errors() {
        let err = pick_shell("zsh", "echo x").unwrap_err();
        assert!(matches!(err, ExecutorError::InvalidPayload(_)));
    }
}
