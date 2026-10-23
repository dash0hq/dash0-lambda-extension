//! Every example in docs/masking.md, run through the extension with DASH0_MASK_IN_EXTENSION on:
//! the docs' payloads as input, the docs' result as the expected output. The docs show them
//! pretty-printed; attribute values are compact JSON. Traceability:
//! docs/design/central-masking-test-matrix.md.

use super::tests::{kv, masked, rules};
use super::*;

const REQUEST_BODY: &str =
    r#"{"email":"jane@example.com","password":"b-pw","apiToken":"t","orderId":"42"}"#;
const RESPONSE_BODY: &str =
    r#"{"email":"jane@example.com","password":"r-pw","sessionToken":"s","orderId":"42"}"#;
const REQUEST_HEADERS: &str = r#"{"content-type":"application/json","x-email":"jane@example.com","x-password":"h-pw","authorization":"Bearer abc","x-signature":"sig","x-orderid":"42"}"#;
const RESPONSE_HEADERS: &str = r#"{"content-type":"application/json","x-email":"jane@example.com","x-password":"h-pw","x-orderid":"42"}"#;
const QUERY: &str = r#"{"email":"jane@example.com","password":"q-pw","orderId":"42"}"#;
const ATTRIBUTES: &str = r#"{"email":{"DataType":"String","StringValue":"jane@example.com"},"password":{"DataType":"String","StringValue":"m-pw"},"orderId":{"DataType":"String","StringValue":"42"}}"#;

fn publish_body() -> String {
    format!(
        r#"{{"QueueUrl":"https://sqs.eu-central-1.amazonaws.com/123456789012/orders","MessageBody":"order 42 for jane@example.com","MessageAttributes":{ATTRIBUTES}}}"#
    )
}

fn consume_body() -> String {
    format!(
        r#"{{"Messages":[{{"MessageId":"m1","Body":"order 42 for jane@example.com","MessageAttributes":{ATTRIBUTES}}}]}}"#
    )
}

const EMAIL: &str = r#"[".*email.*"]"#;
const DEFAULTS_PLUS_EMAIL: &str = r#"[".*pass.*",".*key.*",".*secret.*",".*credential.*",".*passphrase.*",".*token.*",".*signature.*",".*authorization.*",".*email.*"]"#;

/// The value of `key` after the extension masked it, with `extra` attributes on the span.
fn out_with(vars: &[(&str, &str)], key: &str, value: &str, extra: &[(&str, &str)]) -> String {
    let mut attrs = vec![kv(key, value)];
    attrs.extend(extra.iter().map(|(k, v)| kv(k, v)));
    masked(attrs, &rules(vars), key)
}

fn out(vars: &[(&str, &str)], key: &str, value: &str) -> String {
    out_with(vars, key, value, &[])
}

/// `s` with each `from` replaced by `to`; panics if a `from` isn't there, so a typo can't make
/// an expectation equal to the input.
fn with(s: &str, edits: &[(&str, &str)]) -> String {
    edits.iter().fold(s.to_string(), |acc, (from, to)| {
        assert!(acc.contains(from), "{from} not in {acc}");
        acc.replacen(from, to, 1)
    })
}

const PW_ATTR: (&str, &str) = (
    r#""password":{"DataType":"String","StringValue":"m-pw"}"#,
    r#""password":"****""#,
);
const EMAIL_ATTR: (&str, &str) = (
    r#""email":{"DataType":"String","StringValue":"jane@example.com"}"#,
    r#""email":"****""#,
);

fn defaults_request_body() -> String {
    with(
        REQUEST_BODY,
        &[
            (r#""password":"b-pw""#, r#""password":"****""#),
            (r#""apiToken":"t""#, r#""apiToken":"****""#),
        ],
    )
}

fn defaults_response_body() -> String {
    with(
        RESPONSE_BODY,
        &[
            (r#""password":"r-pw""#, r#""password":"****""#),
            (r#""sessionToken":"s""#, r#""sessionToken":"****""#),
        ],
    )
}

// ## Defaults: nothing configured

#[test]
fn docs_defaults() {
    assert_eq!(
        out(&[], "http.request.headers", REQUEST_HEADERS),
        with(
            REQUEST_HEADERS,
            &[
                (r#""x-password":"h-pw""#, r#""x-password":"****""#),
                (
                    r#""authorization":"Bearer abc""#,
                    r#""authorization":"****""#
                ),
                (r#""x-signature":"sig""#, r#""x-signature":"****""#),
            ]
        )
    );
    assert_eq!(
        out(&[], "http.request.body", REQUEST_BODY),
        defaults_request_body()
    );
    assert_eq!(
        out(&[], "messaging.publish.body", &publish_body()),
        with(&publish_body(), &[PW_ATTR])
    );
    // "The query, the response headers and body, and the consume body follow the same pattern."
    assert_eq!(
        out(&[], "http.request.query", QUERY),
        with(QUERY, &[(r#""password":"q-pw""#, r#""password":"****""#)])
    );
    assert_eq!(
        out(&[], "http.response.headers", RESPONSE_HEADERS),
        with(
            RESPONSE_HEADERS,
            &[(r#""x-password":"h-pw""#, r#""x-password":"****""#)]
        )
    );
    assert_eq!(
        out(&[], "http.response.body", RESPONSE_BODY),
        defaults_response_body()
    );
    assert_eq!(
        out(&[], "messaging.consume.body", &consume_body()),
        with(&consume_body(), &[PW_ATTR])
    );
}

// ## DASH0_MASK_RULES

#[test]
fn docs_mask_rules_own_patterns() {
    let v = [("DASH0_MASK_RULES", EMAIL)];
    assert_eq!(
        out(&v, "http.request.body", REQUEST_BODY),
        with(
            REQUEST_BODY,
            &[(r#""email":"jane@example.com""#, r#""email":"****""#)]
        )
    );
    assert_eq!(
        out(&v, "http.request.headers", REQUEST_HEADERS),
        with(
            REQUEST_HEADERS,
            &[(r#""x-email":"jane@example.com""#, r#""x-email":"****""#)]
        )
    );
}

#[test]
fn docs_mask_rules_patterns_plus_defaults() {
    let v = [("DASH0_MASK_RULES", DEFAULTS_PLUS_EMAIL)];
    assert_eq!(
        out(&v, "http.request.body", REQUEST_BODY),
        with(
            &defaults_request_body(),
            &[(r#""email":"jane@example.com""#, r#""email":"****""#)]
        )
    );
    assert_eq!(
        out(&v, "messaging.consume.body", &consume_body()),
        with(&consume_body(), &[EMAIL_ATTR, PW_ATTR])
    );
}

#[test]
fn docs_mask_rules_all() {
    let v = [("DASH0_MASK_RULES", "all")];
    assert_eq!(out(&v, "http.request.body", REQUEST_BODY), MASKED);
    // "Every payload the Node.js distro masks becomes "****"".
    for (key, value) in [
        ("http.request.headers", REQUEST_HEADERS),
        ("http.response.headers", RESPONSE_HEADERS),
        ("http.response.body", RESPONSE_BODY),
        ("http.request.query", QUERY),
        ("messaging.publish.body", &publish_body()),
        ("messaging.consume.body", &consume_body()),
    ] {
        assert_eq!(out(&v, key, value), MASKED, "{key}");
    }
}

// ## DASH0_MASK_REQUEST_BODY

#[test]
fn docs_request_body_patterns() {
    let v = [("DASH0_MASK_REQUEST_BODY", EMAIL)];
    assert_eq!(
        out(&v, "http.request.body", REQUEST_BODY),
        with(
            REQUEST_BODY,
            &[(r#""email":"jane@example.com""#, r#""email":"****""#)]
        )
    );
    assert_eq!(
        out(&v, "messaging.publish.body", &publish_body()),
        with(&publish_body(), &[EMAIL_ATTR])
    );
    // "http.response.body: unchanged from the defaults"
    assert_eq!(
        out(&v, "http.response.body", RESPONSE_BODY),
        defaults_response_body()
    );
}

#[test]
fn docs_request_body_all() {
    let v = [("DASH0_MASK_REQUEST_BODY", "all")];
    assert_eq!(out(&v, "http.request.body", REQUEST_BODY), MASKED);
    // "The queue URL is masked as well."
    assert_eq!(out(&v, "messaging.publish.body", &publish_body()), MASKED);
    assert_eq!(
        out(&v, "messaging.consume.body", &consume_body()),
        with(&consume_body(), &[PW_ATTR])
    );
}

// ## DASH0_MASK_RESPONSE_BODY

#[test]
fn docs_response_body_patterns() {
    let v = [("DASH0_MASK_RESPONSE_BODY", EMAIL)];
    assert_eq!(
        out(&v, "http.response.body", RESPONSE_BODY),
        with(
            RESPONSE_BODY,
            &[(r#""email":"jane@example.com""#, r#""email":"****""#)]
        )
    );
    assert_eq!(
        out(&v, "messaging.consume.body", &consume_body()),
        with(&consume_body(), &[EMAIL_ATTR])
    );
}

#[test]
fn docs_response_body_all() {
    let v = [("DASH0_MASK_RESPONSE_BODY", "all")];
    assert_eq!(out(&v, "http.response.body", RESPONSE_BODY), MASKED);
    assert_eq!(out(&v, "messaging.consume.body", &consume_body()), MASKED);
}

// ## DASH0_MASK_REQUEST_HEADERS

#[test]
fn docs_request_headers_patterns() {
    let v = [("DASH0_MASK_REQUEST_HEADERS", EMAIL)];
    // "the authorization header is no longer masked"
    assert_eq!(
        out(&v, "http.request.headers", REQUEST_HEADERS),
        with(
            REQUEST_HEADERS,
            &[(r#""x-email":"jane@example.com""#, r#""x-email":"****""#)]
        )
    );
}

#[test]
fn docs_request_headers_all() {
    let v = [("DASH0_MASK_REQUEST_HEADERS", "all")];
    assert_eq!(out(&v, "http.request.headers", REQUEST_HEADERS), MASKED);
}

// ## DASH0_MASK_RESPONSE_HEADERS

#[test]
fn docs_response_headers() {
    let v = [("DASH0_MASK_RESPONSE_HEADERS", EMAIL)];
    assert_eq!(
        out(&v, "http.response.headers", RESPONSE_HEADERS),
        with(
            RESPONSE_HEADERS,
            &[(r#""x-email":"jane@example.com""#, r#""x-email":"****""#)]
        )
    );
    // "`all` works the same way as for request headers."
    let v = [("DASH0_MASK_RESPONSE_HEADERS", "all")];
    assert_eq!(out(&v, "http.response.headers", RESPONSE_HEADERS), MASKED);
}

// ## DASH0_MASK_QUERY_PARAMS

#[test]
fn docs_query_params() {
    let v = [("DASH0_MASK_QUERY_PARAMS", EMAIL)];
    assert_eq!(
        out(&v, "http.request.query", QUERY),
        with(
            QUERY,
            &[(r#""email":"jane@example.com""#, r#""email":"****""#)]
        )
    );
}

// ## Combining variables

#[test]
fn docs_combining_variables() {
    let v = [
        ("DASH0_MASK_RULES", r#"[".*orderId.*"]"#),
        ("DASH0_MASK_REQUEST_BODY", EMAIL),
    ];
    assert_eq!(
        out(&v, "http.request.body", REQUEST_BODY),
        with(
            REQUEST_BODY,
            &[(r#""email":"jane@example.com""#, r#""email":"****""#)]
        )
    );
    assert_eq!(
        out(&v, "http.response.body", RESPONSE_BODY),
        with(
            RESPONSE_BODY,
            &[(r#""orderId":"42""#, r#""orderId":"****""#)]
        )
    );
    assert_eq!(
        out(&v, "http.request.headers", REQUEST_HEADERS),
        with(
            REQUEST_HEADERS,
            &[(r#""x-orderid":"42""#, r#""x-orderid":"****""#)]
        )
    );
}

// ## Lambda event and return value
//
// Masked on receive by the master path (`masking::mask_json_string`, DASH0_MASK_RULES or the
// defaults), with or without DASH0_MASK_IN_EXTENSION; tests/masking_e2e.rs checks both flag
// states against the real binary.

#[test]
fn docs_lambda_event() {
    let event = r#"{"headers":{"Authorization":"Bearer eyJhbGciOi…","x-request-id":"7f3c"},"body":"{\"email\":\"jane@example.com\",\"password\":\"hunter2\"}"}"#;
    // Keys come out in alphabetical order; the string `body` is not looked into.
    assert_eq!(
        crate::otlp::masking::mask_json_string(event),
        r#"{"body":"{\"email\":\"jane@example.com\",\"password\":\"hunter2\"}","headers":{"Authorization":"****","x-request-id":"7f3c"}}"#
    );
}

// ## Invalid values

#[test]
fn docs_invalid_values_fall_back() {
    for invalid in [".*email.*", r#"[".*email.*",]"#, "[.*email.*]", "[null]"] {
        let v = [("DASH0_MASK_RULES", invalid)];
        assert_eq!(
            out(&v, "http.request.body", REQUEST_BODY),
            defaults_request_body(),
            "{invalid}"
        );
    }
    // "`[]` is a valid value, and in the extension it turns masking off."
    let v = [("DASH0_MASK_RULES", "[]")];
    assert_eq!(out(&v, "http.request.body", REQUEST_BODY), REQUEST_BODY);
}

// ## Payloads that are strings

#[test]
fn docs_sqs_messages() {
    let v = [(
        "DASH0_MASK_RULES",
        r#"[".*pass.*",".*key.*",".*secret.*",".*credential.*",".*passphrase.*",".*token.*",".*signature.*",".*authorization.*",".*MessageBody.*","^Body$"]"#,
    )];
    assert_eq!(
        out(&v, "messaging.publish.body", &publish_body()),
        with(
            &publish_body(),
            &[
                (
                    r#""MessageBody":"order 42 for jane@example.com""#,
                    r#""MessageBody":"****""#
                ),
                PW_ATTR
            ]
        )
    );
    assert_eq!(
        out(&v, "messaging.consume.body", &consume_body()),
        with(
            &consume_body(),
            &[
                (
                    r#""Body":"order 42 for jane@example.com""#,
                    r#""Body":"****""#
                ),
                PW_ATTR
            ]
        )
    );
    // Without a pattern for the key, the message text is a plain string and stays.
    assert!(out(&[], "messaging.publish.body", &publish_body())
        .contains(r#""MessageBody":"order 42 for jane@example.com""#));
}

const KAFKA_MESSAGE: &str = r#""{\"email\":\"jane@example.com\",\"orderId\":42}""#;

/// Docs: "Even when it contains JSON, its keys are not checked". The extension does unwrap a
/// body that is a JSON string literal holding JSON (how Node writes cut or non-JSON bodies),
/// so it masks by key here: a documented difference of extension masking.
#[test]
fn docs_kafka_message_with_patterns() {
    let v = [("DASH0_MASK_RULES", EMAIL)];
    assert_eq!(
        out(&v, "messaging.produce.body", KAFKA_MESSAGE),
        r#""{\"email\":\"****\",\"orderId\":42}""#
    );
    // A message that is not JSON stays as it is.
    assert_eq!(
        out(
            &v,
            "messaging.produce.body",
            "order 42 for jane@example.com"
        ),
        "order 42 for jane@example.com"
    );
}

#[test]
fn docs_kafka_and_rabbitmq_all() {
    let v = [
        ("DASH0_MASK_QUERY_PARAMS", "all"),
        ("DASH0_MASK_RESPONSE_BODY", "all"),
    ];
    assert_eq!(out(&v, "messaging.produce.body", KAFKA_MESSAGE), MASKED);
    assert_eq!(
        out_with(
            &v,
            "messaging.publish.body",
            KAFKA_MESSAGE,
            &[("messaging.system", "rabbitmq")]
        ),
        MASKED
    );
    assert_eq!(out(&v, "messaging.consume.body", KAFKA_MESSAGE), MASKED);
    // "DASH0_MASK_QUERY_PARAMS=all also masks the query string of requests"
    assert_eq!(out(&v, "http.request.query", QUERY), MASKED);
    // Sent messages don't follow the request-body rules.
    let v = [("DASH0_MASK_REQUEST_BODY", "all")];
    assert_eq!(
        out(&v, "messaging.produce.body", KAFKA_MESSAGE),
        KAFKA_MESSAGE
    );
}

#[test]
fn docs_redis_commands() {
    let redis = [("db.system", "redis")];
    let command = "SET user:1 jane@example.com";
    let v = [("DASH0_MASK_RULES", EMAIL)];
    assert_eq!(out_with(&v, "db.statement", command, &redis), command);
    let v = [("DASH0_MASK_REQUEST_BODY", "all")];
    assert_eq!(out_with(&v, "db.statement", command, &redis), MASKED);
    assert_eq!(
        out_with(&v, "db.statement", command, &[("db.system.name", "redis")]),
        MASKED
    );
    // Redis replies follow the response-body rules; object replies are masked by key.
    let v = [("DASH0_MASK_RESPONSE_BODY", EMAIL)];
    assert_eq!(
        out_with(
            &v,
            "db.response.body",
            r#"{"email":"jane@example.com","name":"Jane"}"#,
            &redis
        ),
        r#"{"email":"****","name":"Jane"}"#
    );
}

// ## Payloads that are not masked

/// Still unmasked with DASH0_MASK_IN_EXTENSION, even with `all` everywhere: known gaps, kept as
/// tests so closing one is a deliberate change.
#[test]
fn docs_payloads_that_stay_unmasked() {
    let all: Vec<(&str, &str)> = [
        "DASH0_MASK_RULES",
        "DASH0_MASK_REQUEST_BODY",
        "DASH0_MASK_RESPONSE_BODY",
        "DASH0_MASK_REQUEST_HEADERS",
        "DASH0_MASK_RESPONSE_HEADERS",
        "DASH0_MASK_QUERY_PARAMS",
    ]
    .iter()
    .map(|v| (*v, "all"))
    .collect();
    let cases: &[(&str, &str, &[(&str, &str)])] = &[
        // Node pg
        (
            "db.postgresql.values",
            r#"["jane@example.com","hunter2"]"#,
            &[],
        ),
        // Node mysql2: the SQL with the values filled in
        (
            "db.statement",
            "SELECT * FROM users WHERE password = 'hunter2'",
            &[("db.system", "mysql")],
        ),
        // Python FastAPI: docs/masking.md lists `http.request.query`, the distro writes this key
        (
            "http.request.query_string",
            "\"email=jane%40example.com&password=hunter2\"",
            &[],
        ),
        // Python psycopg
        (
            "db.statement.parameters",
            "('jane@example.com', 'hunter2')",
            &[],
        ),
    ];
    for (key, value, extra) in cases {
        assert_eq!(out_with(&all, key, value, extra), *value, "{key}");
    }
}

/// Payloads listed as not masked per runtime that the extension masks, because it goes by the
/// attribute, not by the instrumentation that wrote it.
#[test]
fn docs_payloads_the_extension_masks_in_every_runtime() {
    let body = r#"{"email":"jane@example.com","password":"hunter2"}"#;
    let masked_body = r#"{"email":"jane@example.com","password":"****"}"#;
    let cases: &[(&str, &[(&str, &str)])] = &[
        // Node @grpc/grpc-js, Python gRPC
        ("rpc.request.payload", &[]),
        ("rpc.response.payload", &[]),
        ("rpc.grpc.request.payload", &[]),
        ("rpc.grpc.response.payload", &[]),
        // Node mongodb
        ("db.statement", &[("db.system", "mongodb")]),
        ("db.query.text", &[("db.system.name", "mongodb")]),
        // Python requests, httpx and Django bodies and headers
        ("http.request.body", &[]),
        ("http.request.headers", &[]),
        ("http.response.headers", &[]),
    ];
    for (key, extra) in cases {
        assert_eq!(out_with(&[], key, body, extra), masked_body, "{key}");
    }
}

// ## Masking in the extension (experimental)

/// The examples in docs/masking.md, section "Masking in the extension (experimental)", come from
/// this test. Run `cargo test docs_extension_section_examples -- --nocapture` to print them.
#[test]
fn docs_extension_section_examples() {
    let examples: &[(&str, &str, &[(&str, &str)], &str)] = &[
        (
            "`url.full`, default patterns",
            "url.full",
            &[],
            "https://h/p?token=XQZSECRET&email=jane&page=1",
        ),
        (
            "`url.full`, `DASH0_MASK_QUERY_PARAMS=all`",
            "url.full",
            &[("DASH0_MASK_QUERY_PARAMS", "all")],
            "https://h/p?token=XQZSECRET&email=jane&page=1",
        ),
        (
            "`http.response.body` cut off by the attribute limit, default patterns",
            "http.response.body",
            &[],
            r#"{"email":"jane","sessionToken":"XQZSECRET"#,
        ),
        (
            "`http.request.body`, form-encoded, default patterns",
            "http.request.body",
            &[],
            "email=jane&password=XQZSECRET&orderId=42",
        ),
        (
            "`http.request.header.authorization`, default patterns",
            "http.request.header.authorization",
            &[],
            "Bearer XQZSECRET",
        ),
        (
            "`http.request.body`, default patterns: key order and numbers kept",
            "http.request.body",
            &[],
            r#"{"orderId":42,"total":1.50,"apiToken":"XQZSECRET","email":"jane"}"#,
        ),
    ];
    let expected = [
        "https://h/p?token=****&email=jane&page=1",
        "https://h/p?****",
        r#"{"email":"jane","sessionToken":"****""#,
        "email=jane&password=****&orderId=42",
        "****",
        r#"{"orderId":42,"total":1.50,"apiToken":"****","email":"jane"}"#,
    ];
    for ((title, key, vars, input), expected) in examples.iter().zip(expected) {
        let output = out(vars, key, input);
        println!("{title}:\n\n```diff\n-{input}\n+{output}\n```\n");
        assert_eq!(output, expected, "{title}");
    }
}
