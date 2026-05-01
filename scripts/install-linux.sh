#!/usr/bin/env bash
# Install wp-executor as a systemd service.
#
# Usage:
#   sudo ./install-linux.sh                 # System-wide service (root)
#   ./install-linux.sh --user               # Per-user service (no root)
#
# Optional environment overrides:
#   WP_EXECUTOR_BIN=/path/to/wp-executor    # Default: $(which wp-executor) or ./target/release/wp-executor
#   WP_EXECUTOR_CONFIG=/etc/wp-executor/config.toml
#   WP_EXECUTOR_USER=wp-executor            # User to run the service as (system mode)

set -euo pipefail

MODE="system"
if [[ "${1:-}" == "--user" ]]; then
  MODE="user"
fi

# Resolve binary path.
if [[ -z "${WP_EXECUTOR_BIN:-}" ]]; then
  if command -v wp-executor >/dev/null 2>&1; then
    WP_EXECUTOR_BIN="$(command -v wp-executor)"
  elif [[ -x "./target/release/wp-executor" ]]; then
    WP_EXECUTOR_BIN="$(realpath ./target/release/wp-executor)"
  else
    echo "ERROR: wp-executor binary not found. Set WP_EXECUTOR_BIN or place the binary in PATH or ./target/release/." >&2
    exit 1
  fi
fi

if [[ ! -x "${WP_EXECUTOR_BIN}" ]]; then
  echo "ERROR: ${WP_EXECUTOR_BIN} is not executable." >&2
  exit 1
fi

UNIT_NAME="wp-executor.service"

if [[ "${MODE}" == "user" ]]; then
  UNIT_DIR="${XDG_CONFIG_HOME:-${HOME}/.config}/systemd/user"
  CONFIG_PATH="${WP_EXECUTOR_CONFIG:-${XDG_CONFIG_HOME:-${HOME}/.config}/wp-executor/config.toml}"
  SYSTEMCTL=(systemctl --user)
else
  if [[ "$(id -u)" -ne 0 ]]; then
    echo "ERROR: system mode requires root. Re-run with sudo, or pass --user." >&2
    exit 1
  fi
  UNIT_DIR="/etc/systemd/system"
  CONFIG_PATH="${WP_EXECUTOR_CONFIG:-/etc/wp-executor/config.toml}"
  SERVICE_USER="${WP_EXECUTOR_USER:-wp-executor}"

  if ! id "${SERVICE_USER}" >/dev/null 2>&1; then
    echo "Creating system user ${SERVICE_USER}..."
    useradd --system --no-create-home --shell /usr/sbin/nologin "${SERVICE_USER}" || true
  fi
  install -d -m 0750 -o "${SERVICE_USER}" -g "${SERVICE_USER}" "$(dirname "${CONFIG_PATH}")"
  SYSTEMCTL=(systemctl)
fi

mkdir -p "${UNIT_DIR}"
UNIT_PATH="${UNIT_DIR}/${UNIT_NAME}"

# Build the unit file.
SERVICE_USER_LINE=""
if [[ "${MODE}" == "system" ]]; then
  SERVICE_USER_LINE="User=${SERVICE_USER:-wp-executor}"
fi

cat > "${UNIT_PATH}" <<UNIT
[Unit]
Description=ProjectFlash Workflow remote executor
Documentation=https://github.com/Project-Flash-Build/wp-executor
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=${WP_EXECUTOR_BIN} --config ${CONFIG_PATH} run
Restart=on-failure
RestartSec=5s
${SERVICE_USER_LINE}
NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=read-only
PrivateTmp=yes
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=$([ "${MODE}" = "user" ] && echo default.target || echo multi-user.target)
UNIT

if [[ ! -f "${CONFIG_PATH}" ]]; then
  echo "WARN: config not found at ${CONFIG_PATH}. Creating template — fill in base_url and bearer_token before starting the service."
  cat > "${CONFIG_PATH}" <<TEMPLATE
# wp-executor config.toml — fill in the secrets BEFORE starting the service.
# Required:
base_url = "https://CHANGE-ME.example.com"
bearer_token = "pfw_worker_<id>_<secret>"

# Optional (defaults shown):
# namespace = "wp-pfworkflow/v1"
# max_jobs_per_claim = 5
# lease_seconds = 60
# heartbeat_interval_seconds = 15
# idle_poll_seconds = 5
# default_job_timeout_seconds = 300
# sign_requests = true
# allowed_capabilities = ["shell.run", "fs.read", "fs.write", "fs.list", "http.request", "system.info"]
TEMPLATE
  chmod 0600 "${CONFIG_PATH}"
  if [[ "${MODE}" == "system" ]]; then
    chown "${SERVICE_USER}:${SERVICE_USER}" "${CONFIG_PATH}"
  fi
fi

"${SYSTEMCTL[@]}" daemon-reload
"${SYSTEMCTL[@]}" enable "${UNIT_NAME}"
"${SYSTEMCTL[@]}" restart "${UNIT_NAME}"

echo
echo "Installed ${UNIT_NAME} (${MODE} mode)."
echo "  binary : ${WP_EXECUTOR_BIN}"
echo "  config : ${CONFIG_PATH}"
echo "  unit   : ${UNIT_PATH}"
echo
echo "Useful commands:"
echo "  ${SYSTEMCTL[*]} status ${UNIT_NAME}"
echo "  ${SYSTEMCTL[*]} restart ${UNIT_NAME}"
if [[ "${MODE}" == "user" ]]; then
  echo "  journalctl --user -u ${UNIT_NAME} -f"
else
  echo "  journalctl -u ${UNIT_NAME} -f"
fi
