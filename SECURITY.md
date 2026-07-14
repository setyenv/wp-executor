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
- **Two-layer allowlist — what it may do, and where it may reach.**
  - *Capabilities:* the upstream issues a per-worker capability set, and the
    worker enforces its own optional local `allowed_capabilities` allowlist
    (jobs for a capability outside it are rejected with
    `pfw_capability_not_allowed`). Set it to the minimum your workflows need.
  - *Egress (SSRF guard, on by default):* the `http.request` capability rejects
    connections to non-global destinations — RFC-1918 private ranges, loopback,
    link-local (including the cloud metadata endpoint `169.254.169.254`), IPv6
    unique-local / link-local, and IPv4-mapped forms of those. The host is
    resolved and *every* resulting IP is checked (not just the URL string), the
    connection is pinned to the validated address so DNS rebinding can't slip
    through, and redirects are not auto-followed. Relax it for specific trusted
    hosts via `allowed_egress_hosts` (patterns like `*.example.com` are
    supported; a single `"*"` disables the guard). Keep the allowlist narrow.
- **Low-privilege by default.** The Linux installer creates a dedicated system
  user (`useradd --system --no-create-home --shell /usr/sbin/nologin`) and runs
  under systemd hardening: `User=wp-executor`, `NoNewPrivileges=yes`,
  `ProtectSystem=strict`, `ProtectHome=read-only`, `PrivateTmp=yes`. Do not run
  it as root.
- **Per-job timeouts.** Each job has a hard execution timeout (default 300s,
  overridable per job) so a single task cannot run unbounded.

## Operator responsibilities

The worker executes what your trusted site dispatches, as the OS user it runs
as. Its blast radius is bounded by that user's privileges, the two-layer
allowlist above, and the OS sandbox:

- **Tune the egress allowlist.** The SSRF guard blocks internal destinations by
  default. If a workflow legitimately needs a specific internal host, add just
  that host to `allowed_egress_hosts` — do not disable the guard with `"*"`
  unless the worker runs on an already-isolated network. For defence in depth
  you can also restrict outbound reach at the firewall/network layer.
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
