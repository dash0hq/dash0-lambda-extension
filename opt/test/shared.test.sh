#!/usr/bin/env bash
#
# Tests for opt/shared.sh, which every language wrapper sources before the runtime starts.
#
# Covers map_masking_env: mapping the documented DASH0_MASK_* variables onto the
# LUMIGO_SECRET_MASKING_REGEX* variables read by the Node and Python distros. Whether the
# mapped values then mask what each distro captures is tested in that distro's own suite
# (e.g. opt/node/test/masking-env.test.ts).
#
# Usage: bash opt/test/shared.test.sh

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SHARED_SH="$SCRIPT_DIR/../shared.sh"
RUST_MASKING="$SCRIPT_DIR/../../src/otlp/masking.rs"

DEFAULT_RULES='[".*pass.*",".*key.*",".*secret.*",".*credential.*",".*passphrase.*",".*token.*",".*signature.*",".*authorization.*"]'

failures=0
tests=0

# run_mapping <env assignments...> -- prints the environment after map_masking_env,
# one VAR=value per line, restricted to the masking variables. stderr goes to $STDERR_FILE.
STDERR_FILE="$(mktemp)"
trap 'rm -f "$STDERR_FILE"' EXIT

mapped() {
  local var="$1"
  shift
  env -i PATH="$PATH" "$@" bash -c 'source "$1" && map_masking_env && printf "%s" "${!2-<unset>}"' \
    bash "$SHARED_SH" "$var" 2>"$STDERR_FILE"
}

assert_eq() {
  local name="$1" expected="$2" actual="$3"
  tests=$((tests + 1))
  if [[ "$expected" == "$actual" ]]; then
    echo "ok - $name"
  else
    failures=$((failures + 1))
    echo "not ok - $name"
    echo "    expected: $expected"
    echo "    actual:   $actual"
  fi
}

assert_contains() {
  local name="$1" needle="$2" haystack="$3"
  tests=$((tests + 1))
  if [[ "$haystack" == *"$needle"* ]]; then
    echo "ok - $name"
  else
    failures=$((failures + 1))
    echo "not ok - $name"
    echo "    expected to contain: $needle"
    echo "    actual:              $haystack"
  fi
}

# --- defaults ---------------------------------------------------------------------------

assert_eq "injects the unified default rules when nothing is configured" \
  "$DEFAULT_RULES" "$(mapped LUMIGO_SECRET_MASKING_REGEX)"

rust_rules="[$(sed -n '/DEFAULT_MASKING_PATTERNS: &\[&str\] = &\[/,/\];/p' "$RUST_MASKING" |
  grep -o '"[^"]*"' | paste -sd, -)]"
assert_eq "uses the same default rules as the Rust extension (src/otlp/masking.rs)" \
  "$DEFAULT_RULES" "$rust_rules"

assert_eq "does not inject defaults over a legacy LUMIGO_BLACKLIST_REGEX" \
  "<unset>" "$(mapped LUMIGO_SECRET_MASKING_REGEX LUMIGO_BLACKLIST_REGEX='[".*x.*"]')"

assert_eq "does not inject defaults over a legacy LUMIGO_SECRET_MASKING_REGEX" \
  '[".*x.*"]' "$(mapped LUMIGO_SECRET_MASKING_REGEX LUMIGO_SECRET_MASKING_REGEX='[".*x.*"]')"

# --- mapping ----------------------------------------------------------------------------

value='[".*a b.*", "x\"y"]'
for mapping in \
  DASH0_MASK_RULES:LUMIGO_SECRET_MASKING_REGEX \
  DASH0_MASK_REQUEST_BODY:LUMIGO_SECRET_MASKING_REGEX_HTTP_REQUEST_BODIES \
  DASH0_MASK_RESPONSE_BODY:LUMIGO_SECRET_MASKING_REGEX_HTTP_RESPONSE_BODIES \
  DASH0_MASK_REQUEST_HEADERS:LUMIGO_SECRET_MASKING_REGEX_HTTP_REQUEST_HEADERS \
  DASH0_MASK_RESPONSE_HEADERS:LUMIGO_SECRET_MASKING_REGEX_HTTP_RESPONSE_HEADERS \
  DASH0_MASK_QUERY_PARAMS:LUMIGO_SECRET_MASKING_REGEX_HTTP_QUERY_PARAMS \
  DASH0_MASK_ENV_VARS:LUMIGO_SECRET_MASKING_REGEX_ENVIRONMENT; do
  dash0_var="${mapping%%:*}"
  lumigo_var="${mapping#*:}"
  assert_eq "maps $dash0_var onto $lumigo_var verbatim" \
    "$value" "$(mapped "$lumigo_var" "$dash0_var=$value")"
  assert_eq "$dash0_var overrides $lumigo_var" \
    '[".*email.*"]' "$(mapped "$lumigo_var" "$lumigo_var=[\".*x.*\"]" "$dash0_var=[\".*email.*\"]")"
  assert_eq "accepts $dash0_var=all" "all" "$(mapped "$lumigo_var" "$dash0_var=all")"
done

assert_eq "leaves legacy per-payload LUMIGO_* vars alone" \
  '[".*x.*"]' \
  "$(mapped LUMIGO_SECRET_MASKING_REGEX_HTTP_REQUEST_BODIES LUMIGO_SECRET_MASKING_REGEX_HTTP_REQUEST_BODIES='[".*x.*"]')"

assert_eq "does not set per-payload vars that are not configured" \
  "<unset>" "$(mapped LUMIGO_SECRET_MASKING_REGEX_HTTP_REQUEST_BODIES DASH0_MASK_RULES='[".*x.*"]')"

# --- invalid values ---------------------------------------------------------------------

for invalid in 'not json' '[".*email.*"' '".*email.*"' '{"a": 1}' \
  '[null]' '[1]' '[".*email.*",]' '[.*email.*]' '[".*a.*" ".*b.*"]' '[".*a.*\"]' '[[".*a.*"]]'; do
  assert_eq "DASH0_MASK_RULES=$invalid falls back to the defaults, like the Rust extension" \
    "$DEFAULT_RULES" "$(mapped LUMIGO_SECRET_MASKING_REGEX DASH0_MASK_RULES="$invalid")"
done

assert_eq "an invalid DASH0_MASK_RULES leaves a legacy LUMIGO_SECRET_MASKING_REGEX in place" \
  '[".*x.*"]' \
  "$(mapped LUMIGO_SECRET_MASKING_REGEX DASH0_MASK_RULES='not json' LUMIGO_SECRET_MASKING_REGEX='[".*x.*"]')"

assert_eq "an invalid per-payload var is dropped, so that payload falls back to DASH0_MASK_RULES" \
  "<unset>" "$(mapped LUMIGO_SECRET_MASKING_REGEX_HTTP_REQUEST_BODIES DASH0_MASK_REQUEST_BODY=nope)"

mapped LUMIGO_SECRET_MASKING_REGEX DASH0_MASK_RULES='not json' DASH0_MASK_QUERY_PARAMS=nope >/dev/null
stderr="$(cat "$STDERR_FILE")"
assert_contains "warns on stderr naming DASH0_MASK_RULES" "DASH0_MASK_RULES" "$stderr"
assert_contains "warns on stderr naming DASH0_MASK_QUERY_PARAMS" "DASH0_MASK_QUERY_PARAMS" "$stderr"

mapped LUMIGO_SECRET_MASKING_REGEX DASH0_MASK_RULES=$' [".*a.*"]\n' DASH0_MASK_REQUEST_BODY=all >/dev/null
assert_eq "accepts JSON arrays with surrounding whitespace and \"all\" without warning" \
  "" "$(cat "$STDERR_FILE")"

for valid in '[]' '[ ]' '[".*a.*", ".*b.*"]' $'[\n  ".*a.*",\n  ".*b.*"\n]' '["^x\\.y$"]' '["say \"hi\""]' '[",", "]"]'; do
  assert_eq "accepts the JSON array $valid" \
    "$valid" "$(mapped LUMIGO_SECRET_MASKING_REGEX DASH0_MASK_RULES="$valid")"
done

# --- DASH0_MASK_IN_EXTENSION ------------------------------------------------------------

lumigo_vars=(LUMIGO_SECRET_MASKING_REGEX LUMIGO_SECRET_MASKING_REGEX_HTTP_REQUEST_BODIES
  LUMIGO_SECRET_MASKING_REGEX_HTTP_RESPONSE_BODIES LUMIGO_SECRET_MASKING_REGEX_HTTP_REQUEST_HEADERS
  LUMIGO_SECRET_MASKING_REGEX_HTTP_RESPONSE_HEADERS LUMIGO_SECRET_MASKING_REGEX_HTTP_QUERY_PARAMS
  LUMIGO_SECRET_MASKING_REGEX_ENVIRONMENT)

for flag in true TRUE True 1 yes y Y; do
  for lumigo_var in "${lumigo_vars[@]}"; do
    assert_eq "DASH0_MASK_IN_EXTENSION=$flag turns off distro masking: $lumigo_var=[]" \
      "[]" "$(mapped "$lumigo_var" DASH0_MASK_IN_EXTENSION="$flag")"
  done
done

assert_eq "DASH0_MASK_IN_EXTENSION=true ignores DASH0_MASK_* for the distros" \
  "[]" "$(mapped LUMIGO_SECRET_MASKING_REGEX_HTTP_REQUEST_BODIES DASH0_MASK_IN_EXTENSION=true \
    DASH0_MASK_RULES=all DASH0_MASK_REQUEST_BODY='[".*email.*"]')"

assert_eq "DASH0_MASK_IN_EXTENSION=true overrides a user-set LUMIGO_* variable" \
  "[]" "$(mapped LUMIGO_SECRET_MASKING_REGEX_HTTP_QUERY_PARAMS DASH0_MASK_IN_EXTENSION=true \
    LUMIGO_SECRET_MASKING_REGEX_HTTP_QUERY_PARAMS=all)"

for flag in false 0 no '' on; do
  assert_eq "DASH0_MASK_IN_EXTENSION=$flag keeps distro masking as before" \
    "$DEFAULT_RULES" "$(mapped LUMIGO_SECRET_MASKING_REGEX DASH0_MASK_IN_EXTENSION="$flag")"
done

# --- wiring -----------------------------------------------------------------------------

assert_eq "runs as part of setup_otel_env, which every language wrapper calls" \
  '[".*email.*"]' \
  "$(env -i PATH="$PATH" DASH0_MASK_RULES='[".*email.*"]' \
    bash -c 'source "$1" && setup_otel_env && printf "%s" "$LUMIGO_SECRET_MASKING_REGEX"' bash "$SHARED_SH")"

for wrapper in "$SCRIPT_DIR"/../{node,python,java,manual}/wrapper; do
  assert_contains "$(basename "$(dirname "$wrapper")")/wrapper calls setup_otel_env" \
    "setup_otel_env" "$(cat "$wrapper")"
done

echo
echo "$((tests - failures))/$tests passed"
[[ $failures -eq 0 ]]
