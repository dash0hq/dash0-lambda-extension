//! Masks the payload attributes that the language distros capture (HTTP bodies, headers, query
//! strings, messaging and RPC payloads) on decoded OTLP structs, using the `DASH0_MASK_*`
//! variables. Runs at export time, so it never delays the function's response.
//!
//! Only uses external crates and `super::masking`, so `benchmarks/masking/masking.rs` can include it with `#[path]`.

use std::borrow::Cow;

use opentelemetry_proto::tonic::common::v1::any_value::Value;
use opentelemetry_proto::tonic::common::v1::{AnyValue, KeyValue};
use opentelemetry_proto::tonic::logs::v1::ResourceLogs;
use opentelemetry_proto::tonic::trace::v1::ResourceSpans;
use regex::Regex;

use super::masking::DEFAULT_MASKING_PATTERNS;

pub const MASKED: &str = "****";
const ENABLE_ENV: &str = "DASH0_MASK_IN_EXTENSION";

/// The rules for one payload type. Cloning shares the compiled regex.
#[derive(Clone)]
pub enum Rule {
    All,
    /// All patterns joined into one alternation: one regex gets literal prefilters that a
    /// `RegexSet` doesn't. `None` for an empty list.
    Keys(Option<Regex>),
}

impl Rule {
    /// `None` when every pattern of a non-empty list is invalid, so the caller falls back to
    /// the next variable instead of masking nothing. `[]` masks nothing.
    fn from_patterns<S: AsRef<str>>(patterns: &[S]) -> Option<Rule> {
        // Invalid patterns are skipped one by one, as in `masking::patterns_to_rules`. The
        // distros accept JS/Python syntax (look-around, backreferences) that `regex` rejects.
        let valid: Vec<String> = patterns
            .iter()
            .map(|p| format!("(?:{})", p.as_ref()))
            .filter(|p| match Regex::new(&format!("(?i){p}")) {
                Ok(_) => true,
                Err(e) => {
                    tracing::warn!("[DASH0] Ignoring masking pattern {}: {}", p, e);
                    false
                }
            })
            .collect();
        if valid.is_empty() {
            return patterns.is_empty().then_some(Rule::Keys(None));
        }
        // Patterns valid one by one can still fail joined, e.g. a repeated group name or the
        // size limit. Fall back to the next variable rather than panic.
        let joined = format!("(?i){}", valid.join("|"));
        match Regex::new(&joined) {
            Ok(re) => Some(Rule::Keys(Some(re))),
            Err(e) => {
                tracing::warn!("[DASH0] Ignoring masking patterns, combined: {}", e);
                None
            }
        }
    }

    /// `all`, or a JSON array of regexes; anything else is invalid.
    fn parse(value: &str) -> Option<Rule> {
        if value == "all" {
            return Some(Rule::All);
        }
        serde_json::from_str::<Vec<String>>(value)
            .ok()
            .and_then(|p| Rule::from_patterns(&p))
    }

    pub fn matches(&self, key: &str) -> bool {
        match self {
            Rule::All => true,
            Rule::Keys(re) => re.as_ref().is_some_and(|re| re.is_match(key)),
        }
    }

    fn is_empty(&self) -> bool {
        matches!(self, Rule::Keys(None))
    }
}

/// One rule per `DASH0_MASK_*` payload variable, each already resolved through
/// `DASH0_MASK_RULES` and the defaults.
pub struct PayloadRules {
    pub request_body: Rule,
    pub response_body: Rule,
    pub request_headers: Rule,
    pub response_headers: Rule,
    pub query_params: Rule,
    pub global: Rule,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    RequestBody,
    ResponseBody,
    RequestHeaders,
    ResponseHeaders,
    QueryParams,
}

impl PayloadRules {
    /// Resolve like `map_masking_env` in `opt/shared.sh`: the per-payload variable if valid,
    /// else `DASH0_MASK_RULES` if valid, else the defaults.
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> PayloadRules {
        // Most variables resolve to the same list (the defaults, or DASH0_MASK_RULES); compile
        // each distinct value once. Cold compile of six identical sets was 4.7 ms.
        let compiled = std::cell::RefCell::new(Vec::<(Option<String>, Rule)>::new());
        let compile = |value: Option<&str>| -> Option<Rule> {
            let key = value.map(str::to_string);
            if let Some((_, rule)) = compiled.borrow().iter().find(|(k, _)| *k == key) {
                return Some(rule.clone());
            }
            let rule = match value {
                Some(v) => Rule::parse(v)?,
                None => Rule::from_patterns(DEFAULT_MASKING_PATTERNS)
                    .expect("default patterns are valid"),
            };
            compiled.borrow_mut().push((key, rule.clone()));
            Some(rule)
        };
        let resolve = |name: &str| -> Rule {
            for var in [name, "DASH0_MASK_RULES"] {
                if let Some(value) = get(var) {
                    match compile(Some(&value)) {
                        Some(rule) => return rule,
                        None => tracing::warn!(
                            "[DASH0] Ignoring {}: expected a JSON array of regular expressions or \"all\"",
                            var
                        ),
                    }
                }
            }
            compile(None).expect("default patterns are valid")
        };
        PayloadRules {
            request_body: resolve("DASH0_MASK_REQUEST_BODY"),
            response_body: resolve("DASH0_MASK_RESPONSE_BODY"),
            request_headers: resolve("DASH0_MASK_REQUEST_HEADERS"),
            response_headers: resolve("DASH0_MASK_RESPONSE_HEADERS"),
            query_params: resolve("DASH0_MASK_QUERY_PARAMS"),
            global: resolve("DASH0_MASK_RULES"),
        }
    }

    pub fn rule(&self, kind: Kind) -> &Rule {
        match kind {
            Kind::RequestBody => &self.request_body,
            Kind::ResponseBody => &self.response_body,
            Kind::RequestHeaders => &self.request_headers,
            Kind::ResponseHeaders => &self.response_headers,
            Kind::QueryParams => &self.query_params,
        }
    }
}

static RULES: once_cell::sync::OnceCell<Option<PayloadRules>> = once_cell::sync::OnceCell::new();

/// `None` unless `DASH0_MASK_IN_EXTENSION` is truthy. Resolved once.
pub fn rules() -> Option<&'static PayloadRules> {
    RULES
        .get_or_init(|| {
            let enabled = std::env::var(ENABLE_ENV)
                .map(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "y"))
                .unwrap_or(false);
            enabled.then(|| PayloadRules::from_lookup(|k| std::env::var(k).ok()))
        })
        .as_ref()
}

/// How an attribute's value is shaped, and so how it is masked.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Shape {
    /// JSON (possibly cut off by the distro's attribute limit), form-encoded or plain text.
    Body,
    /// A URL or `path?query`; only the query string is masked.
    Url,
    /// A raw query string without the `?`.
    Query,
    /// A single header/metadata value whose name is the attribute key suffix.
    Named,
}

const NAMED_PREFIXES: &[(&str, Kind)] = &[
    ("http.request.header.", Kind::RequestHeaders),
    ("http.response.header.", Kind::ResponseHeaders),
    ("rpc.grpc.request.metadata.", Kind::RequestHeaders),
    ("rpc.grpc.response.metadata.", Kind::ResponseHeaders),
];

/// Which payload variable applies to an attribute, by key. `None` for every other attribute:
/// attribute keys are never matched against the rules themselves (`.*token.*` would hit
/// `gen_ai.usage.input_tokens`).
fn classify<'a>(
    key: &'a str,
    span_attrs: &[KeyValue],
    span_kind: i32,
) -> Option<(Kind, Shape, &'a str)> {
    let found = match key {
        // Java: the direction is only in the span kind (producer 4, consumer 5). Sends follow
        // the query-parameter rules like Node's Kafka/RabbitMQ sends, receives the response body.
        "messaging.message.payload" | "messaging.message.headers" if span_kind == 4 => {
            (Kind::QueryParams, Shape::Body)
        }
        "messaging.message.payload" | "messaging.message.headers" | "messaging.message.body"
            if span_kind == 5 =>
        {
            (Kind::ResponseBody, Shape::Body)
        }
        "messaging.message.body" if span_kind == 4 => (Kind::RequestBody, Shape::Body),
        "http.request.body"
        | "rpc.request.payload"
        | "rpc.grpc.request.payload"
        | "db.request.body" => (Kind::RequestBody, Shape::Body),
        "http.response.body"
        | "messaging.consume.body"
        | "rpc.response.payload"
        | "rpc.grpc.response.payload"
        | "db.response.body" => (Kind::ResponseBody, Shape::Body),
        "http.request.headers" => (Kind::RequestHeaders, Shape::Body),
        "http.response.headers" => (Kind::ResponseHeaders, Shape::Body),
        "http.request.query" => (Kind::QueryParams, Shape::Body),
        "url.query" => (Kind::QueryParams, Shape::Query),
        "url.full" | "http.url" | "http.target" => (Kind::QueryParams, Shape::Url),
        // The Node distro masks messages sent to Kafka and RabbitMQ with the query-parameter
        // rules (docs/masking.md, footnote 2); keep that so the documented examples hold.
        "messaging.produce.body" => (Kind::QueryParams, Shape::Body),
        "messaging.publish.body"
            if string_attr(span_attrs, "messaging.system") == Some("rabbitmq") =>
        {
            (Kind::QueryParams, Shape::Body)
        }
        "messaging.publish.body" => (Kind::RequestBody, Shape::Body),
        // Redis commands and MongoDB command JSON only: masking SQL statements with `all`
        // would hide every query.
        "db.statement" | "db.query.text"
            if matches!(db_system(span_attrs), Some("redis" | "mongodb")) =>
        {
            (Kind::RequestBody, Shape::Body)
        }
        _ => {
            return NAMED_PREFIXES.iter().find_map(|(prefix, kind)| {
                key.strip_prefix(prefix)
                    .map(|name| (*kind, Shape::Named, name))
            })
        }
    };
    Some((found.0, found.1, key))
}

fn string_attr<'a>(attrs: &'a [KeyValue], key: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|kv| kv.key == key)
        .and_then(|kv| match &kv.value {
            Some(AnyValue {
                value: Some(Value::StringValue(s)),
            }) => Some(s.as_str()),
            _ => None,
        })
}

fn db_system<'a>(attrs: &'a [KeyValue]) -> Option<&'a str> {
    string_attr(attrs, "db.system").or_else(|| string_attr(attrs, "db.system.name"))
}

/// Mask the payload attributes in place. Allocates only for values that change. `span_kind` is
/// the OTLP span kind, 0 for events and log records.
pub fn mask_attributes(attrs: &mut [KeyValue], rules: &PayloadRules, span_kind: i32) {
    // Indices first: `classify` reads sibling attributes, which a mutable walk can't.
    let mut targets: Vec<(usize, Kind, Shape)> = Vec::new();
    for (i, kv) in attrs.iter().enumerate() {
        if let Some((kind, shape, name)) = classify(&kv.key, attrs, span_kind) {
            if shape == Shape::Named {
                if rules.rule(kind).matches(name) {
                    targets.push((i, kind, shape));
                }
            } else if !rules.rule(kind).is_empty() {
                targets.push((i, kind, shape));
            }
        }
    }
    for (i, kind, shape) in targets {
        let rule = rules.rule(kind);
        if let Some(value) = attrs[i].value.as_mut() {
            mask_any_value(value, rule, shape);
        }
    }
}

fn mask_any_value(value: &mut AnyValue, rule: &Rule, shape: Shape) {
    match value.value.as_mut() {
        Some(Value::StringValue(s)) => {
            if let Cow::Owned(masked) = mask_str(s, rule, shape) {
                *s = masked;
            }
        }
        Some(Value::ArrayValue(arr)) => {
            for v in arr.values.iter_mut() {
                mask_any_value(v, rule, shape);
            }
        }
        Some(Value::KvlistValue(list)) if shape != Shape::Named => {
            if matches!(rule, Rule::All) {
                *value = masked_value();
                return;
            }
            for kv in list.values.iter_mut() {
                if rule.matches(&kv.key) {
                    kv.value = Some(masked_value());
                } else if let Some(v) = kv.value.as_mut() {
                    mask_any_value(v, rule, shape);
                }
            }
        }
        Some(Value::BytesValue(_)) | Some(Value::KvlistValue(_)) if matches!(rule, Rule::All) => {
            *value = masked_value();
        }
        _ => {}
    }
}

fn masked_value() -> AnyValue {
    AnyValue {
        value: Some(Value::StringValue(MASKED.to_string())),
    }
}

/// Mask one string value. Returns `Cow::Borrowed` when nothing matched.
pub fn mask_str<'a>(s: &'a str, rule: &Rule, shape: Shape) -> Cow<'a, str> {
    if matches!(rule, Rule::All) {
        // `all` on a URL hides the query string, not the host and path.
        return match (shape, s.find('?')) {
            (Shape::Url, Some(q)) => Cow::Owned(format!("{}{}", &s[..=q], MASKED)),
            (Shape::Url, None) => Cow::Borrowed(s),
            _ => Cow::Owned(MASKED.to_string()),
        };
    }
    match shape {
        Shape::Named => Cow::Owned(MASKED.to_string()),
        Shape::Url => match s.find('?') {
            Some(q) => match mask_query(&s[q + 1..], rule) {
                Cow::Borrowed(_) => Cow::Borrowed(s),
                Cow::Owned(masked) => Cow::Owned(format!("{}{}", &s[..=q], masked)),
            },
            None => Cow::Borrowed(s),
        },
        Shape::Query => mask_query(s, rule),
        Shape::Body => mask_body(s, rule),
    }
}

/// Mask a body by key: JSON (also when cut off), else form-encoded pairs. Plain text is left
/// alone; only `all` masks it.
pub fn mask_body<'a>(s: &'a str, rule: &Rule) -> Cow<'a, str> {
    let trimmed = s.trim_start();
    match trimmed.as_bytes().first() {
        Some(b'{') | Some(b'[')
            if trimmed.find(['"', '\'']).map(|i| trimmed.as_bytes()[i]) == Some(b'\'') =>
        {
            mask_quoted(s, rule, b'\'')
        }
        Some(b'{') | Some(b'[') => mask_json_tolerant(s, rule),
        // Node stringifies a body it could not parse, e.g. JSON cut before scrubbing, into a
        // JSON string literal: `"{\"password\":\"hun`. Unwrap it once and mask the content.
        Some(b'"') if skip_json_whitespace(&trimmed[1..]).starts_with(['{', '[']) => {
            let inner = unescape_json_string(&trimmed[1..]);
            match mask_json_tolerant(&inner, rule) {
                Cow::Borrowed(_) => Cow::Borrowed(s),
                Cow::Owned(masked) => {
                    Cow::Owned(serde_json::to_string(&masked).unwrap_or_else(|_| MASKED.into()))
                }
            }
        }
        // Node stringifies form-encoded bodies the same way: `"email=jane&password=hunter2"`.
        Some(b'"') if looks_form_encoded(trimmed) => {
            let inner = unescape_json_string(&trimmed[1..]);
            if !looks_form_encoded(&inner) {
                return Cow::Borrowed(s);
            }
            match mask_pairs(&inner, rule) {
                Cow::Borrowed(_) => Cow::Borrowed(s),
                Cow::Owned(masked) => {
                    Cow::Owned(serde_json::to_string(&masked).unwrap_or_else(|_| MASKED.into()))
                }
            }
        }
        Some(b'"') => Cow::Borrowed(s),
        _ if looks_form_encoded(s) => mask_pairs(s, rule),
        _ => Cow::Borrowed(s),
    }
}

/// Skip whitespace inside a JSON string literal, raw or escaped (`\n`, `\r`, `\t`).
fn skip_json_whitespace(mut s: &str) -> &str {
    loop {
        let t = s.trim_start_matches([' ', '\t', '\n', '\r']);
        let t = ["\\n", "\\r", "\\t"]
            .iter()
            .find_map(|e| t.strip_prefix(e))
            .unwrap_or(t);
        if t.len() == s.len() {
            return t;
        }
        s = t;
    }
}

fn looks_form_encoded(s: &str) -> bool {
    s.contains('=') && !s.bytes().any(|b| b.is_ascii_whitespace())
}

/// Content of a JSON string literal without its opening quote, up to the closing quote or the
/// end of the input (cut off). Invalid escapes are kept as they are.
fn unescape_json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => break,
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('b') => out.push('\u{8}'),
                Some('f') => out.push('\u{c}'),
                Some('u') => {
                    let hex: String = chars.by_ref().take(4).collect();
                    match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                        Some(ch) => out.push(ch),
                        None => {
                            out.push_str("\\u");
                            out.push_str(&hex);
                        }
                    }
                }
                Some(other) => out.push(other),
                None => {}
            },
            c => out.push(c),
        }
    }
    out
}

/// Mask `name=value` pairs separated by `&`, keeping a `#fragment`. Names are matched
/// percent-decoded.
pub fn mask_query<'a>(s: &'a str, rule: &Rule) -> Cow<'a, str> {
    let Some(i) = s.find('#') else {
        return mask_pairs(s, rule);
    };
    let (query, fragment) = s.split_at(i);
    match mask_pairs(query, rule) {
        Cow::Owned(mut buf) => {
            buf.push_str(fragment);
            Cow::Owned(buf)
        }
        Cow::Borrowed(_) => Cow::Borrowed(s),
    }
}

/// Mask `name=value` pairs separated by `&`. A `#` is part of the value: in a form body it
/// is not a fragment.
fn mask_pairs<'a>(query: &'a str, rule: &Rule) -> Cow<'a, str> {
    let mut out: Option<String> = None;
    let mut pos = 0;
    for pair in query.split('&') {
        let end = pos + pair.len();
        if let Some(eq) = pair.find('=') {
            let name = &pair[..eq];
            if rule.matches(&percent_decode(name)) {
                let buf = out.get_or_insert_with(|| {
                    let mut b = String::with_capacity(query.len());
                    b.push_str(&query[..pos]);
                    b
                });
                buf.push_str(name);
                buf.push('=');
                buf.push_str(MASKED);
                if end < query.len() {
                    buf.push('&');
                }
                pos = end + 1;
                continue;
            }
        }
        if let Some(buf) = out.as_mut() {
            buf.push_str(pair);
            if end < query.len() {
                buf.push('&');
            }
        }
        pos = end + 1;
    }
    match out {
        Some(buf) => Cow::Owned(buf),
        None => Cow::Borrowed(query),
    }
}

fn percent_decode(s: &str) -> Cow<'_, str> {
    if !s.contains('%') && !s.contains('+') {
        return Cow::Borrowed(s);
    }
    let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match (
            b[i],
            b.get(i + 1).copied().and_then(hex),
            b.get(i + 2).copied().and_then(hex),
        ) {
            (b'%', Some(hi), Some(lo)) => {
                out.push(hi << 4 | lo);
                i += 3;
            }
            (b'+', _, _) => {
                out.push(b' ');
                i += 1;
            }
            (c, _, _) => {
                out.push(c);
                i += 1;
            }
        }
    }
    Cow::Owned(String::from_utf8_lossy(&out).into_owned())
}

/// Key-based JSON masking on the raw text, without parsing into a tree, so it also works on
/// JSON cut off by an attribute length limit. Every object key that matches gets its whole
/// value (scalar, object or array) replaced by `"****"`; a value cut off by the end of the
/// input is masked up to the end. Keys inside string values are not looked at.
pub fn mask_json_tolerant<'a>(s: &'a str, rule: &Rule) -> Cow<'a, str> {
    mask_quoted(s, rule, b'"')
}

/// The scanner behind `mask_json_tolerant`, with `q` as the string quote: `"` for JSON, `'` for
/// the Python `repr()` of a dict, which is how the Python distro writes `requests` headers.
/// shortcut: a repr string quoted with the other quote (`"it's"`) can hide later keys; parse
/// both quote styles if that shows up in practice.
fn mask_quoted<'a>(s: &'a str, rule: &Rule, q: u8) -> Cow<'a, str> {
    let b = s.as_bytes();
    let mut out: Option<String> = None;
    let mut copied = 0; // bytes of `s` already copied into `out`
                        // Keys repeat across array items; remember the last few verdicts instead of re-running
                        // the regex (half the masking time in the profile).
    let mut seen: [(&str, bool); 8] = [("", false); 8];
    let mut seen_len = 0;
    let mut i = 0;
    while i < b.len() {
        if b[i] != q {
            i += 1;
            continue;
        }
        let str_start = i;
        let str_end = match string_end(b, i, q) {
            Some(e) => e, // index of the closing quote
            None => break,
        };
        i = str_end + 1;
        // A key is a string followed by `:`.
        let mut j = i;
        while j < b.len() && b[j].is_ascii_whitespace() {
            j += 1;
        }
        if j >= b.len() || b[j] != b':' {
            continue;
        }
        let raw_key = &s[str_start + 1..str_end];
        let key: Cow<str> = if q == b'"' && raw_key.contains('\\') {
            serde_json::from_str::<String>(&s[str_start..=str_end])
                .map(Cow::Owned)
                .unwrap_or(Cow::Borrowed(raw_key))
        } else {
            Cow::Borrowed(raw_key)
        };
        let hit = match seen[..seen_len].iter().find(|(k, _)| *k == key) {
            Some((_, hit)) => *hit,
            None => {
                let hit = rule.matches(&key);
                // Only unescaped keys borrow from `s` and can be kept.
                if let (Cow::Borrowed(k), true) = (&key, seen_len < seen.len()) {
                    seen[seen_len] = (k, hit);
                    seen_len += 1;
                }
                hit
            }
        };
        if !hit {
            i = j + 1;
            continue;
        }
        let mut v = j + 1;
        while v < b.len() && b[v].is_ascii_whitespace() {
            v += 1;
        }
        let v_end = value_end(b, v, q);
        let quote = q as char;
        if s[v..v_end]
            .strip_prefix(quote)
            .and_then(|r| r.strip_suffix(quote))
            == Some(MASKED)
        {
            i = v_end;
            continue;
        }
        let buf = out.get_or_insert_with(|| String::with_capacity(s.len()));
        buf.push_str(&s[copied..v]);
        buf.push(quote);
        buf.push_str(MASKED);
        buf.push(quote);
        copied = v_end;
        i = v_end;
    }
    match out {
        Some(mut buf) => {
            buf.push_str(&s[copied..]);
            Cow::Owned(buf)
        }
        None => Cow::Borrowed(s),
    }
}

/// Index of the quote that closes the string opening at `start`, or `None` if cut off.
fn string_end(b: &[u8], start: usize, q: u8) -> Option<usize> {
    let mut i = start + 1;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            c if c == q => return Some(i),
            _ => i += 1,
        }
    }
    None
}

/// End (exclusive) of the JSON value starting at `start`; the input length if cut off.
fn value_end(b: &[u8], start: usize, q: u8) -> usize {
    match b.get(start) {
        Some(&c) if c == q => string_end(b, start, q).map_or(b.len(), |e| e + 1),
        Some(b'{') | Some(b'[') => {
            let mut depth = 0usize;
            let mut i = start;
            while i < b.len() {
                match b[i] {
                    c if c == q => match string_end(b, i, q) {
                        Some(e) => i = e,
                        None => return b.len(),
                    },
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => {
                        depth -= 1;
                        if depth == 0 {
                            return i + 1;
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
            b.len()
        }
        _ => {
            let mut i = start;
            while i < b.len() && !matches!(b[i], b',' | b'}' | b']') {
                i += 1;
            }
            // Keep whitespace before the delimiter out of the masked range.
            while i > start && b[i - 1].is_ascii_whitespace() {
                i -= 1;
            }
            i
        }
    }
}

/// Mask the `message` of a `dash0_payload` log body rendered by `build_payload_body`
/// (`{"message":…,"name":"dash0_payload","type":…}`), leaving the wrapper keys alone.
pub fn mask_payload_log_body<'a>(body: &'a str, rule: &Rule) -> Cow<'a, str> {
    const PREFIX: &str = "{\"message\":";
    let Some(rest) = body.strip_prefix(PREFIX) else {
        return Cow::Borrowed(body);
    };
    let end = value_end(rest.as_bytes(), 0, b'"');
    let message = &rest[..end];
    let masked = if matches!(rule, Rule::All) {
        Cow::Owned(format!("\"{}\"", MASKED))
    } else if message.starts_with('"') {
        // A body that was not valid JSON is stored as a string; mask its content.
        let inner = unescape_json_string(&message[1..]);
        match mask_body(&inner, rule) {
            Cow::Borrowed(_) => Cow::Borrowed(message),
            Cow::Owned(m) => Cow::Owned(serde_json::to_string(&m).unwrap_or_default()),
        }
    } else {
        mask_json_tolerant(message, rule)
    };
    match masked {
        Cow::Borrowed(_) => Cow::Borrowed(body),
        Cow::Owned(m) => Cow::Owned(format!("{}{}{}", PREFIX, m, &rest[end..])),
    }
}

pub fn mask_resource_spans(resource_spans: &mut [ResourceSpans], rules: &PayloadRules) {
    for rs in resource_spans {
        for ss in rs.scope_spans.iter_mut() {
            for span in ss.spans.iter_mut() {
                mask_attributes(&mut span.attributes, rules, span.kind);
                for event in span.events.iter_mut() {
                    mask_attributes(&mut event.attributes, rules, 0);
                }
            }
        }
    }
}

/// OTLP log records: payload attributes as on spans, plus the body by key with
/// `DASH0_MASK_RULES` when it is a map or JSON text.
pub fn mask_resource_logs(resource_logs: &mut [ResourceLogs], rules: &PayloadRules) {
    for rl in resource_logs {
        for sl in rl.scope_logs.iter_mut() {
            for record in sl.log_records.iter_mut() {
                mask_attributes(&mut record.attributes, rules, 0);
                if let Some(body) = record.body.as_mut() {
                    if !rules.global.is_empty() && !matches!(rules.global, Rule::All) {
                        mask_any_value(body, &rules.global, Shape::Body);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "payload_masking_inventory_tests.rs"]
mod inventory_tests;

#[cfg(test)]
#[path = "payload_masking_docs_tests.rs"]
mod docs_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry_proto::tonic::common::v1::{ArrayValue, KeyValueList};
    use opentelemetry_proto::tonic::logs::v1::{LogRecord, ScopeLogs};
    use opentelemetry_proto::tonic::trace::v1::{span::Event, ScopeSpans, Span};

    pub(super) const SECRET: &str = "S3CR3T";

    pub(super) fn rules(vars: &[(&str, &str)]) -> PayloadRules {
        PayloadRules::from_lookup(|k| {
            vars.iter()
                .find(|(name, _)| *name == k)
                .map(|(_, v)| v.to_string())
        })
    }

    pub(super) fn kv(key: &str, value: &str) -> KeyValue {
        KeyValue {
            key: key.to_string(),
            value: Some(AnyValue {
                value: Some(Value::StringValue(value.to_string())),
            }),
        }
    }

    pub(super) fn get<'a>(attrs: &'a [KeyValue], key: &str) -> &'a str {
        string_attr(attrs, key).unwrap()
    }

    pub(super) fn masked(attrs: Vec<KeyValue>, rules: &PayloadRules, key: &str) -> String {
        let mut attrs = attrs;
        mask_attributes(&mut attrs, rules, 0);
        get(&attrs, key).to_string()
    }

    /// A value of the given shape with a secret under `password` and personal data under `email`.
    pub(super) fn sample(shape: &str) -> String {
        match shape {
            "json" => format!(r#"{{"user":"bob","password":"{SECRET}","email":"jane@example.com"}}"#),
            "url" => format!("https://example.com/orders?password={SECRET}&email=jane%40example.com&user=bob#top"),
            "target" => format!("/orders?password={SECRET}&email=jane%40example.com&user=bob"),
            "query" => format!("password={SECRET}&email=jane%40example.com&user=bob"),
            "named" => format!("Bearer {SECRET}"),
            "redis" => format!("SET session {SECRET}"),
            other => panic!("unknown shape {other}"),
        }
    }

    #[test]
    fn other_variables_do_not_leak_into_a_payload_type() {
        let r = rules(&[("DASH0_MASK_RESPONSE_BODY", "all")]);
        assert_eq!(
            masked(
                vec![kv("http.request.body", &sample("json"))],
                &r,
                "http.request.body"
            ),
            sample("json").replace(SECRET, MASKED)
        );
    }

    #[test]
    fn invalid_variable_falls_back_to_mask_rules_then_defaults() {
        let r = rules(&[
            ("DASH0_MASK_REQUEST_BODY", "not json"),
            ("DASH0_MASK_RULES", r#"[".*email.*"]"#),
        ]);
        let out = masked(
            vec![kv("http.request.body", &sample("json"))],
            &r,
            "http.request.body",
        );
        assert!(!out.contains("jane") && out.contains(SECRET), "{out}");

        let r = rules(&[("DASH0_MASK_REQUEST_BODY", "[null]")]);
        let out = masked(
            vec![kv("http.request.body", &sample("json"))],
            &r,
            "http.request.body",
        );
        assert!(!out.contains(SECRET), "{out}");
    }

    #[test]
    fn patterns_that_only_fail_once_joined_fall_back_instead_of_panicking() {
        // Each compiles alone; joined, the group name repeats.
        let r = rules(&[
            (
                "DASH0_MASK_REQUEST_BODY",
                r#"["(?P<k>email)", "(?P<k>phone)"]"#,
            ),
            ("DASH0_MASK_RULES", r#"[".*email.*"]"#),
        ]);
        let out = masked(
            vec![kv("http.request.body", &sample("json"))],
            &r,
            "http.request.body",
        );
        assert!(!out.contains("jane") && out.contains(SECRET), "{out}");
    }

    #[test]
    fn attribute_keys_are_never_matched() {
        let attrs = vec![
            kv("gen_ai.usage.input_tokens", "12"),
            kv("messaging.kafka.message.key", "order-42"),
            kv("http.request.header.x-request-id", "abc"),
            kv("db.statement", "SELECT password FROM users"),
        ];
        let mut out = attrs.clone();
        mask_attributes(&mut out, &rules(&[("DASH0_MASK_REQUEST_BODY", "all")]), 0);
        assert_eq!(out, attrs);
    }

    #[test]
    fn truncated_json_masks_complete_and_cut_values() {
        let r = rules(&[]);
        let cut = r#"{"user":"bob","apiKey":"abc","nested":{"password":"hun"#;
        assert_eq!(
            mask_body(cut, &r.request_body),
            r#"{"user":"bob","apiKey":"****","nested":{"password":"****""#
        );
        let cut_in_object = r#"{"secrets":{"a":1,"b":[1,2"#;
        assert_eq!(
            mask_body(cut_in_object, &r.request_body),
            r#"{"secrets":"****""#
        );
        // The example in docs/masking.md, "Masking in the extension".
        assert_eq!(
            mask_body(
                r#"{"email":"jane","sessionToken":"XQZSECRET"#,
                &r.response_body
            ),
            r#"{"email":"jane","sessionToken":"****""#
        );
        let cut_in_key = r#"{"user":"bob","passw"#;
        assert_eq!(mask_body(cut_in_key, &r.request_body), cut_in_key);
    }

    #[test]
    fn json_masks_objects_arrays_scalars_and_escaped_keys() {
        let r = rules(&[]);
        let input = r#"[{"token": {"a": "b"}, "keys": [1, "x"], "pin": 1234, "password": "p", "note": "\"password\": x"}]"#;
        assert_eq!(
            mask_body(input, &r.request_body),
            r#"[{"token": "****", "keys": "****", "pin": 1234, "password": "****", "note": "\"password\": x"}]"#
        );
        assert_eq!(
            mask_body(r#"{"apiKey" : 42 , "x":1}"#, &r.request_body),
            r#"{"apiKey" : "****" , "x":1}"#
        );
    }

    #[test]
    fn python_repr_dicts_are_masked_by_key() {
        // `requests` headers as the Python distro writes them (from an integration test run).
        let r = rules(&[]);
        let headers = "{'User-Agent': 'python-requests/2.34.2', 'Accept': '*/*', 'x-api-key': 'XQZ-IT-SECRET', 'Content-Length': '58'}";
        assert_eq!(
            mask_body(headers, &r.request_headers),
            "{'User-Agent': 'python-requests/2.34.2', 'Accept': '*/*', 'x-api-key': '****', 'Content-Length': '58'}"
        );
        // Values that aren't strings, and JSON that merely contains a `'`, keep their own path.
        assert_eq!(
            mask_body("{'password': None, 'n': 1}", &r.request_body),
            "{'password': '****', 'n': 1}"
        );
        assert_eq!(
            mask_body(r#"{"note":"it's","password":"p"}"#, &r.request_body),
            r#"{"note":"it's","password":"****"}"#
        );
    }

    #[test]
    fn form_encoded_and_plain_bodies() {
        let r = rules(&[]);
        assert_eq!(
            mask_body("user=bob&pass%77ord=hunter2&api_key=x", &r.request_body),
            "user=bob&pass%77ord=****&api_key=****"
        );
        assert_eq!(
            mask_body("order 42 password hunter2", &r.request_body),
            "order 42 password hunter2"
        );
        // An unencoded `#` in a form body is part of a value, not a fragment.
        assert_eq!(
            mask_body("note=a#b&password=x", &r.request_body),
            "note=a#b&password=****"
        );
    }

    #[test]
    fn urls_keep_everything_but_matching_query_values() {
        let r = rules(&[]);
        let rule = &r.query_params;
        assert_eq!(
            mask_str("https://u:p@h/a?token=t&x=1#f", rule, Shape::Url),
            "https://u:p@h/a?token=****&x=1#f"
        );
        assert_eq!(mask_str("https://h/a", rule, Shape::Url), "https://h/a");
        assert_eq!(
            mask_str("/a?flag&key=", rule, Shape::Url),
            "/a?flag&key=****"
        );
    }

    #[test]
    fn nothing_matching_borrows() {
        let r = rules(&[]);
        assert!(matches!(
            mask_body(r#"{"user":"bob","items":[1,2,3]}"#, &r.request_body),
            Cow::Borrowed(_)
        ));
        assert!(matches!(
            mask_query("a=1&b=2", &r.query_params),
            Cow::Borrowed(_)
        ));
        assert!(matches!(
            mask_str("https://h/a?a=1", &r.query_params, Shape::Url),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn header_arrays_and_kvlists() {
        let r = rules(&[]);
        let mut attrs = vec![
            KeyValue {
                key: "http.request.header.authorization".to_string(),
                value: Some(AnyValue {
                    value: Some(Value::ArrayValue(ArrayValue {
                        values: vec![AnyValue {
                            value: Some(Value::StringValue("Bearer x".to_string())),
                        }],
                    })),
                }),
            },
            KeyValue {
                key: "http.request.headers".to_string(),
                value: Some(AnyValue {
                    value: Some(Value::KvlistValue(KeyValueList {
                        values: vec![kv("authorization", "Bearer x"), kv("accept", "*/*")],
                    })),
                }),
            },
        ];
        mask_attributes(&mut attrs, &r, 0);
        let Some(Value::ArrayValue(arr)) = attrs[0].value.as_ref().unwrap().value.as_ref() else {
            panic!()
        };
        assert_eq!(arr.values[0], masked_value());
        let Some(Value::KvlistValue(list)) = attrs[1].value.as_ref().unwrap().value.as_ref() else {
            panic!()
        };
        assert_eq!(get(&list.values, "authorization"), MASKED);
        assert_eq!(get(&list.values, "accept"), "*/*");
    }

    #[test]
    fn rabbitmq_publish_uses_query_param_rules() {
        let r = rules(&[("DASH0_MASK_QUERY_PARAMS", "all")]);
        let attrs = vec![
            kv("messaging.publish.body", "hello"),
            kv("messaging.system", "rabbitmq"),
        ];
        assert_eq!(masked(attrs, &r, "messaging.publish.body"), MASKED);
        let sqs = vec![
            kv("messaging.publish.body", "hello"),
            kv("messaging.system", "aws_sqs"),
        ];
        assert_eq!(masked(sqs, &r, "messaging.publish.body"), "hello");
    }

    #[test]
    fn resource_spans_and_events_are_masked() {
        let mut spans = vec![ResourceSpans {
            scope_spans: vec![ScopeSpans {
                spans: vec![Span {
                    attributes: vec![kv("url.full", "https://h/?api_key=k")],
                    events: vec![Event {
                        attributes: vec![kv("http.request.body", r#"{"secret":1}"#)],
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }];
        mask_resource_spans(&mut spans, &rules(&[]));
        let span = &spans[0].scope_spans[0].spans[0];
        assert_eq!(get(&span.attributes, "url.full"), "https://h/?api_key=****");
        assert_eq!(
            get(&span.events[0].attributes, "http.request.body"),
            r#"{"secret":"****"}"#
        );
    }

    #[test]
    fn log_bodies_and_attributes_are_masked() {
        let mut logs = vec![ResourceLogs {
            scope_logs: vec![ScopeLogs {
                log_records: vec![
                    LogRecord {
                        body: Some(AnyValue {
                            value: Some(Value::StringValue(
                                r#"{"msg":"login","password":"p"}"#.to_string(),
                            )),
                        }),
                        attributes: vec![
                            kv("http.request.header.cookie", "c"),
                            kv("http.request.header.authorization", "a"),
                        ],
                        ..Default::default()
                    },
                    LogRecord {
                        body: Some(AnyValue {
                            value: Some(Value::KvlistValue(KeyValueList {
                                values: vec![kv("token", "t"), kv("msg", "hi")],
                            })),
                        }),
                        ..Default::default()
                    },
                    LogRecord {
                        body: Some(AnyValue {
                            value: Some(Value::StringValue(
                                "user logged in with password hunter2".to_string(),
                            )),
                        }),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        }];
        mask_resource_logs(&mut logs, &rules(&[]));
        let records = &logs[0].scope_logs[0].log_records;
        let body = |i: usize| records[i].body.clone().unwrap().value.unwrap();
        assert_eq!(
            body(0),
            Value::StringValue(r#"{"msg":"login","password":"****"}"#.to_string())
        );
        assert_eq!(
            get(&records[0].attributes, "http.request.header.cookie"),
            "c"
        );
        assert_eq!(
            get(&records[0].attributes, "http.request.header.authorization"),
            MASKED
        );
        let Value::KvlistValue(list) = body(1) else {
            panic!()
        };
        assert_eq!(get(&list.values, "token"), MASKED);
        // Plain text is not parsed; key-based masking can't see secrets in prose.
        assert_eq!(
            body(2),
            Value::StringValue("user logged in with password hunter2".to_string())
        );
    }

    #[test]
    fn every_cut_of_a_body_hides_the_secret() {
        let r = rules(&[]);
        let body = r#"{"user":"bob","nested":{"apiKey":"XQZSECRET","list":[{"password":"XQZSECRET"}]},"token":"XQZSECRET"}"#;
        for cut in 1..=body.len() {
            let input = &body[..cut];
            let out = mask_body(input, &r.request_body);
            assert!(!out.contains("XQ"), "cut at {cut}: {input} -> {out}");
            // Node wraps bodies it could not parse in a JSON string literal.
            let literal = serde_json::to_string(input).unwrap();
            let literal = &literal[..literal.len() - 1]; // cut-off literals have no closing quote
            let out = mask_body(literal, &r.request_body);
            assert!(
                !out.contains("XQ"),
                "literal cut at {cut}: {literal} -> {out}"
            );
        }
    }

    #[test]
    fn json_string_literal_bodies_are_unwrapped_once() {
        let r = rules(&[]);
        let literal = serde_json::to_string(r#"{"password":"p","id":1}"#).unwrap();
        let out = mask_body(&literal, &r.request_body);
        let inner: String = serde_json::from_str(&out).unwrap();
        assert_eq!(inner, r#"{"password":"****","id":1}"#);
        // Whitespace before the object, raw or escaped, doesn't hide it.
        for body in [" {\"password\":\"p\"}", "\n\t{\"password\":\"p\"}"] {
            let literal = serde_json::to_string(body).unwrap();
            let out = mask_body(&literal, &r.request_body);
            let inner: String = serde_json::from_str(&out).unwrap();
            assert!(inner.contains(r#""password":"****""#), "{inner}");
        }
        // Node writes form bodies as a string literal: masked by field name, still a literal.
        assert_eq!(
            mask_body(r#""email=jane&password=x""#, &r.request_body),
            r#""email=jane&password=****""#
        );
        // Any other string literal stays as it is: strings are not parsed.
        assert_eq!(
            mask_body(r#""order 42 password=x""#, &r.request_body),
            r#""order 42 password=x""#
        );
    }

    #[test]
    fn escaped_keys_are_decoded_before_matching() {
        let r = rules(&[]);
        assert_eq!(
            mask_body(r#"{"password":"p"}"#, &r.request_body),
            r#"{"password":"****"}"#
        );
    }

    #[test]
    fn list_of_only_invalid_patterns_falls_back() {
        let r = rules(&[("DASH0_MASK_REQUEST_BODY", r#"["(?<!x-)email"]"#)]);
        let out = masked(
            vec![kv("http.request.body", &sample("json"))],
            &r,
            "http.request.body",
        );
        assert!(!out.contains(SECRET), "defaults should apply: {out}");
    }

    #[test]
    fn url_all_hides_only_the_query() {
        let r = rules(&[("DASH0_MASK_QUERY_PARAMS", "all")]);
        assert_eq!(
            masked(vec![kv("url.full", "https://h/a?x=1")], &r, "url.full"),
            "https://h/a?****"
        );
        assert_eq!(
            masked(vec![kv("url.full", "https://h/a")], &r, "url.full"),
            "https://h/a"
        );
    }

    #[test]
    fn java_messaging_payload_direction_comes_from_span_kind() {
        let r = rules(&[("DASH0_MASK_QUERY_PARAMS", "all")]);
        let attrs = || vec![kv("messaging.message.payload", "hello")];
        let mut producer = attrs();
        mask_attributes(&mut producer, &r, 4);
        assert_eq!(get(&producer, "messaging.message.payload"), MASKED);
        let mut consumer = attrs();
        mask_attributes(&mut consumer, &r, 5);
        assert_eq!(get(&consumer, "messaging.message.payload"), "hello");
        let mut unknown = attrs();
        mask_attributes(&mut unknown, &r, 0);
        assert_eq!(get(&unknown, "messaging.message.payload"), "hello");
    }

    #[test]
    fn payload_log_bodies_mask_only_the_message() {
        let r = rules(&[("DASH0_MASK_REQUEST_BODY", r#"[".*name.*", ".*password.*"]"#)]);
        let body = r#"{"message":{"username":"bob","password":"p"},"name":"dash0_payload","type":"http_request_body"}"#;
        assert_eq!(
            mask_payload_log_body(body, &r.request_body),
            r#"{"message":{"username":"****","password":"****"},"name":"dash0_payload","type":"http_request_body"}"#
        );
        let cut = r#"{"message":"{\"password\":\"hun","name":"dash0_payload","type":"http_request_body"}"#;
        assert_eq!(
            mask_payload_log_body(cut, &r.request_body),
            r#"{"message":"{\"password\":\"****\"","name":"dash0_payload","type":"http_request_body"}"#
        );
        assert_eq!(
            mask_payload_log_body(body, &Rule::All),
            r#"{"message":"****","name":"dash0_payload","type":"http_request_body"}"#
        );
    }
}
