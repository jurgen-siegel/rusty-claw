use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::types::AgentConfig;

const DEFAULT_RESET_HOUR: u8 = 4;
const DEFAULT_IDLE_TIMEOUT_MINUTES: u64 = 120;

/// A single session entry in the session store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEntry {
    pub session_id: String,
    pub updated_at: u64,
    pub channel: String,
    pub sender: String,
    /// Running estimate of total characters exchanged in this session.
    #[serde(default)]
    pub total_chars: u64,
    /// Number of times this session has been compacted.
    #[serde(default)]
    pub compaction_count: u32,
}

/// Build a session key from agent, channel, and sender.
pub fn resolve_session_key(agent_id: &str, channel: &str, sender: &str) -> String {
    format!("{}:{}:{}", agent_id, channel, sender)
}

/// Load the session store from `{agent_dir}/.rustyclaw/sessions.json`.
pub fn load_sessions(agent_dir: &Path) -> HashMap<String, SessionEntry> {
    let path = agent_dir.join(".rustyclaw/sessions.json");
    if !path.exists() {
        return HashMap::new();
    }
    match std::fs::read_to_string(&path) {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_default(),
        Err(_) => HashMap::new(),
    }
}

/// Save the session store to `{agent_dir}/.rustyclaw/sessions.json`.
pub fn save_sessions(agent_dir: &Path, store: &HashMap<String, SessionEntry>) -> Result<()> {
    let dir = agent_dir.join(".rustyclaw");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("sessions.json");
    let json = serde_json::to_string_pretty(store)?;
    std::fs::write(&path, json)?;
    Ok(())
}

/// Check if a session is still fresh (should NOT be reset).
/// Returns true if the session is fresh, false if it should be reset.
pub fn evaluate_session_freshness(entry: &SessionEntry, agent: &AgentConfig) -> bool {
    let now = now_millis();
    let policy = resolve_reset_policy(agent);

    match policy.as_str() {
        "manual" => true, // never auto-reset
        "daily" => !is_stale_daily(entry.updated_at, now, resolve_reset_hour(agent)),
        "idle" => !is_stale_idle(entry.updated_at, now, resolve_idle_timeout(agent)),
        // "both" or default: reset on either condition
        _ => {
            !is_stale_daily(entry.updated_at, now, resolve_reset_hour(agent))
                && !is_stale_idle(entry.updated_at, now, resolve_idle_timeout(agent))
        }
    }
}

/// Resolve whether an agent invocation should reset, and return the session ID.
/// Also checks the legacy reset_flag file.
///
/// Returns (should_reset, session_id).
pub fn resolve_should_reset(
    agent_dir: &Path,
    agent_id: &str,
    agent: &AgentConfig,
    channel: &str,
    sender: &str,
    workspace_path: &Path,
) -> (bool, String) {
    // Check legacy reset_flag
    let reset_flag = crate::routing::get_agent_reset_flag(agent_id, workspace_path);
    let flag_reset = reset_flag.exists();
    if flag_reset {
        let _ = std::fs::remove_file(&reset_flag);
    }

    let session_key = resolve_session_key(agent_id, channel, sender);
    let sessions = load_sessions(agent_dir);

    if let Some(entry) = sessions.get(&session_key) {
        let fresh = evaluate_session_freshness(entry, agent);
        let should_reset = flag_reset || !fresh;
        (should_reset, entry.session_id.clone())
    } else {
        // No session exists — this is a new session (reset = true to start fresh)
        let session_id = generate_session_id();
        (true, session_id)
    }
}

/// Update a session entry after a successful invocation.
/// Creates the entry if it doesn't exist.
pub fn update_session(
    agent_dir: &Path,
    agent_id: &str,
    channel: &str,
    sender: &str,
    message_chars: usize,
    response_chars: usize,
    was_reset: bool,
) -> Result<SessionEntry> {
    let session_key = resolve_session_key(agent_id, channel, sender);
    let mut sessions = load_sessions(agent_dir);
    let now = now_millis();

    let entry = sessions.entry(session_key).or_insert_with(|| SessionEntry {
        session_id: generate_session_id(),
        updated_at: now,
        channel: channel.to_string(),
        sender: sender.to_string(),
        total_chars: 0,
        compaction_count: 0,
    });

    if was_reset {
        entry.session_id = generate_session_id();
        entry.total_chars = 0;
    }

    entry.updated_at = now;
    entry.total_chars += (message_chars + response_chars) as u64;

    let result = entry.clone();
    save_sessions(agent_dir, &sessions)?;
    Ok(result)
}

fn resolve_reset_policy(agent: &AgentConfig) -> String {
    if agent.reset_policy.is_empty() {
        "both".to_string()
    } else {
        agent.reset_policy.clone()
    }
}

fn resolve_reset_hour(agent: &AgentConfig) -> u8 {
    agent.reset_hour.unwrap_or(DEFAULT_RESET_HOUR)
}

fn resolve_idle_timeout(agent: &AgentConfig) -> u64 {
    agent.idle_timeout_minutes.unwrap_or(DEFAULT_IDLE_TIMEOUT_MINUTES)
}

/// Check if session is stale based on daily reset at a specific UTC hour.
fn is_stale_daily(updated_at: u64, now: u64, reset_hour: u8) -> bool {
    let now_dt = match chrono::DateTime::from_timestamp((now / 1000) as i64, 0) {
        Some(dt) => dt,
        None => return false,
    };

    // Today's reset time in UTC
    let today_reset = now_dt
        .date_naive()
        .and_hms_opt(reset_hour as u32, 0, 0)
        .map(|dt| dt.and_utc().timestamp_millis() as u64);

    let Some(reset_at) = today_reset else {
        return false;
    };

    // If we haven't passed today's reset hour yet, use yesterday's reset time
    let effective_reset = if now < reset_at {
        reset_at.saturating_sub(24 * 60 * 60 * 1000)
    } else {
        reset_at
    };

    updated_at < effective_reset
}

/// Check if session is stale based on idle timeout.
fn is_stale_idle(updated_at: u64, now: u64, timeout_minutes: u64) -> bool {
    let idle_ms = now.saturating_sub(updated_at);
    idle_ms > timeout_minutes * 60 * 1000
}

fn generate_session_id() -> String {
    let ts = Utc::now().timestamp_millis();
    let rand_part: u32 = rand::random();
    format!("sess-{}-{:08x}", ts, rand_part)
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

    fn test_agent(policy: &str) -> AgentConfig {
        AgentConfig {
            name: "Test".to_string(),
            provider: "anthropic".to_string(),
            model: "sonnet".to_string(),
            working_directory: String::new(),
            reset_policy: policy.to_string(),
            reset_hour: Some(4),
            idle_timeout_minutes: Some(60),
            context_window: None,
            fallbacks: None,
            cross_team_handoffs: true,
            route_patterns: None,
            route_priority: 0,
        }
    }

    #[test]
    fn test_resolve_session_key() {
        let key = resolve_session_key("coder", "discord", "user123");
        assert_eq!(key, "coder:discord:user123");
    }

    #[test]
    fn test_save_and_load_sessions() {
        let tmp = TempDir::new().unwrap();
        let agent_dir = tmp.path().join("coder");
        std::fs::create_dir_all(agent_dir.join(".rustyclaw")).unwrap();

        let mut store = HashMap::new();
        store.insert(
            "coder:discord:user1".to_string(),
            SessionEntry {
                session_id: "sess-123".to_string(),
                updated_at: 1708200000000,
                channel: "discord".to_string(),
                sender: "user1".to_string(),
                total_chars: 5000,
                compaction_count: 0,
            },
        );

        save_sessions(&agent_dir, &store).unwrap();
        let loaded = load_sessions(&agent_dir);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded["coder:discord:user1"].session_id, "sess-123");
        assert_eq!(loaded["coder:discord:user1"].total_chars, 5000);
    }

    #[test]
    fn test_fresh_session_manual_policy() {
        let agent = test_agent("manual");
        let entry = SessionEntry {
            session_id: "sess-1".to_string(),
            updated_at: 0, // very old
            channel: "discord".to_string(),
            sender: "user".to_string(),
            total_chars: 0,
            compaction_count: 0,
        };
        // Manual policy never auto-resets
        assert!(evaluate_session_freshness(&entry, &agent));
    }

    #[test]
    fn test_stale_idle_session() {
        let agent = test_agent("idle");
        let now = now_millis();
        let entry = SessionEntry {
            session_id: "sess-1".to_string(),
            updated_at: now - 120 * 60 * 1000, // 2 hours ago, timeout is 60min
            channel: "discord".to_string(),
            sender: "user".to_string(),
            total_chars: 0,
            compaction_count: 0,
        };
        assert!(!evaluate_session_freshness(&entry, &agent));
    }

    #[test]
    fn test_fresh_idle_session() {
        let agent = test_agent("idle");
        let now = now_millis();
        let entry = SessionEntry {
            session_id: "sess-1".to_string(),
            updated_at: now - 30 * 60 * 1000, // 30 minutes ago, timeout is 60min
            channel: "discord".to_string(),
            sender: "user".to_string(),
            total_chars: 0,
            compaction_count: 0,
        };
        assert!(evaluate_session_freshness(&entry, &agent));
    }

    #[test]
    fn test_update_session_creates_new() {
        let tmp = TempDir::new().unwrap();
        let agent_dir = tmp.path().join("coder");
        std::fs::create_dir_all(agent_dir.join(".rustyclaw")).unwrap();

        let entry =
            update_session(&agent_dir, "coder", "discord", "user1", 100, 200, true).unwrap();
        assert_eq!(entry.total_chars, 300);
        assert_eq!(entry.channel, "discord");
        assert_eq!(entry.sender, "user1");

        // Load back and verify
        let sessions = load_sessions(&agent_dir);
        assert_eq!(sessions.len(), 1);
    }

    #[test]
    fn test_update_session_accumulates_chars() {
        let tmp = TempDir::new().unwrap();
        let agent_dir = tmp.path().join("coder");
        std::fs::create_dir_all(agent_dir.join(".rustyclaw")).unwrap();

        update_session(&agent_dir, "coder", "discord", "user1", 100, 200, true).unwrap();
        let entry =
            update_session(&agent_dir, "coder", "discord", "user1", 50, 150, false).unwrap();
        assert_eq!(entry.total_chars, 500); // 300 + 200
    }

    #[test]
    fn test_update_session_reset_clears_chars() {
        let tmp = TempDir::new().unwrap();
        let agent_dir = tmp.path().join("coder");
        std::fs::create_dir_all(agent_dir.join(".rustyclaw")).unwrap();

        update_session(&agent_dir, "coder", "discord", "user1", 100, 200, true).unwrap();
        let entry =
            update_session(&agent_dir, "coder", "discord", "user1", 50, 150, true).unwrap();
        // Reset clears total_chars, then adds new message + response
        assert_eq!(entry.total_chars, 200); // 0 + 50 + 150
    }

    #[test]
    fn test_is_stale_daily() {
        let now = 1708250000000u64; // some arbitrary time
        // Very old — should be stale
        assert!(is_stale_daily(1000, now, 4));
        // Recent — should be fresh
        assert!(!is_stale_daily(now - 1000, now, 4));
    }

    #[test]
    fn test_is_stale_idle() {
        let now = 1708250000000u64;
        // 2 hours ago, 60 min timeout
        assert!(is_stale_idle(now - 7_200_000, now, 60));
        // 30 minutes ago, 60 min timeout
        assert!(!is_stale_idle(now - 1_800_000, now, 60));
    }

    #[test]
    fn test_separate_sessions_per_sender() {
        let tmp = TempDir::new().unwrap();
        let agent_dir = tmp.path().join("coder");
        std::fs::create_dir_all(agent_dir.join(".rustyclaw")).unwrap();

        update_session(&agent_dir, "coder", "discord", "alice", 100, 200, true).unwrap();
        update_session(&agent_dir, "coder", "discord", "bob", 50, 50, true).unwrap();
        update_session(&agent_dir, "coder", "telegram", "alice", 75, 75, true).unwrap();

        let sessions = load_sessions(&agent_dir);
        assert_eq!(sessions.len(), 3);
        assert_eq!(sessions["coder:discord:alice"].total_chars, 300);
        assert_eq!(sessions["coder:discord:bob"].total_chars, 100);
        assert_eq!(sessions["coder:telegram:alice"].total_chars, 150);
    }
}
