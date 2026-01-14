use aws_config::BehaviorVersion;
use aws_sdk_sts::Client as StsClient;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::time::Instant;

static FUNCTION_ARN: Lazy<Mutex<Option<String>>> = Lazy::new(|| Mutex::new(None));
static ACCOUNT_ID: Lazy<Mutex<Option<String>>> = Lazy::new(|| Mutex::new(None));

pub fn store_function_arn(arn: &str) {
    let mut guard = FUNCTION_ARN.lock();
    if guard.is_none() {
        *guard = Some(arn.to_string());
        if let Some(account) = parse_account_id_from_arn(arn) {
            let mut acct_guard = ACCOUNT_ID.lock();
            if acct_guard.is_none() {
                *acct_guard = Some(account);
            }
        }
    }
}

pub fn get_function_arn() -> Option<String> {
    let mut guard = FUNCTION_ARN.lock();
    if guard.as_ref().map(|arn| !arn.is_empty()).unwrap_or(false) {
        return guard.clone();
    }

    let account_id = get_account_id().filter(|id| !id.is_empty());
    let region = std::env::var("AWS_REGION").ok().filter(|r| !r.is_empty());
    let function_name = std::env::var("AWS_LAMBDA_FUNCTION_NAME")
        .ok()
        .filter(|name| !name.is_empty());

    match (account_id, region, function_name) {
        (Some(account_id), Some(region), Some(function_name)) => {
            let arn = format!(
                "arn:aws:lambda:{}:{}:function:{}",
                region, account_id, function_name
            );
            *guard = Some(arn.clone());
            Some(arn)
        }
        _ => None,
    }
}

pub fn get_account_id() -> Option<String> {
    ACCOUNT_ID.lock().clone()
}

pub async fn fetch_and_cache_account_id() -> Option<String> {
    let start = Instant::now();
    let config = aws_config::load_defaults(BehaviorVersion::latest()).await;
    let client = StsClient::new(&config);
    match client.get_caller_identity().send().await {
        Ok(output) => {
            if let Some(account) = output.account() {
                let mut guard = ACCOUNT_ID.lock();
                if guard.as_ref().map(|id| !id.is_empty()).unwrap_or(false) {
                    return guard.clone();
                }
                let account = account.to_string();
                *guard = Some(account.clone());
                tracing::info!(
                    "[{}] Retrieved account ID={} from STS GetCallerIdentity in {} ms",
                    crate::log_prefix(),
                    account,
                    start.elapsed().as_millis()
                );
                Some(account)
            } else {
                tracing::error!(
                    "[{}] STS GetCallerIdentity did not return an account ID",
                    crate::log_prefix()
                );
                None
            }
        }
        Err(err) => {
            tracing::error!(
                "[{}] Failed calling STS GetCallerIdentity: {}",
                crate::log_prefix(),
                err
            );
            None
        }
    }
}

fn parse_account_id_from_arn(arn: &str) -> Option<String> {
    // arn:partition:service:region:account-id:resource
    let parts: Vec<&str> = arn.split(':').collect();
    if parts.len() >= 5 {
        Some(parts[4].to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{get_function_arn, store_function_arn, ACCOUNT_ID, FUNCTION_ARN};
    use serial_test::serial;
    use std::env;

    fn reset_state() {
        FUNCTION_ARN.lock().take();
        ACCOUNT_ID.lock().take();
        env::remove_var("AWS_REGION");
        env::remove_var("AWS_LAMBDA_FUNCTION_NAME");
    }

    #[test]
    #[serial]
    fn returns_cached_function_arn_when_set() {
        reset_state();
        let cached = "arn:aws:lambda:us-east-1:123456789012:function:cached-func";
        store_function_arn(cached);

        assert_eq!(get_function_arn(), Some(cached.to_string()));
    }

    #[test]
    #[serial]
    fn constructs_arn_from_env_when_missing() {
        reset_state();
        *ACCOUNT_ID.lock() = Some("111122223333".to_string());
        env::set_var("AWS_REGION", "us-west-2");
        env::set_var("AWS_LAMBDA_FUNCTION_NAME", "my-function");

        let arn = get_function_arn();
        assert_eq!(
            arn.as_deref(),
            Some("arn:aws:lambda:us-west-2:111122223333:function:my-function")
        );

        // Ensure it is cached for subsequent calls even if env vars are removed
        env::remove_var("AWS_REGION");
        env::remove_var("AWS_LAMBDA_FUNCTION_NAME");
        assert_eq!(
            get_function_arn(),
            Some("arn:aws:lambda:us-west-2:111122223333:function:my-function".to_string())
        );
    }

    #[test]
    #[serial]
    fn returns_none_when_any_component_missing() {
        reset_state();
        // Region present but account id missing
        env::set_var("AWS_REGION", "eu-central-1");
        env::set_var("AWS_LAMBDA_FUNCTION_NAME", "missing-account");

        assert_eq!(get_function_arn(), None);
        assert!(FUNCTION_ARN.lock().is_none());

        // Account present but function name missing
        *ACCOUNT_ID.lock() = Some("444455556666".to_string());
        env::remove_var("AWS_LAMBDA_FUNCTION_NAME");
        assert_eq!(get_function_arn(), None);
    }
}
