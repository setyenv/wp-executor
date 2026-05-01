#!/usr/bin/env bash
# Uninstall the wp-executor systemd service installed by install-linux.sh.
#
# Usage:
#   sudo ./uninstall-linux.sh
#   ./uninstall-linux.sh --user
#
# Does NOT remove the binary, the config file, or the system user. Just
# stops/disables the unit and deletes the unit file.

set -euo pipefail

MODE="system"
if [[ "${1:-}" == "--user" ]]; then
  MODE="user"
fi

UNIT_NAME="wp-executor.service"

if [[ "${MODE}" == "user" ]]; then
  UNIT_DIR="${XDG_CONFIG_HOME:-${HOME}/.config}/systemd/user"
  SYSTEMCTL=(systemctl --user)
else
  if [[ "$(id -u)" -ne 0 ]]; then
    echo "ERROR: system mode requires root. Re-run with sudo, or pass --user." >&2
    exit 1
  fi
  UNIT_DIR="/etc/systemd/system"
  SYSTEMCTL=(systemctl)
fi

UNIT_PATH="${UNIT_DIR}/${UNIT_NAME}"

"${SYSTEMCTL[@]}" disable --now "${UNIT_NAME}" 2>/dev/null || true
rm -f "${UNIT_PATH}"
"${SYSTEMCTL[@]}" daemon-reload

echo "Removed ${UNIT_NAME} (${MODE} mode)."
echo "Note: binary and config file (if any) were left in place."
