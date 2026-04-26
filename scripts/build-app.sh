#!/usr/bin/env bash
# Build secondbrain.app — a macOS menubar-only .app bundle wrapping the
# secondbrain-menubar binary.
#
# Usage:
#   scripts/build-app.sh                  # release build, drops into ./build/
#   scripts/build-app.sh /Applications    # build then install into target dir

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TARGET_DIR="${1:-${REPO_ROOT}/build}"
APP_NAME="secondbrain.app"
APP_PATH="${TARGET_DIR}/${APP_NAME}"

cd "${REPO_ROOT}"
echo ">> building secondbrain-menubar (release)"
cargo build --release -p secondbrain-menubar --bin secondbrain-menubar

mkdir -p "${TARGET_DIR}"
rm -rf "${APP_PATH}"
mkdir -p "${APP_PATH}/Contents/MacOS"
mkdir -p "${APP_PATH}/Contents/Resources"

cp "${REPO_ROOT}/target/release/secondbrain-menubar" "${APP_PATH}/Contents/MacOS/"
cp "${REPO_ROOT}/scripts/Info.plist" "${APP_PATH}/Contents/Info.plist"

# Ad-hoc sign so macOS will run it without quarantine prompts on the
# same machine. Real distribution would use a Developer ID cert.
codesign --force --deep --sign - "${APP_PATH}" >/dev/null 2>&1 || true

echo ">> built ${APP_PATH}"
echo "   first run will require granting Screen Recording permission in"
echo "   System Settings → Privacy & Security → Screen Recording"
echo "   then quit and relaunch."
