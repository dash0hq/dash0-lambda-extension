//! The extension's stdout and stderr go to CloudWatch unmasked. Fails if a `tracing` macro in
//! `src/` formats a request body, payload or attribute values; log sizes (`.len()`) instead.

use regex::Regex;
use std::path::Path;

/// Logs of Lambda control-plane messages that carry no function data, with the reason.
const ALLOWED: &[(&str, &str)] = &[
    // Extensions API event: eventType, requestId, ARN, deadline, X-Ray header.
    ("src/extension/events.rs", "Event status={} payload={}"),
    // Our own Telemetry API subscription request and its response.
    (
        "src/extension/register.rs",
        "Registering telemetry with payload={}",
    ),
    (
        "src/extension/register.rs",
        "Telemetry register uri={} status={} body={}",
    ),
];

fn rust_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// The text of a macro call starting at `open` (just after its `(`), up to the matching `)`,
/// with string literals replaced by `""` and the format string returned separately.
fn macro_args(src: &str, open: usize) -> (String, String) {
    let (mut depth, mut args, mut format, mut in_str, mut escaped) =
        (1, String::new(), None::<String>, false, false);
    let mut literal = String::new();
    for c in src[open..].chars() {
        if in_str {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_str = false;
                format.get_or_insert_with(|| literal.clone());
                args.push_str("\"\"");
                continue;
            }
            literal.push(c);
            continue;
        }
        match c {
            '"' => {
                in_str = true;
                literal.clear();
                continue;
            }
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
        args.push(c);
    }
    (args, format.unwrap_or_default())
}

#[test]
fn log_statements_never_format_payloads() {
    let call = Regex::new(r"\b(?:trace|debug|info|warn|error)!\s*\(").unwrap();
    let sizes = Regex::new(r"\b\w+\s*\.\s*(?:len|is_empty)\s*\(\s*\)").unwrap();
    let payload = Regex::new(
        r"\b(?:body|body_text|body_bytes|encoded_body|payload|masked_payload|return_payload|event_payload|record|records|attributes|attrs|from_utf8_lossy)\b",
    )
    .unwrap();
    let inline = Regex::new(r"\{([A-Za-z_]\w*)[:}]").unwrap();

    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    rust_files(&root.join("src"), &mut files);
    let mut offenders = Vec::new();
    for file in files {
        let rel = file
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let src = std::fs::read_to_string(&file).unwrap();
        for m in call.find_iter(&src) {
            let (args, format) = macro_args(&src, m.end());
            // Inline captures (`"{body}"`) count as arguments too.
            let captures: String = inline
                .captures_iter(&format)
                .map(|c| format!(" {} ", &c[1]))
                .collect();
            let args = sizes.replace_all(&args, "") + captures.as_str();
            if let Some(hit) = payload.find(&args) {
                if ALLOWED
                    .iter()
                    .any(|(f, msg)| *f == rel && format.contains(msg))
                {
                    continue;
                }
                let line = src[..m.start()].lines().count() + 1;
                offenders.push(format!("{rel}:{line}: `{}` in \"{format}\"", hit.as_str()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "log statements that format payload content (log `.len()` instead):\n{}",
        offenders.join("\n")
    );
}

#[test]
fn the_check_catches_a_logged_body() {
    let src = r#"tracing::info!("[{}] body={}", crate::log_prefix(), String::from_utf8_lossy(&body_bytes));"#;
    let open = src.find('(').unwrap() + 1;
    let (args, format) = macro_args(src, open);
    assert_eq!(format, "[{}] body={}");
    assert!(args.contains("from_utf8_lossy") && !args.contains("body={}"));
}
