#!/usr/bin/env bash
#
# Check that a published extension image can be pulled without credentials for
# both Lambda architectures, and that it was built from the expected commit.
#
# Usage: verify-public-image.sh <image-repository> <expected-git-sha> <tag> [tag...]
set -euo pipefail

repository="${1:?image repository required}"; shift
expected_sha="${1:?expected git sha required}"; shift
if [ "$#" -eq 0 ]; then
  echo "at least one tag required" >&2
  exit 2
fi

# An empty Docker config holds no registry logins, so every pull is anonymous,
# the way a customer's build pulls.
DOCKER_CONFIG="$(mktemp -d)"
export DOCKER_CONFIG
trap 'rm -rf "$DOCKER_CONFIG"' EXIT

failed=0
for tag in "$@"; do
  image="${repository}:${tag}"
  for platform in linux/amd64 linux/arm64; do
    if ! container="$(docker create --pull always --platform "$platform" "$image" unused)"; then
      echo "::error::${image} cannot be pulled anonymously for ${platform}"
      failed=1
      continue
    fi
    actual_sha="$(docker cp "${container}:/opt/dash0_git_hash" - | tar -xO || true)"
    docker rm "$container" >/dev/null
    if [ -z "$actual_sha" ]; then
      echo "::error::${image} (${platform}) has no /opt/dash0_git_hash"
      failed=1
      continue
    fi
    if [ "$actual_sha" != "$expected_sha" ]; then
      echo "::error::${image} (${platform}) was built from ${actual_sha}, expected ${expected_sha}"
      failed=1
      continue
    fi
    echo "${image} (${platform}) is built from ${actual_sha}"
  done
done

exit "$failed"
