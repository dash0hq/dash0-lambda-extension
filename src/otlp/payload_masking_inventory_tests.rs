//! Every attribute in the distro inventory (`testdata/distro_inventory.json`) has at least one
//! fixture entry in `testdata/masking_inventory.json` that says whether the extension masks it
//! (`masked`), misses it on purpose (`gap`), leaves it because it isn't sensitive
//! (`not-sensitive`), or leaves it to another masking path (`elsewhere`).

use std::collections::{BTreeMap, HashSet};

use super::tests::{kv, rules, sample, SECRET};
use super::*;

#[derive(serde::Deserialize)]
struct InventoryRow {
    runtime: String,
    instrumentation: String,
    library: String,
    attribute: String,
}

/// A fixture entry. `runtime`, `instrumentation`, `library` and `attribute` are the exact
/// values of the inventory row it covers.
#[derive(serde::Deserialize)]
struct Row {
    runtime: String,
    instrumentation: String,
    library: String,
    attribute: String,
    key: String,
    shape: String,
    #[serde(default)]
    span_kind: i32,
    #[serde(default)]
    siblings: BTreeMap<String, String>,
    expect: String,
    #[serde(default)]
    variable: String,
    #[serde(default)]
    reason: String,
}

impl Row {
    fn source(&self) -> (&str, &str, &str, &str) {
        (
            &self.runtime,
            &self.instrumentation,
            &self.library,
            &self.attribute,
        )
    }

    fn ctx(&self) -> String {
        format!("{} / {} / {}", self.runtime, self.instrumentation, self.key)
    }

    fn value(&self) -> String {
        value(&self.shape)
    }

    fn attrs(&self) -> Vec<KeyValue> {
        let mut attrs = vec![kv(&self.key, &self.value())];
        attrs.extend(self.siblings.iter().map(|(k, v)| kv(k, v)));
        attrs
    }

    /// The attribute's value after masking with `rules`, on a span of this entry's kind.
    fn masked(&self, rules: &PayloadRules) -> String {
        let mut attrs = self.attrs();
        mask_attributes(&mut attrs, rules, self.span_kind);
        super::tests::get(&attrs, &self.key).to_string()
    }
}

const BASE64_SECRET: &str = "eyJwYXNzd29yZCI6IlMzQ1IzVCJ9"; // {"password":"S3CR3T"}

/// `sample()` shapes plus the formats the distros write that the extension can't mask.
fn value(shape: &str) -> String {
    match shape {
        "text" => format!("login failed for bob: password {SECRET}"),
        "sql" => format!("SELECT * FROM users WHERE password = '{SECRET}'"),
        "params" => format!("('bob', '{SECRET}')"),
        "userinfo_url" => format!("redis://bob:{SECRET}@cache.internal:6379"),
        "protobuf_text" => format!("user: \"bob\"\npassword: \"{SECRET}\"\n"),
        "kafka_headers" => format!("authorization={:?}", SECRET.as_bytes()),
        "sqs_body" => format!(
            r#"{{"QueueUrl":"https://sqs.eu-west-1.amazonaws.com/1/orders","MessageBody":"{{\"password\":\"{SECRET}\"}}"}}"#
        ),
        "aws_tostring" => {
            format!(r#"Message(MessageId=1, Body={{"password":"{SECRET}"}}, ReceiptHandle=abc)"#)
        }
        "map_tostring" => format!("{{password={SECRET}, user=bob}}"),
        "sns_form" => format!(
            "Action=Publish&Message=%7B%22password%22%3A%22{SECRET}%22%7D&TopicArn=arn%3Aaws%3Asns%3Aeu-west-1%3A1%3Aorders"
        ),
        "base64_json" => format!(r#"{{"StreamName":"orders","Data":"{BASE64_SECRET}"}}"#),
        "xml" => format!("<Credentials><SecretAccessKey>{SECRET}</SecretAccessKey></Credentials>"),
        "prompt" => format!(
            r#"{{"messages":[{{"role":"user","content":"my password is {SECRET}"}}]}}"#
        ),
        "quoted_query" => format!(r#""password={SECRET}&user=bob""#),
        "rows" => format!(r#"[["bob","{SECRET}"]]"#),
        other => sample(other),
    }
}

/// The secret as it appears in a value of `shape`.
fn secret_in(shape: &str) -> String {
    match shape {
        "kafka_headers" => format!("{:?}", SECRET.as_bytes()),
        "base64_json" => BASE64_SECRET.to_string(),
        _ => SECRET.to_string(),
    }
}

fn distro_inventory() -> Vec<InventoryRow> {
    serde_json::from_str(include_str!("testdata/distro_inventory.json")).unwrap()
}

fn inventory() -> Vec<Row> {
    serde_json::from_str(include_str!("testdata/masking_inventory.json")).unwrap()
}

fn with_expect(expect: &str) -> Vec<Row> {
    inventory()
        .into_iter()
        .filter(|r| r.expect == expect)
        .collect()
}

#[test]
fn every_distro_attribute_has_a_fixture_entry_and_back() {
    let distro = distro_inventory();
    let fixture = inventory();
    let covered: HashSet<_> = fixture.iter().map(Row::source).collect();
    for row in &distro {
        // Rows without an attribute record libraries the distro doesn't instrument.
        if row.attribute == "(none)" {
            continue;
        }
        let source = (
            row.runtime.as_str(),
            row.instrumentation.as_str(),
            row.library.as_str(),
            row.attribute.as_str(),
        );
        assert!(
            covered.contains(&source),
            "{} / {} / {}: no entry in masking_inventory.json",
            row.runtime,
            row.instrumentation,
            row.attribute
        );
    }
    let known: HashSet<_> = distro
        .iter()
        .map(|r| {
            (
                r.runtime.as_str(),
                r.instrumentation.as_str(),
                r.library.as_str(),
                r.attribute.as_str(),
            )
        })
        .collect();
    for row in &fixture {
        assert!(
            known.contains(&row.source()),
            "{}: points at no distro_inventory.json row ({:?})",
            row.ctx(),
            row.attribute
        );
    }
}

#[test]
fn every_fixture_entry_is_well_formed() {
    for row in inventory() {
        let ctx = row.ctx();
        assert!(
            row.value().contains(&secret_in(&row.shape)),
            "{ctx}: value holds no secret"
        );
        match row.expect.as_str() {
            "masked" => assert!(
                row.variable.starts_with("DASH0_MASK_"),
                "{ctx}: no variable"
            ),
            "gap" | "not-sensitive" | "elsewhere" => {
                assert!(!row.reason.is_empty(), "{ctx}: no reason")
            }
            other => panic!("{ctx}: unknown expect {other}"),
        }
    }
}

#[test]
fn inventory_secrets_are_masked_with_defaults() {
    for row in with_expect("masked") {
        // Redis commands are plain strings: only `all` masks them.
        let vars: Vec<(&str, &str)> = if row.shape == "redis" {
            vec![(row.variable.as_str(), "all")]
        } else {
            vec![]
        };
        let out = row.masked(&rules(&vars));
        let ctx = row.ctx();
        assert!(!out.contains(SECRET), "{ctx}: secret leaked: {out}");
        if row.shape != "named" && row.shape != "redis" {
            assert!(out.contains("bob"), "{ctx}: non-secret data lost: {out}");
            assert!(out.contains("email"), "{ctx}: non-matching key lost: {out}");
        }
    }
}

#[test]
fn inventory_custom_rule_replaces_defaults_for_its_variable() {
    for row in with_expect("masked")
        .into_iter()
        .filter(|r| r.shape != "redis")
    {
        let ctx = row.ctx();
        if row.shape == "named" {
            // The header name is the key suffix; a rule that doesn't name it keeps the value.
            let r = rules(&[(row.variable.as_str(), r#"["^x-custom$"]"#)]);
            assert_eq!(row.masked(&r), sample("named"), "{ctx}");
            continue;
        }
        let r = rules(&[(row.variable.as_str(), r#"[".*email.*"]"#)]);
        let out = row.masked(&r);
        assert!(
            !out.contains("jane"),
            "{ctx}: custom rule not applied: {out}"
        );
        assert!(out.contains(SECRET), "{ctx}: defaults still applied: {out}");
    }
}

#[test]
fn inventory_variable_all_masks_whole_value() {
    for row in with_expect("masked") {
        let out = row.masked(&rules(&[(row.variable.as_str(), "all")]));
        let expected = match row.shape.as_str() {
            // `all` on a URL hides the query string only.
            "url" | "target" => {
                let v = sample(&row.shape);
                format!("{}?{MASKED}", &v[..v.find('?').unwrap()])
            }
            _ => MASKED.to_string(),
        };
        assert_eq!(out, expected, "{}", row.ctx());
    }
}

#[test]
fn inventory_variable_empty_list_masks_nothing() {
    for row in with_expect("masked") {
        let out = row.masked(&rules(&[(row.variable.as_str(), "[]")]));
        assert_eq!(out, row.value(), "{}", row.ctx());
    }
}

/// Known gaps: sensitive, but the extension leaves them with the defaults. Kept as tests so a
/// change in coverage is a deliberate decision: promote the entry to `masked` when it changes.
#[test]
fn inventory_expected_gaps_are_left_alone() {
    for row in with_expect("gap") {
        assert_eq!(
            row.masked(&rules(&[])),
            row.value(),
            "{} ({})",
            row.ctx(),
            row.reason
        );
    }
}

/// Metadata the extension must not touch, even though the test value holds the secret.
#[test]
fn inventory_not_sensitive_attributes_are_left_alone() {
    for row in with_expect("not-sensitive") {
        assert_eq!(
            row.masked(&rules(&[])),
            row.value(),
            "{} ({})",
            row.ctx(),
            row.reason
        );
    }
}

/// Masked by another path (capture-time payload logs, env vars); export-time payload masking
/// must leave them alone.
#[test]
fn inventory_masked_elsewhere_is_left_alone_here() {
    for row in with_expect("elsewhere") {
        assert_eq!(
            row.masked(&rules(&[])),
            row.value(),
            "{} ({})",
            row.ctx(),
            row.reason
        );
    }
}
