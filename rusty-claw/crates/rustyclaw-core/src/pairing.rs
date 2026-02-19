use std::collections::HashSet;
use std::path::Path;

use anyhow::Result;
use rand::Rng;
use serde::{Deserialize, Serialize};

/// Pending sender awaiting approval
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairingPendingEntry {
    pub channel: String,
    #[serde(rename = "senderId")]
    pub sender_id: String,
    pub sender: String,
    pub code: String,
    #[serde(rename = "createdAt")]
    pub created_at: u64,
    #[serde(rename = "lastSeenAt")]
    pub last_seen_at: u64,
}

/// Approved sender
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairingApprovedEntry {
    pub channel: String,
    #[serde(rename = "senderId")]
    pub sender_id: String,
    pub sender: String,
    #[serde(rename = "approvedAt")]
    pub approved_at: u64,
    #[serde(rename = "approvedCode", skip_serializing_if = "Option::is_none")]
    pub approved_code: Option<String>,
}

/// Full pairing file structure
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PairingState {
    pub pending: Vec<PairingPendingEntry>,
    pub approved: Vec<PairingApprovedEntry>,
}

/// Result of a pairing check
#[derive(Debug, Clone)]
pub struct PairingCheckResult {
    pub approved: bool,
    pub code: Option<String>,
    pub is_new_pending: Option<bool>,
}

/// Result of approving a pairing code
#[derive(Debug, Clone)]
pub struct PairingApproveResult {
    pub ok: bool,
    pub reason: Option<String>,
    pub entry: Option<PairingApprovedEntry>,
}

/// Pairing code alphabet — excludes confusing chars (0/O, 1/I/L)
const PAIRING_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const PAIRING_CODE_LEN: usize = 8;

fn make_sender_key(channel: &str, sender_id: &str) -> String {
    format!("{}::{}", channel, sender_id)
}

fn random_pairing_code() -> String {
    let mut rng = rand::thread_rng();
    (0..PAIRING_CODE_LEN)
        .map(|_| {
            let idx = rng.gen_range(0..PAIRING_ALPHABET.len());
            PAIRING_ALPHABET[idx] as char
        })
        .collect()
}

fn create_unique_code(state: &PairingState) -> String {
    let existing: HashSet<String> = state
        .pending
        .iter()
        .map(|e| e.code.to_uppercase())
        .chain(
            state
                .approved
                .iter()
                .filter_map(|e| e.approved_code.as_ref())
                .map(|c| c.to_uppercase()),
        )
        .collect();

    for _ in 0..20 {
        let candidate = random_pairing_code();
        if !existing.contains(&candidate) {
            return candidate;
        }
    }

    // Fallback: timestamp-based
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let encoded = format!("{:X}", ts);
    let padded = format!("{:A>8}", &encoded[encoded.len().saturating_sub(8)..]);
    padded[..8].to_string()
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Load pairing state from file. Returns default state if file missing or invalid.
pub fn load_pairing_state(pairing_file: &Path) -> PairingState {
    if !pairing_file.exists() {
        return PairingState::default();
    }
    match std::fs::read_to_string(pairing_file) {
        Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
        Err(_) => PairingState::default(),
    }
}

/// Save pairing state atomically (write to .tmp then rename).
pub fn save_pairing_state(pairing_file: &Path, state: &PairingState) -> Result<()> {
    if let Some(dir) = pairing_file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = pairing_file.with_extension("tmp");
    let json = serde_json::to_string_pretty(state)?;
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, pairing_file)?;
    Ok(())
}

/// Check if a sender is paired. If not, creates a pending entry with a pairing code.
pub fn ensure_sender_paired(
    pairing_file: &Path,
    channel: &str,
    sender_id: &str,
    sender: &str,
) -> PairingCheckResult {
    let mut state = load_pairing_state(pairing_file);
    let sender_key = make_sender_key(channel, sender_id);

    // Check if already approved
    let approved_key_map: std::collections::HashMap<String, usize> = state
        .approved
        .iter()
        .enumerate()
        .map(|(i, e)| (make_sender_key(&e.channel, &e.sender_id), i))
        .collect();

    if let Some(&idx) = approved_key_map.get(&sender_key) {
        if state.approved[idx].sender != sender {
            state.approved[idx].sender = sender.to_string();
            let _ = save_pairing_state(pairing_file, &state);
        }
        return PairingCheckResult {
            approved: true,
            code: None,
            is_new_pending: None,
        };
    }

    // Check if already pending
    if let Some(existing) = state
        .pending
        .iter_mut()
        .find(|e| e.channel == channel && e.sender_id == sender_id)
    {
        existing.last_seen_at = now_millis();
        existing.sender = sender.to_string();
        let code = existing.code.clone();
        let _ = save_pairing_state(pairing_file, &state);
        return PairingCheckResult {
            approved: false,
            code: Some(code),
            is_new_pending: Some(false),
        };
    }

    // New pending entry
    let code = create_unique_code(&state);
    let now = now_millis();
    state.pending.push(PairingPendingEntry {
        channel: channel.to_string(),
        sender_id: sender_id.to_string(),
        sender: sender.to_string(),
        code: code.clone(),
        created_at: now,
        last_seen_at: now,
    });
    let _ = save_pairing_state(pairing_file, &state);
    PairingCheckResult {
        approved: false,
        code: Some(code),
        is_new_pending: Some(true),
    }
}

/// Approve a pending pairing code. Moves sender from pending to approved.
pub fn approve_pairing_code(pairing_file: &Path, code: &str) -> PairingApproveResult {
    let normalized = code.trim().to_uppercase();
    if normalized.is_empty() {
        return PairingApproveResult {
            ok: false,
            reason: Some("Pairing code is required.".to_string()),
            entry: None,
        };
    }

    let mut state = load_pairing_state(pairing_file);
    let pending_idx = state
        .pending
        .iter()
        .position(|e| e.code.to_uppercase() == normalized);

    let Some(idx) = pending_idx else {
        return PairingApproveResult {
            ok: false,
            reason: Some(format!("Pairing code not found: {}", normalized)),
            entry: None,
        };
    };

    let pending = state.pending.remove(idx);

    let approved_entry = PairingApprovedEntry {
        channel: pending.channel.clone(),
        sender_id: pending.sender_id.clone(),
        sender: pending.sender.clone(),
        approved_at: now_millis(),
        approved_code: Some(normalized),
    };

    // Replace existing approved entry if found, otherwise push
    let existing_idx = state
        .approved
        .iter()
        .position(|e| e.channel == pending.channel && e.sender_id == pending.sender_id);

    if let Some(idx) = existing_idx {
        state.approved[idx] = approved_entry.clone();
    } else {
        state.approved.push(approved_entry.clone());
    }

    let _ = save_pairing_state(pairing_file, &state);
    PairingApproveResult {
        ok: true,
        reason: None,
        entry: Some(approved_entry),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_random_pairing_code_format() {
        let code = random_pairing_code();
        assert_eq!(code.len(), 8);
        assert!(code.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn test_load_missing_file() {
        let state = load_pairing_state(Path::new("/nonexistent/pairing.json"));
        assert!(state.pending.is_empty());
        assert!(state.approved.is_empty());
    }

    #[test]
    fn test_full_pairing_lifecycle() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("pairing.json");

        // First contact — should be new pending
        let result = ensure_sender_paired(&file, "telegram", "123", "Alice");
        assert!(!result.approved);
        assert!(result.code.is_some());
        assert_eq!(result.is_new_pending, Some(true));
        let code = result.code.unwrap();

        // Second contact — should be existing pending
        let result2 = ensure_sender_paired(&file, "telegram", "123", "Alice");
        assert!(!result2.approved);
        assert_eq!(result2.code.as_deref(), Some(code.as_str()));
        assert_eq!(result2.is_new_pending, Some(false));

        // Approve
        let approve = approve_pairing_code(&file, &code);
        assert!(approve.ok);
        assert!(approve.entry.is_some());

        // Now should be approved
        let result3 = ensure_sender_paired(&file, "telegram", "123", "Alice");
        assert!(result3.approved);
    }

    #[test]
    fn test_approve_nonexistent_code() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("pairing.json");

        let result = approve_pairing_code(&file, "NONEXIST");
        assert!(!result.ok);
        assert!(result.reason.unwrap().contains("not found"));
    }

    #[test]
    fn test_approve_empty_code() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("pairing.json");

        let result = approve_pairing_code(&file, "");
        assert!(!result.ok);
        assert!(result.reason.unwrap().contains("required"));
    }

    #[test]
    fn test_sender_name_update() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("pairing.json");

        let r1 = ensure_sender_paired(&file, "discord", "456", "Bob");
        let code = r1.code.unwrap();
        approve_pairing_code(&file, &code);

        // Contact again with different name
        let r2 = ensure_sender_paired(&file, "discord", "456", "Robert");
        assert!(r2.approved);

        // Verify name was updated
        let state = load_pairing_state(&file);
        let entry = state.approved.iter().find(|e| e.sender_id == "456").unwrap();
        assert_eq!(entry.sender, "Robert");
    }
}
