#!/usr/bin/env bash
#
# Retry a command with linear backoff, tolerating "already gone" outcomes.
#
# Usage: retry.sh <max_attempts> <base_delay_seconds> <description> -- <command> [args...]
#
# Behaviour:
#   - Runs the command, streaming its output live.
#   - Exit 0                          -> success.
#   - Output matches a "benign" regex -> the target is already gone; treated as
#                                        success (e.g. PR closed before deploy).
#   - Otherwise                       -> retry after (base_delay * attempt) seconds.
#   - After <max_attempts> failures   -> propagate the last non-zero exit code so
#                                        the workflow step (and job) fails.
#
# This exists because AWS throttles destructive API calls aggressively
# (e.g. ApiGateway DeleteRestApi at ~1 req / 30s), which makes CloudFormation
# stack deletes return DELETE_FAILED with HTTP 429 "Too Many Requests".
# Re-running the destroy after a short pause lets the throttle window reset.
set -uo pipefail

max_attempts="${1:?max_attempts required}"; shift
base_delay="${1:?base_delay required}"; shift
desc="${1:?description required}"; shift
if [ "${1:-}" = "--" ]; then shift; fi

# Outputs that mean "there is nothing left to delete" -> not a failure.
benign_re='does not exist|ResourceNotFoundException|RepositoryNotFoundException|NoSuchEntity|could not be found|No stack|Stack .* does not exist'

attempt=1
while true; do
  tmp="$(mktemp)"
  "$@" 2>&1 | tee "$tmp"
  status="${PIPESTATUS[0]}"

  if [ "$status" -eq 0 ]; then
    rm -f "$tmp"
    exit 0
  fi

  if grep -qiE "$benign_re" "$tmp"; then
    echo "::notice::${desc}: nothing to delete (already gone); treating as success"
    rm -f "$tmp"
    exit 0
  fi
  rm -f "$tmp"

  if [ "$attempt" -ge "$max_attempts" ]; then
    echo "::error::${desc} failed after ${attempt} attempts (exit ${status})"
    exit "$status"
  fi

  delay=$(( base_delay * attempt ))
  echo "::warning::${desc} attempt ${attempt}/${max_attempts} failed (exit ${status}); retrying in ${delay}s"
  sleep "${delay}"
  attempt=$(( attempt + 1 ))
done
