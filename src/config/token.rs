use hmac::{Hmac, Mac};
use hyper::{body, Body, Client, Method, Request};
use hyper_rustls::HttpsConnectorBuilder;
use once_cell::sync::OnceCell;
use sha2::{Digest, Sha256};

static DASH0_TOKEN: OnceCell<Option<String>> = OnceCell::new();

/// Initialize the Dash0 token, fetching from Secrets Manager if configured.
/// Must be called once at startup from an async context.
pub async fn init_dash0_token() {
    let token = resolve_token().await;
    if token.is_none() {
        tracing::warn!(
            "[{}] No Dash0 token configured, no telemetry will be collected",
            crate::log_prefix()
        );
    }
    if DASH0_TOKEN.set(token).is_err() {
        tracing::warn!("[{}] Dash0 token already initialized", crate::log_prefix());
    }
}

/// Returns the cached Dash0 token, if any.
pub fn get_dash0_token() -> Option<String> {
    DASH0_TOKEN.get().and_then(|t| t.clone())
}

async fn resolve_token() -> Option<String> {
    let start = std::time::Instant::now();

    if let Ok(arn) = std::env::var("DASH0_TOKEN_SECRET_ARN") {
        if !arn.is_empty() {
            match fetch_secret_value(&arn).await {
                Ok(secret_string) => {
                    let token = extract_token_from_secret(&secret_string);
                    if token.is_some() {
                        tracing::info!(
                            "[{}] Dash0 token resolved from Secrets Manager in {}ms",
                            crate::log_prefix(),
                            start.elapsed().as_millis()
                        );
                        return token;
                    }
                    tracing::error!(
                        "[{}] Secret from Secrets Manager was empty or key not found, falling back to DASH0_TOKEN env var",
                        crate::log_prefix()
                    );
                }
                Err(e) => {
                    tracing::error!(
                        "[{}] Failed to fetch secret from Secrets Manager: {}, falling back to DASH0_TOKEN env var",
                        crate::log_prefix(),
                        e
                    );
                }
            }
        }
    }

    let token = std::env::var("DASH0_TOKEN").ok().filter(|v| !v.is_empty());
    token
}

/// If DASH0_TOKEN_SECRET_KEY is set, parse the secret as JSON and extract that field.
/// Otherwise, use the entire secret string as the token.
fn extract_token_from_secret(secret_string: &str) -> Option<String> {
    match std::env::var("DASH0_TOKEN_SECRET_KEY") {
        Ok(key) if !key.is_empty() => {
            let parsed: serde_json::Value = match serde_json::from_str(secret_string) {
                Ok(v) => v,
                Err(e) => {
                    tracing::error!(
                        "[{}] DASH0_TOKEN_SECRET_KEY is set but secret is not valid JSON: {}",
                        crate::log_prefix(),
                        e
                    );
                    return None;
                }
            };
            match parsed.get(&key).and_then(|v| v.as_str()) {
                Some(val) if !val.is_empty() => Some(val.to_string()),
                _ => {
                    tracing::error!(
                        "[{}] Key '{}' not found or empty in secret JSON",
                        crate::log_prefix(),
                        key
                    );
                    None
                }
            }
        }
        _ => {
            if secret_string.is_empty() {
                None
            } else {
                Some(secret_string.to_string())
            }
        }
    }
}

/// Parse the AWS region from a Secrets Manager ARN.
/// Expected format: arn:aws:secretsmanager:{region}:{account}:secret:{name}
fn parse_region_from_arn(arn: &str) -> Result<String, String> {
    let parts: Vec<&str> = arn.split(':').collect();
    if parts.len() < 4 || parts[2] != "secretsmanager" {
        return Err(format!("Invalid Secrets Manager ARN: {}", arn));
    }
    let region = parts[3];
    if region.is_empty() {
        return Err(format!("Empty region in ARN: {}", arn));
    }
    Ok(region.to_string())
}

/// Fetch a secret value from AWS Secrets Manager using a raw HTTPS call with SigV4 signing.
async fn fetch_secret_value(secret_arn: &str) -> Result<String, String> {
    let region = parse_region_from_arn(secret_arn)?;
    let host = format!("secretsmanager.{}.amazonaws.com", region);

    let access_key =
        std::env::var("AWS_ACCESS_KEY_ID").map_err(|_| "AWS_ACCESS_KEY_ID not set".to_string())?;
    let secret_key = std::env::var("AWS_SECRET_ACCESS_KEY")
        .map_err(|_| "AWS_SECRET_ACCESS_KEY not set".to_string())?;
    let session_token = std::env::var("AWS_SESSION_TOKEN").ok();

    let request_body = serde_json::json!({ "SecretId": secret_arn }).to_string();
    let now = chrono::Utc::now();
    let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
    let date_stamp = now.format("%Y%m%d").to_string();

    let body_hash = hex_sha256(request_body.as_bytes());

    // Build canonical headers and signed headers
    let mut canonical_headers = format!(
        "content-type:application/x-amz-json-1.1\nhost:{}\nx-amz-date:{}\n",
        host, amz_date
    );
    let mut signed_headers = "content-type;host;x-amz-date".to_string();

    if let Some(ref token) = session_token {
        canonical_headers.push_str(&format!("x-amz-security-token:{}\n", token));
        signed_headers.push_str(";x-amz-security-token");
    }

    canonical_headers.push_str(&format!("x-amz-target:secretsmanager.GetSecretValue\n"));
    signed_headers.push_str(";x-amz-target");

    let canonical_request = format!(
        "POST\n/\n\n{}\n{}\n{}",
        canonical_headers, signed_headers, body_hash
    );

    let credential_scope = format!("{}/{}/secretsmanager/aws4_request", date_stamp, region);
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{}\n{}\n{}",
        amz_date,
        credential_scope,
        hex_sha256(canonical_request.as_bytes())
    );

    let signing_key = derive_signing_key(&secret_key, &date_stamp, &region, "secretsmanager");
    let signature = hex::encode(hmac_sha256(&signing_key, string_to_sign.as_bytes()));

    let authorization = format!(
        "AWS4-HMAC-SHA256 Credential={}/{}, SignedHeaders={}, Signature={}",
        access_key, credential_scope, signed_headers, signature
    );

    let uri = format!("https://{}/", host);
    let mut builder = Request::builder()
        .method(Method::POST)
        .uri(&uri)
        .header("Content-Type", "application/x-amz-json-1.1")
        .header("Host", &host)
        .header("X-Amz-Date", &amz_date)
        .header("X-Amz-Target", "secretsmanager.GetSecretValue")
        .header("Authorization", &authorization);

    if let Some(ref token) = session_token {
        builder = builder.header("X-Amz-Security-Token", token);
    }

    let request = builder
        .body(Body::from(request_body))
        .map_err(|e| format!("Failed to build request: {}", e))?;

    let https = HttpsConnectorBuilder::new()
        .with_native_roots()
        .https_only()
        .enable_http1()
        .build();
    let client = Client::builder().build(https);

    let timeout = std::time::Duration::from_millis(crate::config::request_timeout_ms());
    let response = tokio::time::timeout(timeout, client.request(request))
        .await
        .map_err(|_| {
            format!(
                "Secrets Manager request timed out after {}ms",
                timeout.as_millis()
            )
        })?
        .map_err(|e| format!("Secrets Manager request failed: {}", e))?;

    let status = response.status();
    let response_body = body::to_bytes(response.into_body())
        .await
        .map_err(|e| format!("Failed to read response body: {}", e))?;

    if !status.is_success() {
        let body_str = String::from_utf8_lossy(&response_body);
        return Err(format!(
            "Secrets Manager returned HTTP {}: {}",
            status, body_str
        ));
    }

    let parsed: serde_json::Value = serde_json::from_slice(&response_body)
        .map_err(|e| format!("Failed to parse Secrets Manager response: {}", e))?;

    parsed
        .get("SecretString")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "SecretString not found in Secrets Manager response".to_string())
}

fn hex_sha256(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC can take key of any size");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

/// Derive the SigV4 signing key.
fn derive_signing_key(secret_key: &str, date_stamp: &str, region: &str, service: &str) -> Vec<u8> {
    let k_date = hmac_sha256(
        format!("AWS4{}", secret_key).as_bytes(),
        date_stamp.as_bytes(),
    );
    let k_region = hmac_sha256(&k_date, region.as_bytes());
    let k_service = hmac_sha256(&k_region, service.as_bytes());
    hmac_sha256(&k_service, b"aws4_request")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    #[test]
    fn parse_region_from_valid_arn() {
        let arn = "arn:aws:secretsmanager:us-east-1:123456789012:secret:my-secret-AbCdEf";
        assert_eq!(parse_region_from_arn(arn).unwrap(), "us-east-1");
    }

    #[test]
    fn parse_region_from_arn_different_region() {
        let arn = "arn:aws:secretsmanager:eu-west-1:123456789012:secret:test";
        assert_eq!(parse_region_from_arn(arn).unwrap(), "eu-west-1");
    }

    #[test]
    fn parse_region_from_invalid_arn() {
        assert!(parse_region_from_arn("not-an-arn").is_err());
        assert!(parse_region_from_arn("arn:aws:s3:::bucket").is_err());
    }

    #[test]
    #[serial]
    fn extract_token_plain_string() {
        std::env::remove_var("DASH0_TOKEN_SECRET_KEY");
        assert_eq!(
            extract_token_from_secret("my-token-value"),
            Some("my-token-value".to_string())
        );
    }

    #[test]
    #[serial]
    fn extract_token_empty_string() {
        std::env::remove_var("DASH0_TOKEN_SECRET_KEY");
        assert_eq!(extract_token_from_secret(""), None);
    }

    #[test]
    #[serial]
    fn extract_token_json_with_key() {
        std::env::set_var("DASH0_TOKEN_SECRET_KEY", "apiToken");
        let secret = r#"{"apiToken": "secret-123", "other": "value"}"#;
        assert_eq!(
            extract_token_from_secret(secret),
            Some("secret-123".to_string())
        );
        std::env::remove_var("DASH0_TOKEN_SECRET_KEY");
    }

    #[test]
    #[serial]
    fn extract_token_json_missing_key() {
        std::env::set_var("DASH0_TOKEN_SECRET_KEY", "missing");
        let secret = r#"{"apiToken": "secret-123"}"#;
        assert_eq!(extract_token_from_secret(secret), None);
        std::env::remove_var("DASH0_TOKEN_SECRET_KEY");
    }

    #[test]
    #[serial]
    fn extract_token_json_key_set_but_not_json() {
        std::env::set_var("DASH0_TOKEN_SECRET_KEY", "apiToken");
        assert_eq!(extract_token_from_secret("plain-text"), None);
        std::env::remove_var("DASH0_TOKEN_SECRET_KEY");
    }

    // AWS SigV4 test vector: verify signing key derivation
    // Reference: https://docs.aws.amazon.com/general/latest/gr/sigv4-calculate-signature.html
    #[test]
    fn sigv4_signing_key_derivation() {
        let key = derive_signing_key(
            "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            "20120215",
            "us-east-1",
            "iam",
        );
        let expected = "f4780e2d9f65fa895f9c67b32ce1baf0b0d8a43505a000a1a9e090d414db404d";
        assert_eq!(hex::encode(&key), expected);
    }

    #[test]
    fn hex_sha256_works() {
        // SHA256 of empty string
        assert_eq!(
            hex_sha256(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
