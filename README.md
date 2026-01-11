# Dash0 Lambda Extension

An extension for capturing observability data from lambda invocations and shipping to Dash0.

This extension has four main functionalities:
1. Enable auto-instrumentation for supported runtimes, which currently include Python, Node, Java.
2. Receive traces from auto/manual instrumentations, enrich with data acquired in the extension, and send to Dash0.
3. Detect runtime errors such as timeout or out of memory and create synthetic traces for them
4. Collect all logs and send to Dash0, correlated with the trace id of the invocation.


## Configuration

* `AWS_LAMBDA_EXEC_WRAPPER=/opt/wrapper` - This environment variable must be set in order to enable tracing. If this environment variable will not be set, only logs will be collected.

* `DASH0_AUTH` - the api token for your Dash0 project.

* `DISABLE_AUTO_INSTRUMENTATION` - Auto-instrumentation can be turned off by this environment variable, which will result in creating synthetic traces by the extension for all invocations.

* `SEND_ON_INVOCATION_END` - The extension has two modes of sending to the backend, either on invocation end or on the next invocations. This is controlled by the env var `SEND_ON_INVOCATION_END`. The default is `true`. Sending on invocation end will increase the billed duration of the lambda, but not the response time. Sending on next invocation will decrease the billed duration since the sending will take place in parallel of the regular execution, but might delay the sending up to 7 minutes in case of last invocation in the container. 


