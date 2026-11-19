#!/usr/bin/env bash
#
# Resolve the layer version a release publishes, and print it.
#
# Usage: resolve-layer-version.sh <version | auto> <region> [region ...]
#
# Reads the latest version of every dash0-extension layer in every region:
#   - auto:               the highest version found, plus one.
#   - a version above it: that version; publishing pads the counters up to it
#                         (see tools/publish-layer.sh).
#   - the highest itself: re-releases it, with a warning; the publish skips the
#                         layers already at it.
#   - a version below it: fails; some layer can no longer get it.
#
# Logs go to stderr, so the version is the only thing on stdout.
set -euo pipefail

usage="usage: $0 <version | auto> <region> [region ...]"
requested="${1:?${usage}}"; shift
[ "$#" -gt 0 ] || { echo "${usage}" >&2; exit 1; }

if [ "${requested}" != auto ] && ! [[ "${requested}" =~ ^[1-9][0-9]*$ ]]; then
  echo "error: version must be a positive number or 'auto', got '${requested}'" >&2
  exit 1
fi

highest=0
for region in "$@"; do
  for layer in python node java manual; do
    version="$(aws lambda list-layer-versions --region "${region}" \
      --layer-name "dash0-extension-${layer}" --max-items 1 \
      --query 'LayerVersions[0].Version' --output text --no-cli-pager | head -1)"
    echo "${region} dash0-extension-${layer}: ${version}" >&2
    # None: the layer has no versions in this region yet.
    if [ "${version}" != None ] && [ "${version}" -gt "${highest}" ]; then
      highest="${version}"
    fi
  done
done

if [ "${requested}" = auto ]; then
  version="$((highest + 1))"
elif [ "${requested}" -lt "${highest}" ]; then
  echo "error: cannot release version ${requested}: some layer is already at version ${highest}" >&2
  exit 1
else
  version="${requested}"
fi

if [ "${version}" -eq "${highest}" ]; then
  echo "::warning::Re-releasing version ${version}: layers already at it are skipped" >&2
fi
echo "Releasing version ${version}" >&2
echo "${version}"
