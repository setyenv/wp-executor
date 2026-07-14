# Security Policy

## Reporting a vulnerability

Email **build@setyenv.com** with a description and, if possible, a proof of
concept. Please report privately first and give us reasonable time to ship a fix
before any public disclosure — do not open a public issue for a security report.
We aim to acknowledge reports within a few business days.

## What wp-executor is

`wp-executor` is a self-hosted worker that runs **host-side actions** for a
WP-PFWorkflow site: it runs shell commands, reads and writes files, makes HTTP
requests, and reports system info — whatever a workflow step asks of it. It is
powerful by design, so the security model is about **bounding that power**.

## Design

- **Pull-only — no inbound surface.** The worker *polls* (claims jobs from) a
  single configured site over outbound HTTPS. It never opens a listening port,
  so there is no inbound service to expose, firewall, or attack.
- **One authenticated upstream.** It talks to exactly one site (`base_url`) using
  a per-worker bearer token (`pfw_worker_<id>_<secret>`). Every request is
  additionally signed with **HMAC-SHA256** (`X-PFW-Signature`, on by default)
  using the shared worker secret, so the site authenticates the worker. Jobs are
  only ever received in the response to an authenticated claim to that one site.
- **TLS via rustls.** HTTPS uses [rustls](https://github.com/rustls/rustls)
  (`reqwest` built with `default-features = false, features = ["rustls-tls"]`) —
  there is no OpenSSL / native-TLS in the dependency tree. Use an `https://`
  `base_url` in production.
- **Capability allowlist.** Two layers gate which actions may run: the upstream
  issues a per-worker capability set, and the worker enforces its own optional
  local `allowed_capabilities` allowlist (jobs for a capability outside it are
  rejected with `pfw_capability_not_allowed`). Set it to the minimum your
  workflows need.
- **Low-privilege by default.** The Linux installer creates a dedicated system
  user (`useradd --system --no-create-home --shell /usr/sbin/nologin`) and runs
  under systemd hardening: `User=wp-executor`, `NoNewPrivileges=yes`,
  `ProtectSystem=strict`, `ProtectHome=read-only`, `PrivateTmp=yes`. Do not run
  it as root.
- **Per-job timeouts.** Each job has a hard execution timeout (default 300s,
  overridable per job) so a single task cannot run unbounded.

## Operator responsibilities

The worker executes what your trusted site dispatches, as the OS user it runs
as. Its blast radius is bounded by that user's privileges, the capability
allowlist, and the OS sandbox — **not** by application-level content filtering:

- **No built-in network egress allowlist.** The `http_request` capability will
  connect to whatever URL a job specifies; the worker does not restrict
  destination hosts. If you need to limit outbound reach — for example to block
  SSRF-style access to internal services — enforce it at the **firewall/network
  layer**, and/or disable `http_request` via `allowed_capabilities`.
- **Filesystem and shell reach follow the OS user.** `fs.write` and `shell.run`
  run with the service user's permissions. Keep that user unprivileged and rely
  on the systemd hardening above (or an equivalent container/sandbox) to
  constrain which paths it can touch.
- **Protect the config.** `config.toml` holds the bearer token; keep it readable
  only by the service user.

## Supported versions

Security fixes are released against the latest tagged version. Please upgrade to
the [latest release](https://github.com/setyenv/wp-executor/releases) before
reporting.
