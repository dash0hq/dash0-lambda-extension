#!/bin/bash

setup_otel_env() {
  export AWS_LAMBDA_RUNTIME_API="127.0.0.1:9009"

  LAMBDA_RESOURCE_ATTRIBUTES="cloud.region=$AWS_REGION,cloud.provider=aws,cloud.platform=aws_lambda,faas.name=$AWS_LAMBDA_FUNCTION_NAME,faas.version=$AWS_LAMBDA_FUNCTION_VERSION,faas.instance=$AWS_LAMBDA_LOG_STREAM_NAME";

  # Add extension git hash if the file was baked in at build time
  if [[ -f /opt/dash0_git_hash ]]; then
    LAMBDA_RESOURCE_ATTRIBUTES="$LAMBDA_RESOURCE_ATTRIBUTES,faas.extension.git_hash=$(cat /opt/dash0_git_hash)"
  fi

  if [ -z "${OTEL_RESOURCE_ATTRIBUTES}" ]; then
      export OTEL_RESOURCE_ATTRIBUTES=$LAMBDA_RESOURCE_ATTRIBUTES;
  else
      export OTEL_RESOURCE_ATTRIBUTES="$LAMBDA_RESOURCE_ATTRIBUTES,$OTEL_RESOURCE_ATTRIBUTES";
  fi

  if [ -z "${OTEL_SERVICE_NAME}" ]; then
      export OTEL_SERVICE_NAME=$AWS_LAMBDA_FUNCTION_NAME;
  fi

  if [[ -z "$OTEL_PROPAGATORS" ]]; then
    export OTEL_PROPAGATORS="tracecontext,baggage,xray-lambda"
  fi

  if [[ -z "$OTEL_TRACES_SAMPLER" ]]; then
    export OTEL_TRACES_SAMPLER="always_on"
  fi

  if [[ -z "$ENABLE_KINESIS_INSTRUMENTATION" ]]; then
    export ENABLE_KINESIS_INSTRUMENTATION="true"
  fi

  map_masking_env
}

# Default secret-masking patterns. Must match DEFAULT_MASKING_PATTERNS in src/otlp/masking.rs.
DASH0_DEFAULT_MASK_RULES='[".*pass.*",".*key.*",".*secret.*",".*credential.*",".*passphrase.*",".*token.*",".*signature.*",".*authorization.*"]'

# The Node and Python distros mask payloads with Lumigo's core libraries, which only read
# LUMIGO_SECRET_MASKING_REGEX*, once, when they are first loaded. Map the documented
# DASH0_MASK_* variables onto them here, before the runtime starts. A DASH0_MASK_* value
# overrides its LUMIGO_* counterpart; LUMIGO_* still works on its own for backwards
# compatibility. Values (a JSON array of regexes, or "all") are passed through verbatim.
# Any other value, including malformed JSON such as '[null]' or '["a",]', is ignored with a
# warning, so that it falls back to DASH0_MASK_RULES or the defaults, as in the Rust
# extension, rather than to the Lumigo libraries' own built-in defaults.
is_valid_mask_value() {
  # A JSON array of strings is a regular language, so it can be validated exactly here.
  local string='"([^"\\]|\\.)*"'
  local array="^[[:space:]]*\[[[:space:]]*(${string}[[:space:]]*(,[[:space:]]*${string}[[:space:]]*)*)?\][[:space:]]*$"
  [[ "$1" == "all" || "$1" =~ $array ]]
}

valid_mask_value_of() {
  local name="$1"
  if [[ -z "${!name}" ]]; then
    return 1
  fi
  if ! is_valid_mask_value "${!name}"; then
    echo "[dash0] Ignoring $name: expected a JSON array of regular expressions or \"all\"" >&2
    return 1
  fi
}

map_masking_env() {
  if valid_mask_value_of DASH0_MASK_RULES; then
    export LUMIGO_SECRET_MASKING_REGEX="$DASH0_MASK_RULES"
  elif [[ -z "$LUMIGO_SECRET_MASKING_REGEX" && -z "$LUMIGO_BLACKLIST_REGEX" ]]; then
    export LUMIGO_SECRET_MASKING_REGEX="$DASH0_DEFAULT_MASK_RULES"
  fi

  local mapping dash0_var lumigo_var
  for mapping in \
    DASH0_MASK_REQUEST_BODY:LUMIGO_SECRET_MASKING_REGEX_HTTP_REQUEST_BODIES \
    DASH0_MASK_RESPONSE_BODY:LUMIGO_SECRET_MASKING_REGEX_HTTP_RESPONSE_BODIES \
    DASH0_MASK_REQUEST_HEADERS:LUMIGO_SECRET_MASKING_REGEX_HTTP_REQUEST_HEADERS \
    DASH0_MASK_RESPONSE_HEADERS:LUMIGO_SECRET_MASKING_REGEX_HTTP_RESPONSE_HEADERS \
    DASH0_MASK_QUERY_PARAMS:LUMIGO_SECRET_MASKING_REGEX_HTTP_QUERY_PARAMS \
    DASH0_MASK_ENV_VARS:LUMIGO_SECRET_MASKING_REGEX_ENVIRONMENT; do
    dash0_var="${mapping%%:*}"
    lumigo_var="${mapping#*:}"
    if valid_mask_value_of "$dash0_var"; then
      export "$lumigo_var=${!dash0_var}"
    fi
  done
}

write_env_vars() {
  env -0 2>/dev/null | {
    echo -n "{"
    first=true
    while IFS= read -r -d '' entry; do
      [ -z "$entry" ] && continue
      key="${entry%%=*}"
      value="${entry#*=}"
      key="${key//\\/\\\\}"
      key="${key//\"/\\\"}"
      value="${value//\\/\\\\}"
      value="${value//\"/\\\"}"
      value="${value//$'\n'/\\n}"
      value="${value//$'\t'/\\t}"
      value="${value//$'\r'/\\r}"
      if [ "$first" = true ]; then first=false; else echo -n ","; fi
      echo -n "\"$key\":\"$value\""
    done
    echo -n "}"
  } > /tmp/dash0_env_vars 2>/dev/null || true
}
