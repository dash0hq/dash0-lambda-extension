#!/usr/bin/env bash
set -euo pipefail

# Builds @opentelemetry/instrumentation-aws-sdk from an upstream release tag with
# Dash0's patches applied (Kinesis PutRecord/PutRecords trace context injection,
# originally from mosheshaham-dash0/opentelemetry-js-contrib@instrument-kinesis-inject-context-v3).
REPO="https://github.com/open-telemetry/opentelemetry-js-contrib.git"
TAG="instrumentation-aws-sdk-v0.78.0"

# Resolve the target path relative to this script (opt/node/scripts -> ../../build)
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../../.." && pwd)"
TGZ_PATH="$ROOT_DIR/build/opentelemetry-instrumentation-aws-sdk.tgz"

if [ -f "$TGZ_PATH" ]; then
  echo "@opentelemetry/instrumentation-aws-sdk tarball already exists at $TGZ_PATH, skipping build"
  exit 0
fi

echo "Building @opentelemetry/instrumentation-aws-sdk from $TAG with Dash0 patches"
mkdir -p "$ROOT_DIR/build"
CLONE_DIR="$ROOT_DIR/build/opentelemetry-js-contrib"
rm -rf "$CLONE_DIR"

git clone --depth 1 --branch "$TAG" "$REPO" "$CLONE_DIR"
cd "$CLONE_DIR"
git apply "$SCRIPT_DIR"/patches/instrumentation-aws-sdk-*.patch
npm install
npm run compile -w packages/contrib-test-utils
npm run version:update -w packages/instrumentation-aws-sdk
npm run compile -w packages/instrumentation-aws-sdk
npm pack -w packages/instrumentation-aws-sdk

cp opentelemetry-instrumentation-aws-sdk-*.tgz "$TGZ_PATH"
rm -rf "$CLONE_DIR"

echo "Tarball created at $TGZ_PATH"
