# WP Executor

> The remote agent for **ProjectFlash Workflow**. A small, single-binary worker written in Rust that runs the actions a WordPress site should not — system commands, filesystem operations, outbound HTTP — on a host you control.

[![ci](https://github.com/Project-Flash-Build/wp-executor/actions/workflows/ci.yml/badge.svg)](https://github.com/Project-Flash-Build/wp-executor/actions/workflows/ci.yml)
[![release](https://img.shields.io/github/v/release/Project-Flash-Build/wp-executor?display_name=tag&sort=semver)](https://github.com/Project-Flash-Build/wp-executor/releases)
[![license](https://img.shields.io/badge/license-MIT%20%2F%20Apache--2.0-blue.svg)](#license)

`wp-executor` is the open-source companion of two commercial WordPress plugins built by Project Flash. It does the things your WordPress install cannot do safely from inside a `php-fpm` request: shell commands, OS-level file IO, long network calls. The plugin tells the executor *what* to do; the executor decides *whether* to do it and reports back.

This repository contains the executor binary, install scripts for the three major desktop platforms, and the cross-platform CI that builds it.

---

## Where this fits

```
                                       results + logs
        ┌─────────────────────────────┐  ◀───────────  ┌───────────────────┐
        │   wp-pfworkflow (WP plugin) │                │   wp-executor     │
        │   visual workflow builder   │  ──────────▶   │   (this binary)   │
        └─────────────────────────────┘  job intent    └────────┬──────────┘
                       ▲                                        │
                       │ user-facing automations                │ runs in your
                       │                                        │ environment
                ┌──────┴──────┐                          ┌──────▼──────────┐
                │ WordPress   │                          │ shell · files · │
                │ admin       │                          │ HTTP · system   │
                └─────────────┘                          └─────────────────┘
```

The plugin is the canvas where customers draw automations. When an action requires a host operation that is unsafe inside WordPress (think `git pull`, `ffmpeg` encode, scheduled backup), the plugin records the intent. `wp-executor` periodically picks up that intent, runs it on its own machine, and ships the structured result back. Authentication is bearer-token plus optional HMAC body signing.

The transport details and queue semantics are intentionally not documented here — they live in the plugin's own contract, served at runtime over a versioned REST endpoint that the executor consumes on startup.

### What it shows

| <img src="assets/wp-pfworkflow-dashboard.png" alt="ProjectFlash Workflow dashboard" /> | <img src="assets/wp-pfworkflow-editor.png" alt="ProjectFlash Workflow editor" /> |
|:---:|:---:|
| `wp-pfworkflow` workflow library — visual catalog of every automation a site has, with state badges and node counts. | The visual editor where workflows are designed. Triggers, conditions, function calls, error boundaries, all visible. |

The screenshots above are taken from the upstream plugin (commercial, not open source — see [Related products](#related-products)). They're included here so the role of the executor is obvious: a workflow in the editor pushes a job; the executor runs it.

---

## Capabilities

The executor implements exactly the six capabilities the upstream plugin defines, no more, no less. Each one has a typed payload and a typed result. The plugin describes them in full at runtime; the summary below is operator-facing.

| Key | What it does | Typical use |
|---|---|---|
| `shell.run` | Execute a shell command (cmd / powershell / pwsh / bash / sh / auto) with hard timeout, optional stdin, environment overrides, working directory | Run scripts, scheduled maintenance, build pipelines, anything you'd put in `cron` |
| `fs.read` | Read a file as utf-8 or base64, with a max-bytes guard and best-effort MIME detection | Hand a config file, a log fragment or a generated artifact back to the workflow |
| `fs.write` | Write a file (overwrite / append / create-only), auto-create parent directories, base64 input ok | Drop a generated report, save a downloaded asset, stage a file before another step picks it up |
| `fs.list` | Directory listing, optional recursive, hidden filter, max-entries cap | Inventory a release directory, drive a per-file loop in the workflow |
| `http.request` | Arbitrary HTTP call (any method, any headers, JSON or string body) with timeout. Status code is surfaced as the exit code | Hit a LAN-only API, call a self-hosted service, fetch from a private host the WP server cannot reach |
| `system.info` | OS, arch, hostname, CPU count, memory, executor version, uptime, capabilities advertised | Health checks, populating workflow metadata, fleet inventory |

Every capability accepts an executor-side timeout, refuses to perform operations outside the configured allowlist, and returns a uniform `{ exit_code, stdout, stderr, output, duration_ms, error }` payload.

---

## Install

### Pre-built binaries

Each tagged release publishes binaries for Linux (x86_64), macOS (x86_64 + Apple Silicon) and Windows (x86_64). Download the archive that matches your platform from the [releases page](https://github.com/Project-Flash-Build/wp-executor/releases), unzip, and place the `wp-executor` binary somewhere on your `PATH`.

### Build from source

```bash
cargo build --release
# binary at target/release/wp-executor
```

Requires Rust 1.80 or newer. No system dependencies (TLS uses rustls).

### Run as a service

Install scripts are shipped under [`scripts/`](scripts/). They write a service definition appropriate for the platform and start the worker.

| Platform | Command |
|---|---|
| Linux (systemd) | `sudo ./scripts/install-linux.sh` (system) or `./scripts/install-linux.sh --user` |
| macOS (launchd) | `./scripts/install-macos.sh` (per-user) or `sudo ./scripts/install-macos.sh --system` |
| Windows (sc.exe) | Run elevated PowerShell: `.\scripts\install-windows.ps1` |

Each installer creates a config template if one does not exist; the worker will refuse to start until you fill in `base_url` and `bearer_token`. Matching `uninstall-*` scripts are provided.

---

## Configuration

The executor reads a TOML file from the platform's user config directory:

| Platform | Default path |
|---|---|
| Linux | `~/.config/wp-executor/config.toml` |
| macOS | `~/Library/Application Support/wp-executor/config.toml` |
| Windows | `%APPDATA%\wp-executor\config.toml` |

Override with `--config /path/to/config.toml` or `WP_EXECUTOR_CONFIG=/path`.

Minimum config:

```toml
base_url     = "https://your-wordpress-site.example.com"
bearer_token = "pfw_worker_<id>_<secret>"
```

The full set of tunables (poll interval, lease duration, allowlist, signing toggle, etc.) is documented in [`scripts/config.example.toml`](scripts/config.example.toml).

The bearer token is issued by the upstream plugin admin when a worker is registered. It is shown in plain text exactly once; rotate via the plugin's admin UI when needed. The executor never prints the secret in log lines.

---

## CLI

```text
USAGE:
    wp-executor [OPTIONS] <COMMAND>

COMMANDS:
    run            Start the worker loop until SIGINT / SIGTERM / Ctrl+C
    probe          One-shot connectivity check against the upstream contract endpoint
    show-config    Print the resolved configuration with the token redacted
    system-info    Print the local system.info payload (no upstream call)
    capabilities   List the capabilities this binary implements
```

Quick health check before installing as a service:

```bash
wp-executor --base-url=https://your-site.tld --token=pfw_worker_1_xxx probe
```

A successful probe prints the upstream contract document and exits zero.

---

## Security model

- All upstream calls authenticate with a worker-specific bearer token and (by default) carry an `X-PFW-Signature` HMAC-SHA256 of the request body. Disable the second factor only if you understand the trade-off.
- Capabilities run with the privileges of the user the executor process runs as. Install as a dedicated low-privilege user where possible (the Linux installer does this by default).
- The executor never writes secrets to disk beyond the config file, and redacts the bearer token in `show-config` output.
- TLS is provided by `rustls`; OpenSSL is not a dependency. Rotating the system trust store is sufficient to update the executor's trust anchors.
- Capability allowlist (`allowed_capabilities` in config) is enforced *before* execution. The upstream also enforces a per-worker allowlist server-side; both must agree for a job to run.

---

## Related products

`wp-executor` is the open-source piece of a three-product ProjectFlash family:

| Component | Status |
|---|---|
| **wp-executor** (this repo) | Open source. Released today as v1.0.0. |
| **WP-PFWorkflow** | Commercial WordPress plugin. The visual workflow studio shown in the screenshots above. **General availability: May 14, 2026.** |
| **WP-PFAgent** | Commercial WordPress plugin. AI agent surface that drives the workflow studio from natural language. **General availability: May 14, 2026.** |

WP-PFWorkflow and WP-PFAgent are **proprietary, source-available to licensees only**, and are not distributed under the same license as this repository. They will be available for purchase, evaluation download and licensing through the **Project Flash product portal launching at [project-flash.com](https://project-flash.com) on May 7, 2026**.

This executor is fully functional on its own against the public REST contract those plugins publish; you do not need a license to *use* the executor, only to use the plugins on the WordPress side.

---

## Development

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
```

The unit and integration test suites run on every commit on Linux, macOS and Windows via GitHub Actions. The integration tests in `tests/worker_loop.rs` use [`wiremock`](https://crates.io/crates/wiremock) to stand in for the upstream REST API, so they do not require a running WordPress instance.

---

## License

Dual-licensed under either of:

- Apache License, Version 2.0
- MIT license

at your option. SPDX: `MIT OR Apache-2.0`.

The screenshots in `assets/` belong to ProjectFlash and depict the commercial WP-PFWorkflow plugin; they are licensed for redistribution as part of this repository under the same dual license.

---

© 2026 Project Flash Build. The ProjectFlash name and logo are trademarks of Project Flash Build.
