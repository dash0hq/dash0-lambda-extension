#!/bin/bash

write_env_vars() {
  env -0 | {
    echo -n "{"
    first=true
    while IFS= read -r -d '' entry; do
      [ -z "$entry" ] && continue
      key="${entry%%=*}"
      value="${entry#*=}"
      value="${value//\\/\\\\}"
      value="${value//\"/\\\"}"
      value="${value//$'\n'/\\n}"
      value="${value//$'\t'/\\t}"
      value="${value//$'\r'/\\r}"
      if [ "$first" = true ]; then first=false; else echo -n ","; fi
      echo -n "\"$key\":\"$value\""
    done
    echo -n "}"
  } > /tmp/dash0_env_vars
}
