#!/bin/bash
# Startup script executed by the AWS Lambda Web Adapter (the function handler
# in Web Adapter scenarios). Inherits NODE_OPTIONS from the exec wrapper chain,
# so the Dash0 auto-instrumentation loads here when the chained wrapper is used.
exec node "${LAMBDA_TASK_ROOT:-/var/task}/server.js"
