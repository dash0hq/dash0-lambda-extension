#!/usr/bin/env bash
#
# Align Lambda layer version counters in a newly launched AWS region.
#
# Usage: backfill-layer-versions.sh [--dry-run] <source-region> <target-region> [layer-name ...]
#
# Lambda layer version numbers auto-increment per region and can never be
# chosen. When we start deploying to a new region (e.g. il-central-1), its
# layers would begin at version 1 while every existing region is at some
# aligned version N — breaking tooling that assumes the same layer name +
# version resolves in every region (e.g. the sls-plugin ARN builder).
#
# The version counter never resets, even when versions are deleted. So for
# each layer this script reads the highest version N from the source region,
# then publishes dummy versions (a minimal placeholder zip) in the target
# region — leaving each dummy in place — until the counter reaches N.
# The next real deploy then produces version N+1 in the new region and in
# all existing regions alike.
#
# Notes:
#   - Dummy versions are left in place (deletion requires extra permissions);
#     they occupy version numbers but are never made public. Delete them
#     manually later if desired. Any real versions already present keep their zips.
#   - If the target counter is already ahead of the source (possible when
#     versions were published *and deleted* there before — the counter is
#     not observable via the API), alignment is impossible and the layer is
#     reported as failed.
#
# Prerequisites: AWS CLI v2, jq, credentials for the account owning the
# layers (same account for both regions), target region opted in.
set -euo pipefail

# Lean on the CLI's built-in throttle handling instead of a retry loop.
export AWS_RETRY_MODE=adaptive
export AWS_MAX_ATTEMPTS=10

usage="usage: $0 [--dry-run] <source-region> <target-region> [layer-name ...]"

dry_run=false
if [ "${1:-}" = "--dry-run" ]; then
  dry_run=true
  shift
fi

source_region="${1:?${usage}}"; shift
target_region="${1:?${usage}}"; shift

if [ "${source_region}" = "${target_region}" ]; then
  echo "error: source and target region are both '${source_region}'" >&2
  exit 1
fi

if [ "$#" -gt 0 ]; then
  layers=("$@")
else
  layers=(dash0-extension-python dash0-extension-node dash0-extension-java dash0-extension-manual)
fi

workdir="$(mktemp -d)"
trap 'rm -rf "${workdir}"' EXIT

dummy_zip="${workdir}/dummy-layer.zip"
echo "Placeholder to align Lambda layer version numbering across regions. Safe to delete." \
  > "${workdir}/README-dummy.txt"
(cd "${workdir}" && zip -q "${dummy_zip}" README-dummy.txt)

dummy_description="Placeholder to align layer version numbering across regions - safe to delete"

# Highest version of a layer in a region ("0" if the layer does not exist).
# list-layer-versions returns versions newest-first.
max_version() {
  local layer="$1" region="$2" out
  if ! out="$(aws lambda list-layer-versions --layer-name "${layer}" --region "${region}" \
      --max-items 1 --query 'LayerVersions[0].Version' --output text --no-cli-pager 2>&1)"; then
    if grep -q ResourceNotFoundException <<< "${out}"; then
      echo 0
      return 0
    fi
    echo "${out}" >&2
    return 1
  fi
  # Extract first non-empty line and replace None with 0
  echo "${out}" | grep -v '^None$' | grep . | head -1 | sed 's/^None$/0/'
}

# All existing versions of a layer in a region, oldest-first, space-separated
# (empty if the layer does not exist).
list_versions() {
  local layer="$1" region="$2" out
  if ! out="$(aws lambda list-layer-versions --layer-name "${layer}" --region "${region}" \
      --query 'LayerVersions[].Version' --output json --no-cli-pager 2>&1)"; then
    if grep -q ResourceNotFoundException <<< "${out}"; then
      return 0
    fi
    echo "${out}" >&2
    return 1
  fi
  jq -r 'sort | map(tostring) | join(" ")' <<< "${out}"
}

failed_layers=()
summary=()

for layer in "${layers[@]}"; do
  echo "=== ${layer} ==="

  source_max="$(max_version "${layer}" "${source_region}")"
  if [ "${source_max}" -eq 0 ]; then
    echo "error: layer '${layer}' does not exist in source region ${source_region}; skipping" >&2
    failed_layers+=("${layer}")
    summary+=("${layer}: FAILED (not found in ${source_region})")
    continue
  fi
  echo "source ${source_region}: highest version ${source_max}"

  target_versions="$(list_versions "${layer}" "${target_region}")"
  if [ -n "${target_versions}" ]; then
    echo "target ${target_region}: existing versions: ${target_versions}"
    target_listed_max="${target_versions##* }"
  else
    echo "target ${target_region}: layer does not exist yet"
    target_listed_max=0
  fi

  if [ "${target_listed_max}" -gt "${source_max}" ]; then
    echo "error: target is already at version ${target_listed_max}, ahead of source (${source_max}); cannot align" >&2
    failed_layers+=("${layer}")
    summary+=("${layer}: FAILED (target ahead: ${target_listed_max} > ${source_max})")
    continue
  fi
  if [ "${target_listed_max}" -eq "${source_max}" ]; then
    echo "already aligned at version ${source_max}; nothing to do"
    summary+=("${layer}: already aligned at ${source_max}")
    continue
  fi

  to_publish=$(( source_max - target_listed_max ))
  if "${dry_run}"; then
    echo "dry-run: would publish ${to_publish} dummy version(s) ($((target_listed_max + 1))..${source_max} expected)"
    summary+=("${layer}: dry-run, would publish ${to_publish} dummy version(s) to reach ${source_max}")
    continue
  fi

  published=0
  layer_failed=false
  while true; do
    v="$(aws lambda publish-layer-version --layer-name "${layer}" --region "${target_region}" \
      --zip-file "fileb://${dummy_zip}" \
      --description "${dummy_description}" \
      --query Version --output text --no-cli-pager)"
    if [ "${v}" -gt "${source_max}" ]; then
      # The target's hidden counter was already at/above source_max (versions
      # published and deleted there before). The counter only moves forward,
      # so alignment is impossible.
      echo "error: dummy publish produced version ${v} > source max ${source_max}; target counter is ahead, cannot align" >&2
      layer_failed=true
      break
    fi
    published=$(( published + 1 ))
    echo "[${layer}] published dummy version ${v}/${source_max} (left in place; delete manually if desired)"
    if [ "${v}" -eq "${source_max}" ]; then
      break
    fi
  done

  if "${layer_failed}"; then
    failed_layers+=("${layer}")
    summary+=("${layer}: FAILED (target counter ahead of ${source_max})")
  else
    summary+=("${layer}: aligned at ${source_max} (${published} dummy version(s) published, left in place)")
  fi
done

echo
echo "=== summary (${source_region} -> ${target_region}) ==="
for line in "${summary[@]}"; do
  echo "${line}"
done

if [ "${#failed_layers[@]}" -gt 0 ]; then
  exit 1
fi
