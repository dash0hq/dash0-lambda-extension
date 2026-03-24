#!/usr/bin/env bash
# grype-annotate.sh <grype-json-file> <layer-label>
#
# - Writes a markdown findings table to $GITHUB_STEP_SUMMARY
# - Emits ::error:: annotations for CRITICAL/HIGH findings
# - Emits ::warning:: annotations for MEDIUM/LOW findings
set -euo pipefail

RESULTS_FILE="${1:?usage: grype-annotate.sh <results.json> <label>}"
LABEL="${2:?usage: grype-annotate.sh <results.json> <label>}"
SUMMARY="${GITHUB_STEP_SUMMARY:-/dev/stdout}"

count=$(jq '.matches | length' "$RESULTS_FILE")

echo "### $LABEL — $count vulnerability finding(s)" >> "$SUMMARY"
echo "" >> "$SUMMARY"

if [ "$count" -eq 0 ]; then
  echo ":white_check_mark: No vulnerabilities found." >> "$SUMMARY"
  exit 0
fi

# Step summary table
echo "| Package | Installed | Fixed In | Severity | Vulnerability |" >> "$SUMMARY"
echo "|---|---|---|---|---|" >> "$SUMMARY"
jq -r '
  .matches[] |
  (.vulnerability.fix.versions | if length > 0 then join(", ") else "—" end) as $fix |
  "| \(.artifact.name) | \(.artifact.version) | \($fix) | \(.vulnerability.severity) | [\(.vulnerability.id)](https://osv.dev/vulnerability/\(.vulnerability.id)) |"
' "$RESULTS_FILE" >> "$SUMMARY"

# Inline annotations
jq -r '
  .matches[] |
  (if (.vulnerability.severity | ascii_downcase) == "critical" or
      (.vulnerability.severity | ascii_downcase) == "high"
   then "error" else "warning" end) as $level |
  (.vulnerability.fix.versions | if length > 0 then "fix available: " + join(", ") else "no fix available" end) as $fix |
  "::\($level) title=[\(.vulnerability.severity)] \(.vulnerability.id)::\(.artifact.name) \(.artifact.version) — \(.vulnerability.id) (\(.vulnerability.severity)), \($fix)"
' "$RESULTS_FILE"
