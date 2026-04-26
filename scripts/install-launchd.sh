#!/usr/bin/env bash
# Install secondbrain as a per-user macOS LaunchAgent so it starts on
# login and is restarted if it crashes.
#
# Usage:
#   scripts/install-launchd.sh        # install + load
#   scripts/install-launchd.sh stop   # unload + remove
#   scripts/install-launchd.sh status # show launchctl status

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LABEL="io.mrtns.secondbrain"
SRC_PLIST="${REPO_ROOT}/scripts/${LABEL}.plist"
DEST_PLIST="${HOME}/Library/LaunchAgents/${LABEL}.plist"

case "${1:-install}" in
    install)
        if [[ ! -d "/Applications/secondbrain.app" ]]; then
            echo "error: /Applications/secondbrain.app not found."
            echo "       run: scripts/build-app.sh && cp -R build/secondbrain.app /Applications/"
            exit 1
        fi
        mkdir -p "${HOME}/Library/LaunchAgents"
        cp "${SRC_PLIST}" "${DEST_PLIST}"
        # bootout is a no-op if not loaded
        launchctl bootout "gui/$(id -u)/${LABEL}" 2>/dev/null || true
        launchctl bootstrap "gui/$(id -u)" "${DEST_PLIST}"
        launchctl enable "gui/$(id -u)/${LABEL}"
        echo "installed launch agent at ${DEST_PLIST}"
        echo "logs:  /tmp/secondbrain.{out,err}.log"
        echo "stop:  scripts/install-launchd.sh stop"
        ;;
    stop)
        launchctl bootout "gui/$(id -u)/${LABEL}" 2>/dev/null || true
        rm -f "${DEST_PLIST}"
        echo "unloaded and removed ${DEST_PLIST}"
        ;;
    status)
        launchctl print "gui/$(id -u)/${LABEL}" 2>/dev/null | head -40 || \
            echo "not loaded"
        ;;
    *)
        echo "usage: $0 [install|stop|status]"
        exit 1
        ;;
esac
