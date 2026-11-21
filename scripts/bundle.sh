#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

if [[ -z "${CARGO_BUILD_ARGS+x}" ]]; then
  cargo build --release --offline
elif [[ -n "${CARGO_BUILD_ARGS}" ]]; then
  # shellcheck disable=SC2086
  cargo build --release ${CARGO_BUILD_ARGS}
else
  cargo build --release
fi

APP_DIR="target/Sampler.app"
rm -rf "${APP_DIR}"
mkdir -p "${APP_DIR}/Contents/MacOS"

cp target/release/sampler "${APP_DIR}/Contents/MacOS/sampler"
chmod +x "${APP_DIR}/Contents/MacOS/sampler"
cp assets/Info.plist "${APP_DIR}/Contents/Info.plist"

if command -v codesign >/dev/null 2>&1; then
  codesign --force --deep --sign - "${APP_DIR}" || echo "Warning: ad-hoc codesign skipped"
else
  echo "Warning: ad-hoc codesign skipped"
fi

echo "Bundled target/Sampler.app"
