#!/usr/bin/env bash
# Install wp-executor as a launchd service.
#
# Usage:
#   ./install-macos.sh                   # Per-user (LaunchAgent under ~/Library/LaunchAgents)
#   sudo ./install-macos.sh --system     # System-wide (LaunchDaemon under /Library/LaunchDaemons)
#
# Optional environment overrides:
#   WP_EXECUTOR_BIN=/path/to/wp-executor
#   WP_EXECUTOR_CONFIG=/path/to/config.toml

set -euo pipefail

MODE="user"
if [[ "${1:-}" == "--system" ]]; then
  MODE="system"
fi

LABEL="com.setyenv.wp-executor"

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

if [[ "${MODE}" == "user" ]]; then
  PLIST_DIR="${HOME}/Library/LaunchAgents"
  LOG_DIR="${HOME}/Library/Logs/wp-executor"
  CONFIG_PATH="${WP_EXECUTOR_CONFIG:-${HOME}/Library/Application Support/wp-executor/config.toml}"
  LAUNCHCTL_DOMAIN="gui/$(id -u)"
else
  if [[ "$(id -u)" -ne 0 ]]; then
    echo "ERROR: --system requires root. Re-run with sudo." >&2
    exit 1
  fi
  PLIST_DIR="/Library/LaunchDaemons"
  LOG_DIR="/var/log/wp-executor"
  CONFIG_PATH="${WP_EXECUTOR_CONFIG:-/etc/wp-executor/config.toml}"
  LAUNCHCTL_DOMAIN="system"
fi

mkdir -p "${PLIST_DIR}" "${LOG_DIR}" "$(dirname "${CONFIG_PATH}")"

PLIST_PATH="${PLIST_DIR}/${LABEL}.plist"

cat > "${PLIST_PATH}" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>${LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>${WP_EXECUTOR_BIN}</string>
        <string>--config</string>
        <string>${CONFIG_PATH}</string>
        <string>run</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>${LOG_DIR}/stdout.log</string>
    <key>StandardErrorPath</key>
    <string>${LOG_DIR}/stderr.log</string>
    <key>EnvironmentVariables</key>
    <dict>
        <key>RUST_LOG</key>
        <string>wp_executor=info</string>
    </dict>
</dict>
</plist>
PLIST

if [[ ! -f "${CONFIG_PATH}" ]]; then
  echo "WARN: config not found at ${CONFIG_PATH}. Creating template — fill in base_url and bearer_token before starting the service."
  cat > "${CONFIG_PATH}" <<TEMPLATE
# wp-executor config.toml — fill in the secrets BEFORE starting the service.
base_url = "https://CHANGE-ME.example.com"
bearer_token = "pfw_worker_<id>_<secret>"
TEMPLATE
  chmod 0600 "${CONFIG_PATH}"
fi

# Reload (modern bootstrap/bootout pattern; falls back to load/unload on older macOS).
if launchctl bootout "${LAUNCHCTL_DOMAIN}" "${PLIST_PATH}" >/dev/null 2>&1; then
  :
fi
launchctl bootstrap "${LAUNCHCTL_DOMAIN}" "${PLIST_PATH}" || launchctl load "${PLIST_PATH}"
launchctl enable "${LAUNCHCTL_DOMAIN}/${LABEL}" 2>/dev/null || true
launchctl kickstart -k "${LAUNCHCTL_DOMAIN}/${LABEL}" 2>/dev/null || true

echo
echo "Installed ${LABEL} (${MODE} mode)."
echo "  binary : ${WP_EXECUTOR_BIN}"
echo "  config : ${CONFIG_PATH}"
echo "  plist  : ${PLIST_PATH}"
echo "  logs   : ${LOG_DIR}/{stdout,stderr}.log"
echo
echo "Useful commands:"
echo "  launchctl print ${LAUNCHCTL_DOMAIN}/${LABEL}"
echo "  launchctl kickstart -k ${LAUNCHCTL_DOMAIN}/${LABEL}"
echo "  tail -f ${LOG_DIR}/stdout.log"
