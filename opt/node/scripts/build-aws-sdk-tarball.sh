#!/usr/bin/env bash
set -euo pipefail

REPO="https://github.com/mosheshaham-dash0/opentelemetry-js-contrib.git"
BRANCH="instrument-kinesis-inject-context-v3"

# Resolve the target path relative to this script (opt/node/scripts -> ../../build)
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../../.." && pwd)"
TGZ_PATH="$ROOT_DIR/build/opentelemetry-instrumentation-aws-sdk.tgz"

if [ -f "$TGZ_PATH" ]; then
  echo "@opentelemetry/instrumentation-aws-sdk tarball already exists at $TGZ_PATH, skipping build"
  exit 0
fi

echo "Building @opentelemetry/instrumentation-aws-sdk from fork ($BRANCH)"
mkdir -p "$ROOT_DIR/build"
CLONE_DIR="$ROOT_DIR/build/opentelemetry-js-contrib"
rm -rf "$CLONE_DIR"

git clone --depth 1 --branch "$BRANCH" "$REPO" "$CLONE_DIR"
cd "$CLONE_DIR"
npm install
npm run compile -w packages/contrib-test-utils
npm run version:update -w packages/instrumentation-aws-sdk
npm run compile -w packages/instrumentation-aws-sdk
npm pack -w packages/instrumentation-aws-sdk

cp opentelemetry-instrumentation-aws-sdk-*.tgz "$TGZ_PATH"
rm -rf "$CLONE_DIR"

echo "Tarball created at $TGZ_PATH"
