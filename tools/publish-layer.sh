#!/usr/bin/env bash
#
# Publish a Lambda layer version and make it public.
#
# Usage: publish-layer.sh <distro> <layer-name> <zip> <description> [<version> | auto]
#
# With `auto` (the default), publishes whatever version AWS assigns next.
#
# With a version N, the publish must produce exactly N, so a release has the same
# layer version in every region. Lambda assigns layer versions from a per-region
# counter that can't be set, so depending on the latest version in the region:
#   - below N:  publishes placeholder versions, never made public, until the
#               counter is at N-1, then publishes the layer. Fails, leaving the
#               layer private, if AWS assigns anything but N: the counter is
#               ahead of the listed versions when versions were deleted.
#   - N:        an earlier attempt of the same release already published it.
#               Skips the publish with a warning and only makes sure N is
#               public, so re-releasing a version is idempotent.
#   - above N:  fails; N can no longer be published here.
#
# <distro> is node, python, java or manual, as in tools/layer-runtimes.py.
set -euo pipefail

usage="usage: $0 <distro> <layer-name> <zip> <description> [<version> | auto]"
distro="${1:?${usage}}"
layer="${2:?${usage}}"
zip="${3:?${usage}}"
description="${4:?${usage}}"
expected="${5:-auto}"

tools_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if [ "${expected}" != auto ] && ! [[ "${expected}" =~ ^[1-9][0-9]*$ ]]; then
  echo "error: version must be a positive number or 'auto', got '${expected}'" >&2
  exit 1
fi

# Placeholders occupy version numbers so the real publish lands on the expected
# one. They are never made public, and are safe to delete.
pad_to() {
  local target="$1" latest="$2" workdir v
  workdir="$(mktemp -d)"
  echo "Placeholder to align Lambda layer version numbering across regions. Safe to delete." \
    > "${workdir}/README-placeholder.txt"
  (cd "${workdir}" && zip -q placeholder.zip README-placeholder.txt)
  while [ "${latest}" -lt "${target}" ]; do
    v="$(aws lambda publish-layer-version --layer-name "${layer}" \
      --zip-file "fileb://${workdir}/placeholder.zip" \
      --description "Placeholder to align layer version numbering across regions - safe to delete" \
      --query Version --output text --no-cli-pager)"
    echo "Published placeholder ${layer} version ${v}"
    if [ "${v}" -gt "${target}" ]; then
      echo "error: ${layer} placeholder got version ${v}, above ${target}; the version counter is ahead, so version $((target + 1)) can't be published here" >&2
      rm -rf "${workdir}"
      exit 1
    fi
    latest="${v}"
  done
  rm -rf "${workdir}"
}

version=""
if [ "${expected}" != auto ]; then
  # Newest first. The description tells a real layer from a placeholder.
  read -r latest latest_description < <(aws lambda list-layer-versions --layer-name "${layer}" \
    --max-items 1 --query 'LayerVersions[0].[Version,Description]' --output text --no-cli-pager | head -1)
  if [ "${latest}" = None ]; then
    latest=0
  fi
  if [ "${latest}" -gt "${expected}" ]; then
    echo "error: ${layer} is already at version ${latest}, above ${expected}" >&2
    exit 1
  elif [ "${latest}" -eq "${expected}" ]; then
    if [ "${latest_description}" != "${description}" ]; then
      echo "error: ${layer} version ${expected} exists but is not a published layer (description: '${latest_description}')" >&2
      exit 1
    fi
    echo "::warning::${layer} version ${expected} is already published; skipping the publish and only making sure it is public"
    version="${expected}"
  else
    pad_to "$((expected - 1))" "${latest}"
  fi
fi

if [ -z "${version}" ]; then
  echo "Publishing Lambda Extension to layer \"${layer}\""
  read -ra runtimes <<< "$(python3 "${tools_dir}/layer-runtimes.py" "${distro}")"
  version="$(aws lambda publish-layer-version --layer-name "${layer}" --zip-file "fileb://${zip}" \
    --description "${description}" \
    --compatible-runtimes "${runtimes[@]}" \
    --compatible-architectures x86_64 arm64 --no-cli-pager --output json \
    | python3 "${tools_dir}/layer-runtimes.py" "${distro}" --check)"
  if [ "${expected}" != auto ] && [ "${version}" != "${expected}" ]; then
    echo "error: ${layer} was published as version ${version}, expected ${expected}; not making it public" >&2
    exit 1
  fi
fi

echo "Making ${layer} version ${version} public"
if ! out="$(aws lambda add-layer-version-permission --layer-name "${layer}" --version-number "${version}" \
    --statement-id public --action lambda:GetLayerVersion --principal "*" --no-cli-pager 2>&1)"; then
  if ! grep -q ResourceConflictException <<< "${out}"; then
    echo "${out}" >&2
    exit 1
  fi
  echo "${layer} version ${version} is already public"
fi
