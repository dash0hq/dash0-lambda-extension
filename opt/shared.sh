#!/bin/bash

setup_otel_env() {
  export AWS_LAMBDA_RUNTIME_API="127.0.0.1:9009"

  LAMBDA_RESOURCE_ATTRIBUTES="cloud.region=$AWS_REGION,cloud.provider=aws,cloud_platform=aws_lambda,faas.name=$AWS_LAMBDA_FUNCTION_NAME,faas.version=$AWS_LAMBDA_FUNCTION_VERSION,faas.instance=$AWS_LAMBDA_LOG_STREAM_NAME";

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
