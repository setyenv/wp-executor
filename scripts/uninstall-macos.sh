#!/usr/bin/env bash
# Uninstall the wp-executor launchd service installed by install-macos.sh.

set -euo pipefail

MODE="user"
if [[ "${1:-}" == "--system" ]]; then
  MODE="system"
fi

LABEL="com.setyenv.wp-executor"

if [[ "${MODE}" == "user" ]]; then
  PLIST_DIR="${HOME}/Library/LaunchAgents"
  LAUNCHCTL_DOMAIN="gui/$(id -u)"
else
  if [[ "$(id -u)" -ne 0 ]]; then
    echo "ERROR: --system requires root. Re-run with sudo." >&2
    exit 1
  fi
  PLIST_DIR="/Library/LaunchDaemons"
  LAUNCHCTL_DOMAIN="system"
fi

PLIST_PATH="${PLIST_DIR}/${LABEL}.plist"

launchctl bootout "${LAUNCHCTL_DOMAIN}" "${PLIST_PATH}" 2>/dev/null || \
  launchctl unload "${PLIST_PATH}" 2>/dev/null || true
rm -f "${PLIST_PATH}"

echo "Removed ${LABEL} (${MODE} mode)."
echo "Note: binary, config and logs were left in place."
