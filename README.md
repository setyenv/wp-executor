# WP-Executor

> **wp-executor — run on your own machine.** The open-source runner for the Setyenv™ suite: a single-binary worker, written in Rust, that takes your workflow events and runs them on *your own machine, in your own tools* — system commands, file IO, outbound HTTP — under a capability allowlist you define.

[![license](https://img.shields.io/badge/license-MIT%20%2F%20Apache--2.0-blue.svg)](#license)
[![rust](https://img.shields.io/badge/rust-1.80%2B-orange.svg)](#build-from-source)
[![platforms](https://img.shields.io/badge/platforms-linux%20%7C%20macos%20%7C%20windows-lightgrey.svg)](#install)
[![Setyenv](https://img.shields.io/badge/suite-setyenv.com-6c5ce7.svg)](https://setyenv.com)

`wp-executor` is the open-source companion of the Setyenv workflow engine. The engine models the *intent* of an automation; this binary executes that intent's host-side actions — shell commands, file system operations, outbound network calls — on hardware you operate, under a capability allowlist you define.

This repository contains the executor binary and install scripts for Linux, macOS and Windows.

---

## Where this fits

<p align="center">
  <img src="assets/cross-plugin-architecture.svg" alt="Setyenv architecture: WP-PFAgent (LLM-driven console) constructs WP-PFManagement entities and WP-PFWorkflow workflows inside your WordPress install; WP-PFWorkflow publishes an HMAC-signed job queue that wp-executor polls from an external machine." width="900" />
</p>

The platform is where customers compose automations. When a step requires an action that should not run inside the WordPress request lifecycle — repository synchronisation, media transcoding, scheduled backups, or any operation that benefits from a separate execution boundary — the platform records the intent. `wp-executor` polls for that intent on its own cadence, evaluates it against a local capability allowlist, executes it, and returns a structured result. Authentication is bearer-token, optionally augmented with HMAC body signing.

The wire protocol and queue semantics are not enumerated here: the platform publishes its versioned contract over a public REST surface, and the executor consumes it on startup as the source of truth.

A worked end-to-end example — a WooCommerce order becoming a ticket, a workflow, an AI triage and an executor-generated RMA file — is documented at [setyenv.com/use-case](https://setyenv.com/use-case).

### What it shows

| <img src="assets/wp-pfworkflow-dashboard.png" alt="WP-PFWorkflow workflow library" /> | <img src="assets/wp-pfworkflow-editor.png" alt="WP-PFWorkflow workflow editor" /> |
|:---:|:---:|
| **The workflow library.** A unified inventory of every automation defined for a site, with execution state and structural metrics surfaced at a glance. | **The visual workflow editor.** Triggers, conditional branches, function invocations and error boundaries are first-class graph elements. The canvas is the production surface, not a sketch. |

The screenshots are taken from WP-PFWorkflow, the upstream commercial product — see [Related products](#related-products). They are included here so the role of the executor is unambiguous: the platform composes and dispatches a workflow; the executor performs the host-side work that workflow requires.

---

## Capabilities

The executor implements exactly the six capabilities the platform's contract defines, no more, no less. Each one carries a typed payload and returns a typed result.

| Key | What it does | Typical use |
|---|---|---|
| `shell.run` | Execute a shell command (cmd / powershell / pwsh / bash / sh / auto) with hard timeout, optional stdin, environment overrides, working directory | Run scripts, scheduled maintenance, build pipelines, anything you'd put in `cron` |
| `fs.read` | Read a file as utf-8 or base64, with a max-bytes guard and best-effort MIME detection | Hand a config file, a log fragment or a generated artifact back to the workflow |
| `fs.write` | Write a file (overwrite / append / create-only), auto-create parent directories, base64 input ok | Drop a generated report, save a downloaded asset, stage a file before another step picks it up |
| `fs.list` | Directory listing, optional recursive, hidden filter, max-entries cap | Inventory a release directory, drive a per-file loop in the workflow |
| `http.request` | Arbitrary HTTP call (any method, any headers, JSON or string body) with timeout. Status code is surfaced as the exit code | Hit a LAN-only API, call a self-hosted service, fetch from a private host the WordPress server cannot reach |
| `system.info` | OS, arch, hostname, CPU count, memory, executor version, uptime, capabilities advertised | Health checks, populating workflow metadata, fleet inventory |

Every capability accepts an executor-side timeout, refuses to perform operations outside the configured allowlist, and returns a uniform `{ exit_code, stdout, stderr, output, duration_ms, error }` payload.

---

## Install

### Pre-built binaries

Each tagged release publishes binaries for Linux (x86_64), macOS (x86_64 + Apple Silicon) and Windows (x86_64). Download the archive that matches your platform from the [releases page](https://github.com/setyenv/wp-executor/releases), unzip, and place the `wp-executor` binary somewhere on your `PATH`.

### Build from source

```bash
cargo build --release
# binary at target/release/wp-executor
```

Requires Rust 1.80 or newer. No system dependencies (TLS uses `rustls`).

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

The bearer token is provisioned for each worker through the WP-PFWorkflow administration surface; it is shown in plain text exactly once and can be rotated at any time. The executor never writes the secret to disk beyond the config file and never emits it in log output.

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
- Capability allowlist (`allowed_capabilities` in config) is enforced *before* execution. The platform also enforces a per-worker allowlist server-side; both must agree for a job to run.

---

## Related products

`wp-executor` is the open-source surface of the Setyenv™ suite from Setyenv Build. Two of the plugins are open source and free; two are proprietary and licensed:

| Component | Status |
|---|---|
| <img src="assets/logo-executor.png" alt="WP-Executor logo" width="64" /><br/>**WP-Executor** (this repository) | **Open source**, MIT OR Apache-2.0. Free; no licence is required to run it. |
| <img src="assets/logo-pfa.png" alt="WP-PFAgent logo" width="64" /><br/>**WP-PFAgent™** | The AI agent layer that turns natural language into workflows and entities. **Open source (GPL-2.0-or-later), free** — [github.com/setyenv/wp-pfagent](https://github.com/setyenv/wp-pfagent). Needs a licensed WP-PFWorkflow or WP-PFManagement on the WordPress side to do useful work. |
| <img src="assets/logo-pfw.png" alt="WP-PFWorkflow logo" width="64" /><br/>**WP-PFWorkflow™** | The visual workflow engine shown in the screenshots above. **Proprietary**, per-customer-licensed WordPress plugin (monthly or annual, per domain). |
| <img src="assets/logo-pfm.png" alt="WP-PFManagement logo" width="64" /><br/>**WP-PFManagement™** | The **low-code platform** — model entities, fields, forms, lists, permissions and business rules to build real apps (ITSM, CRM, asset/CMDB, service desk) inside WordPress, no external SaaS. **Proprietary**, per-customer-licensed WordPress plugin. |

**WP-PFWorkflow** and **WP-PFManagement** are the proprietary, per-customer-licensed plugins: the default build ships obfuscated and is refundable, with an optional annual add-on that delivers the clean PHP source. **WP-PFAgent** and **WP-Executor** are open source and free. All are available for evaluation, purchase and licensing through the Setyenv product portal at [setyenv.com](https://setyenv.com).

The executor and WP-PFAgent are fully open source; no commercial licence is required to operate them — only the WordPress side, WP-PFWorkflow or WP-PFManagement, requires a licence.

---

## Development

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
```

The unit and integration suites cover Linux, macOS and Windows. Integration tests in `tests/worker_loop.rs` use [`wiremock`](https://crates.io/crates/wiremock) to stand in for the upstream REST surface, so they do not require a running WordPress instance.

---

## License

Dual-licensed under either of:

- Apache License, Version 2.0
- MIT license

at your option. SPDX: `MIT OR Apache-2.0`.

The screenshots under `assets/` depict the commercial WP-PFWorkflow™ product and are redistributed within this repository under the same dual licence as the source.

---

Setyenv™, WP-PFWorkflow™, WP-PFManagement™ and WP-PFAgent™ are trademarks of Setyenv™.
