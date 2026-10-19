# Secret masking by example

This page shows what each masking option does to the payloads the Dash0 Lambda layer captures. Every example gives the environment variable, the payload as your function sent or received it, and the value that reaches Dash0.

The examples use a Node.js function. Support differs by runtime, so each section starts with a line like this:

> **Node.js** ✅ · **Python** ⚠️ · **Java** ❌

- ✅ works as described
- ⚠️ works in part; the note below the line says what differs
- ❌ has no effect; the payload is sent as captured

Masking of the Lambda event, the return value and environment variables is done by the Dash0 extension, so it works the same in every runtime.

- [How masking works](#how-masking-works)
- [Where each payload ends up](#where-each-payload-ends-up)
- [The example payloads](#the-example-payloads)
- [Defaults: nothing configured](#defaults-nothing-configured)
- [`DASH0_MASK_RULES`](#dash0_mask_rules)
- [`DASH0_MASK_REQUEST_BODY`](#dash0_mask_request_body)
- [`DASH0_MASK_RESPONSE_BODY`](#dash0_mask_response_body)
- [`DASH0_MASK_REQUEST_HEADERS`](#dash0_mask_request_headers)
- [`DASH0_MASK_RESPONSE_HEADERS`](#dash0_mask_response_headers)
- [`DASH0_MASK_QUERY_PARAMS`](#dash0_mask_query_params)
- [Combining variables](#combining-variables)
- [`DASH0_MASK_ENV_VARS`](#dash0_mask_env_vars)
- [Lambda event and return value](#lambda-event-and-return-value)
- [Invalid values](#invalid-values)
- [Payloads that are strings](#payloads-that-are-strings)
- [Payloads that are not masked](#payloads-that-are-not-masked)

## How masking works

- **Masking goes by key name.** A pattern is a regular expression that is checked against every key in the payload, at any depth. The check ignores case and matches anywhere in the key, so `.*email.*` and `email` both match `x-email`. Use `^email$` to match only the exact key.
  - **Python:** patterns must match from the start of the key, so `email` does not match `x-email`. Write patterns as `.*email.*` to get the same result in every runtime.
- **A matching key masks its whole value.** The value becomes `"****"`, even when it is an object or an array.
- **Values are never checked.** A key named `note` that contains an email address is left alone.
- **Strings are not parsed.** When a payload is a string, such as an SQS `MessageBody`, a Kafka message or a Redis command, nothing inside it is checked, even when it holds JSON. See [Payloads that are strings](#payloads-that-are-strings).
  - **Node.js:** HTTP bodies sent as `application/json` are parsed and masked by key. Other strings are not parsed.
  - **Python:** any string that starts with `{` is parsed and masked by key.
- **`all` replaces the whole payload** with `"****"`.
- **Your list replaces the defaults.** `DASH0_MASK_RULES` replaces the [default patterns](#defaults-nothing-configured). A per-payload variable replaces the rules for its own payload type. To add to the defaults, list them too.

## Where each payload ends up

| Payload | Span attribute | Controlled by | Node.js | Python | Java |
|---|---|---|---|---|---|
| Query string of requests your app receives | `http.request.query` | `DASH0_MASK_QUERY_PARAMS`, then `DASH0_MASK_RULES` | ✅ Express, Fastify | ❌ | ❌ |
| HTTP request headers | `http.request.headers` | `DASH0_MASK_REQUEST_HEADERS`, then `DASH0_MASK_RULES` | ✅ `http`/`https`, `fetch`, Express, Fastify | ⚠️ not `requests` or Django | ❌ |
| HTTP request body | `http.request.body` ¹ | `DASH0_MASK_REQUEST_BODY`, then `DASH0_MASK_RULES` | ✅ `http`/`https`, `fetch`, Express, Fastify | ⚠️ FastAPI and AWS SDK only; not `requests`, `httpx` or Django | ❌ |
| HTTP response headers | `http.response.headers` | `DASH0_MASK_RESPONSE_HEADERS`, then `DASH0_MASK_RULES` | ✅ `http`/`https`, `fetch`, Express; Fastify captures none | ⚠️ not Flask | ❌ |
| HTTP response body | `http.response.body` ¹ | `DASH0_MASK_RESPONSE_BODY`, then `DASH0_MASK_RULES` | ✅ `http`/`https`, Express; `fetch` and Fastify capture none | ⚠️ depends on the library | ❌ |
| SQS `SendMessage` / `SendMessageBatch` input | `messaging.publish.body` | `DASH0_MASK_REQUEST_BODY`, then `DASH0_MASK_RULES` | ✅ | ⚠️ `SendMessage` captures only `MessageBody`, in `http.request.body` | ❌ |
| SQS `ReceiveMessage` output | `messaging.consume.body` | `DASH0_MASK_RESPONSE_BODY`, then `DASH0_MASK_RULES` | ✅ | not captured | ❌ |
| Kafka message sent | `messaging.produce.body` | `DASH0_MASK_QUERY_PARAMS`, then `DASH0_MASK_RULES` ² | ✅ `kafkajs` | ❌ | ❌ |
| RabbitMQ message published | `messaging.publish.body` | `DASH0_MASK_QUERY_PARAMS`, then `DASH0_MASK_RULES` ² | ✅ `amqplib` | ❌ | ❌ |
| Kafka or RabbitMQ message received | `messaging.consume.body` | `DASH0_MASK_RESPONSE_BODY`, then `DASH0_MASK_RULES` | ✅ | ❌ | ❌ |
| Redis command | `db.statement` | `DASH0_MASK_REQUEST_BODY`, then `DASH0_MASK_RULES` | ✅ `redis`, `ioredis` | ❌ | ❌ |
| Redis reply | `db.response.body` | `DASH0_MASK_RESPONSE_BODY`, then `DASH0_MASK_RULES` | ✅ `redis`, `ioredis` | ❌ | ❌ |
| Lambda event and return value | `dash0_payload` log records | `DASH0_MASK_RULES` | ✅ | ✅ | ✅ |
| Environment variables | `process.environment_variable.*` resource attributes | `DASH0_MASK_ENV_VARS`, then `DASH0_MASK_RULES` | ✅ | ✅ | ✅ |

¹ The extension moves HTTP bodies off the span into a `dash0_payload` log record of type `http_request_body` or `http_response_body`. It doesn't mask them again, so what you see below is what the log record contains. In Java, these records contain the bodies unmasked.

² Messages sent to Kafka and RabbitMQ are masked with the query-parameter rules, not the request-body rules.

Some payloads are captured but not masked at all. See [Payloads that are not masked](#payloads-that-are-not-masked).

"Then" means: the per-payload variable applies if it is set and valid; otherwise `DASH0_MASK_RULES` applies; otherwise the defaults.

Attribute values are stored as compact JSON strings. They are pretty-printed here for readability.

## The example payloads

All examples use the same payloads. Each one has three kinds of keys:

- `email`: personal data that isn't masked by default
- `password`, `apiToken`, `authorization`: secrets that are masked by default
- `orderId`: data that is never masked

<details>
<summary>Show the unmasked payloads</summary>

`http.request.query`

```json
{ "email": "jane@example.com", "password": "q-pw", "orderId": "42" }
```

`http.request.headers`

```json
{
  "content-type": "application/json",
  "x-email": "jane@example.com",
  "x-password": "h-pw",
  "authorization": "Bearer abc",
  "x-signature": "sig",
  "x-orderid": "42"
}
```

`http.request.body`

```json
{ "email": "jane@example.com", "password": "b-pw", "apiToken": "t", "orderId": "42" }
```

`http.response.headers`

```json
{ "content-type": "application/json", "x-email": "jane@example.com", "x-password": "h-pw", "x-orderid": "42" }
```

`http.response.body`

```json
{ "email": "jane@example.com", "password": "r-pw", "sessionToken": "s", "orderId": "42" }
```

`messaging.publish.body`

```json
{
  "QueueUrl": "https://sqs.eu-central-1.amazonaws.com/123456789012/orders",
  "MessageBody": "order 42 for jane@example.com",
  "MessageAttributes": {
    "email": { "DataType": "String", "StringValue": "jane@example.com" },
    "password": { "DataType": "String", "StringValue": "m-pw" },
    "orderId": { "DataType": "String", "StringValue": "42" }
  }
}
```

`messaging.consume.body`

```json
{
  "Messages": [{
    "MessageId": "m1",
    "Body": "order 42 for jane@example.com",
    "MessageAttributes": {
      "email": { "DataType": "String", "StringValue": "jane@example.com" },
      "password": { "DataType": "String", "StringValue": "m-pw" },
      "orderId": { "DataType": "String", "StringValue": "42" }
    }
  }]
}
```

</details>

## Defaults: nothing configured

> **Node.js** ✅ · **Python** ⚠️ · **Java** ❌

Python applies the defaults only to the payloads it masks (see the [table](#where-each-payload-ends-up)). Java applies them to the Lambda event, the return value and environment variables only.

```bash
# no DASH0_MASK_* variables
```

These patterns apply everywhere:

```json
[".*pass.*", ".*key.*", ".*secret.*", ".*credential.*", ".*passphrase.*", ".*token.*", ".*signature.*", ".*authorization.*"]
```

`http.request.headers`

```diff
 {
   "content-type": "application/json",
   "x-email": "jane@example.com",
-  "x-password": "h-pw",
-  "authorization": "Bearer abc",
-  "x-signature": "sig",
+  "x-password": "****",
+  "authorization": "****",
+  "x-signature": "****",
   "x-orderid": "42"
 }
```

`http.request.body`

```diff
-{ "email": "jane@example.com", "password": "b-pw", "apiToken": "t", "orderId": "42" }
+{ "email": "jane@example.com", "password": "****", "apiToken": "****", "orderId": "42" }
```

`messaging.publish.body`

```diff
 {
   "QueueUrl": "https://sqs.eu-central-1.amazonaws.com/123456789012/orders",
   "MessageBody": "order 42 for jane@example.com",
   "MessageAttributes": {
     "email": { "DataType": "String", "StringValue": "jane@example.com" },
-    "password": { "DataType": "String", "StringValue": "m-pw" },
+    "password": "****",
     "orderId": { "DataType": "String", "StringValue": "42" }
   }
 }
```

The secrets are masked. The email address and the message body are not, because no default pattern matches them. The query, the response headers and body, and the consume body follow the same pattern.

## `DASH0_MASK_RULES`

Sets the patterns for every payload. It replaces the defaults.

### Only your own patterns

> **Node.js** ✅ · **Python** ⚠️ · **Java** ⚠️

Python applies the rules only to the payloads it masks. Java applies them to the Lambda event, the return value and environment variables only.

```bash
DASH0_MASK_RULES='[".*email.*"]'
```

`http.request.body`

```diff
-{ "email": "jane@example.com", "password": "b-pw", "apiToken": "t", "orderId": "42" }
+{ "email": "****", "password": "b-pw", "apiToken": "t", "orderId": "42" }
```

`http.request.headers`

```diff
 {
   "content-type": "application/json",
-  "x-email": "jane@example.com",
+  "x-email": "****",
   "x-password": "h-pw",
   "authorization": "Bearer abc",
   "x-signature": "sig",
   "x-orderid": "42"
 }
```

The email address is masked everywhere. The password, token and `authorization` header are now **not** masked, because the defaults were replaced.

> [!WARNING]
> The same list also applies to environment variables, unless you set [`DASH0_MASK_ENV_VARS`](#dash0_mask_env_vars). Without `.*key.*`, `.*secret.*` and `.*token.*`, the function's `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` and `AWS_SESSION_TOKEN` are sent unmasked. Include the defaults, as in the next example.

### Your patterns plus the defaults

> **Node.js** ✅ · **Python** ⚠️ · **Java** ⚠️

Python applies the rules only to the payloads it masks, and doesn't capture the SQS receive below. Java applies them to the Lambda event, the return value and environment variables only.

```bash
DASH0_MASK_RULES='[".*pass.*",".*key.*",".*secret.*",".*credential.*",".*passphrase.*",".*token.*",".*signature.*",".*authorization.*",".*email.*"]'
```

`http.request.body`

```diff
-{ "email": "jane@example.com", "password": "b-pw", "apiToken": "t", "orderId": "42" }
+{ "email": "****", "password": "****", "apiToken": "****", "orderId": "42" }
```

`messaging.consume.body`

```diff
 {
   "Messages": [{
     "MessageId": "m1",
     "Body": "order 42 for jane@example.com",
     "MessageAttributes": {
-      "email": { "DataType": "String", "StringValue": "jane@example.com" },
-      "password": { "DataType": "String", "StringValue": "m-pw" },
+      "email": "****",
+      "password": "****",
       "orderId": { "DataType": "String", "StringValue": "42" }
     }
   }]
 }
```

This is the setting to use when you want to mask more than the defaults.

### `all`

> **Node.js** ✅ · **Python** ❌ · **Java** ❌

In Python, `DASH0_MASK_RULES=all` breaks masking and the payloads are sent unmasked. Use `all` in the per-payload variables instead.

```bash
DASH0_MASK_RULES=all
```

Every payload the Node.js distro masks becomes `"****"`:

```diff
-{ "email": "jane@example.com", "password": "b-pw", "apiToken": "t", "orderId": "42" }
+"****"
```

This doesn't cover the [payloads that are not masked](#payloads-that-are-not-masked).

> [!NOTE]
> The extension doesn't accept `all` in `DASH0_MASK_RULES`. It uses the defaults for the Lambda event, the return value and environment variables, and logs `Failed to parse DASH0_MASK_RULES as JSON array of strings: … Using default masking rules.`

## `DASH0_MASK_REQUEST_BODY`

Sets the patterns for request bodies: `http.request.body`, the SQS `messaging.publish.body` and Redis commands. Inside those payloads it replaces `DASH0_MASK_RULES` and the defaults. Every other payload keeps its rules.

### Patterns

> **Node.js** ✅ · **Python** ⚠️ · **Java** ❌

Python applies it to FastAPI and AWS SDK request bodies only. Its SQS `SendMessage` capture contains only `MessageBody`, so the `MessageAttributes` example below doesn't apply.

```bash
DASH0_MASK_REQUEST_BODY='[".*email.*"]'
```

`http.request.body`

```diff
-{ "email": "jane@example.com", "password": "b-pw", "apiToken": "t", "orderId": "42" }
+{ "email": "****", "password": "b-pw", "apiToken": "t", "orderId": "42" }
```

`messaging.publish.body`

```diff
 {
   "QueueUrl": "https://sqs.eu-central-1.amazonaws.com/123456789012/orders",
   "MessageBody": "order 42 for jane@example.com",
   "MessageAttributes": {
-    "email": { "DataType": "String", "StringValue": "jane@example.com" },
+    "email": "****",
     "password": { "DataType": "String", "StringValue": "m-pw" },
     "orderId": { "DataType": "String", "StringValue": "42" }
   }
 }
```

`http.response.body`: unchanged from the defaults

```diff
-{ "email": "jane@example.com", "password": "r-pw", "sessionToken": "s", "orderId": "42" }
+{ "email": "jane@example.com", "password": "****", "sessionToken": "****", "orderId": "42" }
```

The password in the request body is no longer masked, because the per-payload list replaced the defaults there. Add the defaults to the list to keep them.

### `all`

> **Node.js** ✅ · **Python** ⚠️ · **Java** ❌

Python applies it to FastAPI and AWS SDK request bodies only.

```bash
DASH0_MASK_REQUEST_BODY=all
```

`http.request.body`

```diff
-{ "email": "jane@example.com", "password": "b-pw", "apiToken": "t", "orderId": "42" }
+"****"
```

`messaging.publish.body`

```diff
-{ "QueueUrl": "https://sqs.eu-central-1.amazonaws.com/123456789012/orders", "MessageBody": "order 42 for jane@example.com", "MessageAttributes": { … } }
+"****"
```

The queue URL is masked as well. Received messages are covered by [`DASH0_MASK_RESPONSE_BODY`](#dash0_mask_response_body). The bodies of SQS messages that trigger your function are part of the Lambda event, which only [`DASH0_MASK_RULES`](#lambda-event-and-return-value) covers.

## `DASH0_MASK_RESPONSE_BODY`

Sets the patterns for response bodies: `http.response.body`, the SQS, Kafka and RabbitMQ `messaging.consume.body`, and Redis replies.

### Patterns

> **Node.js** ✅ · **Python** ⚠️ · **Java** ❌

Python applies it only to the response bodies it masks, and doesn't capture SQS receives.

```bash
DASH0_MASK_RESPONSE_BODY='[".*email.*"]'
```

`http.response.body`

```diff
-{ "email": "jane@example.com", "password": "r-pw", "sessionToken": "s", "orderId": "42" }
+{ "email": "****", "password": "r-pw", "sessionToken": "s", "orderId": "42" }
```

`messaging.consume.body`

```diff
 {
   "Messages": [{
     "MessageId": "m1",
     "Body": "order 42 for jane@example.com",
     "MessageAttributes": {
-      "email": { "DataType": "String", "StringValue": "jane@example.com" },
+      "email": "****",
       "password": { "DataType": "String", "StringValue": "m-pw" },
       "orderId": { "DataType": "String", "StringValue": "42" }
     }
   }]
 }
```

### `all`

> **Node.js** ✅ · **Python** ⚠️ · **Java** ❌

Same coverage as the patterns above.

```bash
DASH0_MASK_RESPONSE_BODY=all
```

`http.response.body` and `messaging.consume.body`

```diff
-{ "email": "jane@example.com", "password": "r-pw", "sessionToken": "s", "orderId": "42" }
+"****"
```

## `DASH0_MASK_REQUEST_HEADERS`

Sets the patterns for `http.request.headers`.

### Patterns

> **Node.js** ✅ · **Python** ⚠️ · **Java** ❌

Python doesn't mask headers sent with `requests` or received by Django.

```bash
DASH0_MASK_REQUEST_HEADERS='[".*email.*"]'
```

`http.request.headers`

```diff
 {
   "content-type": "application/json",
-  "x-email": "jane@example.com",
+  "x-email": "****",
   "x-password": "h-pw",
   "authorization": "Bearer abc",
   "x-signature": "sig",
   "x-orderid": "42"
 }
```

Be careful with this one: the `authorization` header is no longer masked. Include `".*authorization.*"` in the list to keep it masked.

### `all`

> **Node.js** ✅ · **Python** ⚠️ · **Java** ❌

Same coverage as the patterns above.

```bash
DASH0_MASK_REQUEST_HEADERS=all
```

`http.request.headers`

```diff
-{ "content-type": "application/json", "x-email": "jane@example.com", "x-password": "h-pw", "authorization": "Bearer abc", "x-signature": "sig", "x-orderid": "42" }
+"****"
```

## `DASH0_MASK_RESPONSE_HEADERS`

Sets the patterns for `http.response.headers`.

> **Node.js** ✅ · **Python** ⚠️ · **Java** ❌

Fastify captures no response headers. Python doesn't mask Flask response headers.

```bash
DASH0_MASK_RESPONSE_HEADERS='[".*email.*"]'
```

`http.response.headers`

```diff
-{ "content-type": "application/json", "x-email": "jane@example.com", "x-password": "h-pw", "x-orderid": "42" }
+{ "content-type": "application/json", "x-email": "****", "x-password": "h-pw", "x-orderid": "42" }
```

`all` works the same way as for request headers.

## `DASH0_MASK_QUERY_PARAMS`

Sets the patterns for `http.request.query`, and for messages sent to Kafka and RabbitMQ.

> **Node.js** ✅ · **Python** ❌ · **Java** ❌

Python doesn't mask query strings at all.

```bash
DASH0_MASK_QUERY_PARAMS='[".*email.*"]'
```

`http.request.query`

```diff
-{ "email": "jane@example.com", "password": "q-pw", "orderId": "42" }
+{ "email": "****", "password": "q-pw", "orderId": "42" }
```

## Combining variables

A per-payload variable wins over `DASH0_MASK_RULES` for its own payloads. `DASH0_MASK_RULES` applies to the rest.

> **Node.js** ✅ · **Python** ⚠️ · **Java** ❌

Python follows the same order for the payloads it masks.

```bash
DASH0_MASK_RULES='[".*orderId.*"]'
DASH0_MASK_REQUEST_BODY='[".*email.*"]'
```

`http.request.body`: uses `DASH0_MASK_REQUEST_BODY`

```diff
-{ "email": "jane@example.com", "password": "b-pw", "apiToken": "t", "orderId": "42" }
+{ "email": "****", "password": "b-pw", "apiToken": "t", "orderId": "42" }
```

`http.response.body`: uses `DASH0_MASK_RULES`

```diff
-{ "email": "jane@example.com", "password": "r-pw", "sessionToken": "s", "orderId": "42" }
+{ "email": "jane@example.com", "password": "r-pw", "sessionToken": "s", "orderId": "****" }
```

`http.request.headers`: uses `DASH0_MASK_RULES`

```diff
-{ "content-type": "application/json", "x-email": "jane@example.com", "x-password": "h-pw", "authorization": "Bearer abc", "x-signature": "sig", "x-orderid": "42" }
+{ "content-type": "application/json", "x-email": "jane@example.com", "x-password": "h-pw", "authorization": "Bearer abc", "x-signature": "sig", "x-orderid": "****" }
```

## `DASH0_MASK_ENV_VARS`

Sets the patterns for the environment variables the extension records as `process.environment_variable.<NAME>` resource attributes. It replaces `DASH0_MASK_RULES` for environment variables only. If it is unset or invalid, `DASH0_MASK_RULES` (or the defaults) applies.

> **Node.js** ✅ · **Python** ✅ · **Java** ✅

The extension records every environment variable of the function, including the AWS credentials Lambda provides. Keep `.*key.*`, `.*secret.*` and `.*token.*` in your list, or `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` and `AWS_SESSION_TOKEN` are sent unmasked.

```bash
DASH0_MASK_ENV_VARS='[".*pass.*",".*key.*",".*secret.*",".*credential.*",".*passphrase.*",".*token.*",".*signature.*",".*authorization.*",".*DATABASE_URL.*"]'
```

Resource attributes, with function environment `DATABASE_URL=postgres://app:pw@db/orders`, `DB_PASSWORD=hunter2`, `GITHUB_TOKEN=ghp_123`, `LOG_LEVEL=info`:

```diff
-process.environment_variable.DATABASE_URL = "postgres://app:pw@db/orders"
-process.environment_variable.DB_PASSWORD  = "hunter2"
-process.environment_variable.GITHUB_TOKEN = "ghp_123"
+process.environment_variable.DATABASE_URL = "****"
+process.environment_variable.DB_PASSWORD  = "****"
+process.environment_variable.GITHUB_TOKEN = "****"
 process.environment_variable.LOG_LEVEL    = "info"
```

Without `DASH0_MASK_ENV_VARS`, the defaults would mask `DB_PASSWORD` and `GITHUB_TOKEN`, but `DATABASE_URL` would stay visible.

## Lambda event and return value

The extension masks the event your handler receives and the value it returns before writing them as `dash0_payload` log records. It uses `DASH0_MASK_RULES` or the defaults. The per-payload variables don't apply here.

> **Node.js** ✅ · **Python** ✅ · **Java** ✅

```bash
# no DASH0_MASK_* variables
```

`dash0_payload` log record, type `lambda_event`

```diff
 {
   "headers": {
-    "Authorization": "Bearer eyJhbGciOi…",
+    "Authorization": "****",
     "x-request-id": "7f3c"
   },
   "body": "{\"email\":\"jane@example.com\",\"password\":\"hunter2\"}"
 }
```

The API Gateway `body` is a string, so the extension doesn't look inside it, and the password in it is not masked. Add `"^body$"` to `DASH0_MASK_RULES` if the event body can contain secrets. The same applies to `rawQueryString`.

Keys in the masked event are written in alphabetical order.

> [!WARNING]
> Headers and the query string captured with `DASH0_API_GATEWAY_REQUEST_HEADERS_TO_CAPTURE` and `DASH0_CAPTURE_API_GATEWAY_QUERY_STRING` are read from the event before masking, so they are not masked.

## Invalid values

A value that is not a JSON array of strings or `all` is ignored. The function logs show a warning, and the next rule in line applies.

> **Node.js** ✅ · **Python** ✅ · **Java** ✅

The check runs in the wrapper, before the runtime starts, so the warning and the fallback are the same in every runtime.

```bash
DASH0_MASK_RULES='.*email.*'   # missing the [ ] and quotes
```

Function log:

```text
[dash0] Ignoring DASH0_MASK_RULES: expected a JSON array of regular expressions or "all"
```

`http.request.body`: the defaults apply

```diff
-{ "email": "jane@example.com", "password": "b-pw", "apiToken": "t", "orderId": "42" }
+{ "email": "jane@example.com", "password": "****", "apiToken": "****", "orderId": "42" }
```

The wrapper only checks the shape of the value. A value that looks like an array but isn't valid JSON, such as `[".*email.*",]`, gets past that check, and then:

- **Node.js** uses the masking library's own built-in patterns for that payload, even when `DASH0_MASK_RULES` is set.
- **Python** stops masking if the value is in `DASH0_MASK_RULES`. In a per-payload variable, it falls back to `DASH0_MASK_RULES` or the defaults.
- **The extension** uses the defaults.

In the extension, a pattern that isn't a valid regular expression is dropped without a warning, and `[]` turns masking off.

## Payloads that are strings

Patterns are checked against keys, so they never see inside a string. Several payloads are strings, or contain one:

| Payload | Attribute | What works | Node.js | Python | Java |
|---|---|---|---|---|---|
| SQS message body | `MessageBody` and `Body` keys inside `messaging.publish.body` and `messaging.consume.body` | A pattern for the key, or `all` | ✅ | ❌ | ❌ |
| API Gateway request body | `body` key inside the Lambda event | A pattern for the key | ✅ | ✅ | ✅ |
| Kafka and RabbitMQ messages | the whole `messaging.produce.body`, `messaging.publish.body` or `messaging.consume.body` | `all` only | ✅ | ❌ | ❌ |
| Redis commands and replies | the whole `db.statement` or `db.response.body` | `all` only ³ | ✅ | ❌ | ❌ |

³ Replies from `redis` that are objects, such as `HGETALL`, are masked by key. `ioredis` writes object replies as `"[object Object]"`.

### SQS messages

> **Node.js** ✅ · **Python** ❌ · **Java** ❌

Python captures `SendMessage` as the bare `MessageBody` string and doesn't capture receives.

The SQS attributes are JSON objects, so you can mask the key that holds the message:

```bash
DASH0_MASK_RULES='[".*pass.*",".*key.*",".*secret.*",".*credential.*",".*passphrase.*",".*token.*",".*signature.*",".*authorization.*",".*MessageBody.*","^Body$"]'
```

`messaging.publish.body`

```diff
 {
   "QueueUrl": "https://sqs.eu-central-1.amazonaws.com/123456789012/orders",
-  "MessageBody": "order 42 for jane@example.com",
+  "MessageBody": "****",
   "MessageAttributes": {
     "email": { "DataType": "String", "StringValue": "jane@example.com" },
-    "password": { "DataType": "String", "StringValue": "m-pw" },
+    "password": "****",
     "orderId": { "DataType": "String", "StringValue": "42" }
   }
 }
```

`messaging.consume.body`

```diff
 {
   "Messages": [{
     "MessageId": "m1",
-    "Body": "order 42 for jane@example.com",
+    "Body": "****",
     "MessageAttributes": {
       "email": { "DataType": "String", "StringValue": "jane@example.com" },
-      "password": { "DataType": "String", "StringValue": "m-pw" },
+      "password": "****",
       "orderId": { "DataType": "String", "StringValue": "42" }
     }
   }]
 }
```

`^Body$` matches only a key named exactly `Body`. Patterns ignore case, so it also masks any `body` key in your HTTP payloads and in the Lambda event, such as the API Gateway request body.

To hide the whole SQS payload, including the queue URL and attributes, use `DASH0_MASK_REQUEST_BODY=all` and `DASH0_MASK_RESPONSE_BODY=all` instead.

### Kafka and RabbitMQ messages

> **Node.js** ✅ · **Python** ❌ · **Java** ❌

Python and Java send Kafka and RabbitMQ messages unmasked.

The message is captured as a string. Even when it contains JSON, its keys are not checked:

```bash
DASH0_MASK_RULES='[".*email.*"]'
```

`messaging.produce.body`: not masked

```diff
 "{\"email\":\"jane@example.com\",\"orderId\":42}"
```

Use `all` for the matching payload type. Sent messages follow the query-parameter rules:

```bash
DASH0_MASK_QUERY_PARAMS=all     # messages sent to Kafka or RabbitMQ
DASH0_MASK_RESPONSE_BODY=all    # messages received from Kafka, RabbitMQ or SQS
```

`messaging.produce.body`

```diff
-"{\"email\":\"jane@example.com\",\"orderId\":42}"
+"****"
```

`DASH0_MASK_QUERY_PARAMS=all` also masks the query string of requests to Express and Fastify. `DASH0_MASK_RESPONSE_BODY=all` also masks HTTP response bodies, SQS receives and Redis replies.

### Redis commands

> **Node.js** ✅ · **Python** ❌ · **Java** ❌

Python and Java send Redis commands and replies unmasked.

The command and its arguments are captured as one string:

```bash
DASH0_MASK_RULES='[".*email.*"]'
```

`db.statement`: not masked

```diff
 "SET user:1 jane@example.com"
```

Use `all` to hide the commands:

```bash
DASH0_MASK_REQUEST_BODY=all
```

`db.statement`

```diff
-"SET user:1 jane@example.com"
+"****"
```

`DASH0_MASK_REQUEST_BODY=all` also masks HTTP request bodies and SQS sends.

## Payloads that are not masked

These attributes are captured as they are. None of the masking variables apply to them, not even `all`.

**Node.js**

| Payload | Attribute |
|---|---|
| gRPC request and response messages (`@grpc/grpc-js`) | `rpc.request.payload`, `rpc.response.payload` |
| PostgreSQL query parameters (`pg`) | `db.postgresql.values` |
| MySQL queries, with the parameter values filled in (`mysql2`) | `db.statement` |
| MongoDB commands, including the values in them (`mongodb`) | `db.statement` or `db.query.text` |

**Python**

| Payload | Attribute |
|---|---|
| Request bodies sent with `requests` or `httpx`, and received by Django | `http.request.body` |
| Headers sent with `requests` or received by Django, and Flask response headers | `http.request.headers`, `http.response.headers` |
| Query strings (Flask, FastAPI) | `http.request.query` |
| Redis, Kafka and RabbitMQ (`pika`) payloads | the library's payload attributes |
| PostgreSQL query parameters (`psycopg`) | `db.statement.parameters` |
| gRPC request and response messages | `rpc.grpc.*.payload` |

**Java**

Nothing the auto-instrumentation captures is masked. That includes:
- HTTP headers and bodies
- AWS SDK payloads
- Kafka and RabbitMQ messages
- gRPC messages
- Redis (Lettuce) commands
- JDBC results

If these can contain sensitive data, turn off auto-instrumentation for the function with `DASH0_DISABLE_AUTO_INSTRUMENTATION=true`. The extension still masks the Lambda event, the return value and environment variables. In Python, you can also turn off single instrumentations with `DASH0_DISABLE_INSTRUMENTATION=<names>`, or only gRPC payloads with `DASH0_INSTRUMENT_GRPC_PAYLOADS=false`.
