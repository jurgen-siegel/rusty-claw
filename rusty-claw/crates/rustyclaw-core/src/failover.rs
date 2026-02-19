use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Reason for a model invocation failure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FailoverReason {
    RateLimit,
    Auth,
    Timeout,
    Unknown,
}

/// Cooldown entry for a specific provider:model combination.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CooldownEntry {
    /// Timestamp (ms) until which this model is in cooldown.
    pub until: u64,
    /// Consecutive error count.
    pub error_count: u32,
}

/// Build a cooldown key from provider and model.
pub fn cooldown_key(provider: &str, model: &str) -> String {
    format!("{}:{}", provider, model)
}

/// Load cooldowns from a JSON file.
pub fn load_cooldowns(path: &Path) -> HashMap<String, CooldownEntry> {
    if !path.exists() {
        return HashMap::new();
    }
    match std::fs::read_to_string(path) {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_default(),
        Err(_) => HashMap::new(),
    }
}

/// Save cooldowns to a JSON file.
pub fn save_cooldowns(path: &Path, cooldowns: &HashMap<String, CooldownEntry>) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string_pretty(cooldowns)?;
    std::fs::write(path, json)?;
    Ok(())
}

/// Check if a model is currently in cooldown.
pub fn is_in_cooldown(cooldowns: &HashMap<String, CooldownEntry>, key: &str) -> bool {
    if let Some(entry) = cooldowns.get(key) {
        let now = now_millis();
        now < entry.until
    } else {
        false
    }
}

/// Record a failure for a model, updating its cooldown.
pub fn record_failure(
    cooldowns: &mut HashMap<String, CooldownEntry>,
    key: &str,
    _reason: FailoverReason,
) {
    let entry = cooldowns.entry(key.to_string()).or_insert(CooldownEntry {
        until: 0,
        error_count: 0,
    });
    entry.error_count += 1;
    let cooldown_ms = calculate_cooldown_ms(entry.error_count);
    entry.until = now_millis() + cooldown_ms;
}

/// Clear cooldown for a model after a successful invocation.
pub fn clear_cooldown(cooldowns: &mut HashMap<String, CooldownEntry>, key: &str) {
    cooldowns.remove(key);
}

/// Calculate cooldown duration in milliseconds using exponential backoff.
/// Progression: 60s → 5min → 25min → 60min (capped).
pub fn calculate_cooldown_ms(error_count: u32) -> u64 {
    let count = error_count.max(1);
    let exponent = (count - 1).min(3);
    let seconds = 60u64 * 5u64.pow(exponent);
    seconds.min(3600) * 1000 // cap at 1 hour, convert to ms
}

/// Classify an error message into a failover reason.
pub fn classify_error(error_msg: &str) -> FailoverReason {
    let lower = error_msg.to_lowercase();

    if lower.contains("rate limit")
        || lower.contains("rate_limit")
        || lower.contains("429")
        || lower.contains("too many requests")
    {
        return FailoverReason::RateLimit;
    }

    if lower.contains("401")
        || lower.contains("403")
        || lower.contains("402")
        || lower.contains("unauthorized")
        || lower.contains("forbidden")
        || lower.contains("authentication")
        || lower.contains("invalid api key")
        || lower.contains("billing")
        || lower.contains("credit")
    {
        return FailoverReason::Auth;
    }

    if lower.contains("timeout")
        || lower.contains("408")
        || lower.contains("timed out")
        || lower.contains("etimedout")
        || lower.contains("econnreset")
    {
        return FailoverReason::Timeout;
    }

    FailoverReason::Unknown
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_cooldown_key() {
        assert_eq!(cooldown_key("anthropic", "opus"), "anthropic:opus");
    }

    #[test]
    fn test_cooldown_progression() {
        assert_eq!(calculate_cooldown_ms(1), 60_000);       // 60s
        assert_eq!(calculate_cooldown_ms(2), 300_000);      // 5min
        assert_eq!(calculate_cooldown_ms(3), 1_500_000);    // 25min
        assert_eq!(calculate_cooldown_ms(4), 3_600_000);    // 60min (capped)
        assert_eq!(calculate_cooldown_ms(5), 3_600_000);    // still capped
    }

    #[test]
    fn test_classify_rate_limit() {
        assert_eq!(classify_error("429 Too Many Requests"), FailoverReason::RateLimit);
        assert_eq!(classify_error("rate limit exceeded"), FailoverReason::RateLimit);
        assert_eq!(classify_error("rate_limit_error"), FailoverReason::RateLimit);
    }

    #[test]
    fn test_classify_auth() {
        assert_eq!(classify_error("401 Unauthorized"), FailoverReason::Auth);
        assert_eq!(classify_error("invalid api key"), FailoverReason::Auth);
        assert_eq!(classify_error("403 Forbidden"), FailoverReason::Auth);
    }

    #[test]
    fn test_classify_timeout() {
        assert_eq!(classify_error("request timed out"), FailoverReason::Timeout);
        assert_eq!(classify_error("ETIMEDOUT"), FailoverReason::Timeout);
    }

    #[test]
    fn test_classify_unknown() {
        assert_eq!(classify_error("something went wrong"), FailoverReason::Unknown);
    }

    #[test]
    fn test_record_and_check_cooldown() {
        let mut cooldowns = HashMap::new();
        let key = "anthropic:opus";

        assert!(!is_in_cooldown(&cooldowns, key));

        record_failure(&mut cooldowns, key, FailoverReason::RateLimit);
        assert!(is_in_cooldown(&cooldowns, key));
        assert_eq!(cooldowns[key].error_count, 1);
    }

    #[test]
    fn test_clear_cooldown() {
        let mut cooldowns = HashMap::new();
        let key = "anthropic:opus";
        record_failure(&mut cooldowns, key, FailoverReason::Unknown);
        assert!(is_in_cooldown(&cooldowns, key));

        clear_cooldown(&mut cooldowns, key);
        assert!(!is_in_cooldown(&cooldowns, key));
    }

    #[test]
    fn test_save_and_load_cooldowns() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("cooldowns.json");

        let mut cooldowns = HashMap::new();
        record_failure(&mut cooldowns, "anthropic:opus", FailoverReason::RateLimit);

        save_cooldowns(&path, &cooldowns).unwrap();
        let loaded = load_cooldowns(&path);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded["anthropic:opus"].error_count, 1);
    }

    #[test]
    fn test_escalating_cooldown() {
        let mut cooldowns = HashMap::new();
        let key = "anthropic:opus";

        record_failure(&mut cooldowns, key, FailoverReason::RateLimit);
        assert_eq!(cooldowns[key].error_count, 1);

        record_failure(&mut cooldowns, key, FailoverReason::RateLimit);
        assert_eq!(cooldowns[key].error_count, 2);
        // Second failure should have longer cooldown
    }
}
