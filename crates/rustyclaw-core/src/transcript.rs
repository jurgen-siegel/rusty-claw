use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};

/// A single transcript entry (one line in a JSONL file).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptEntry {
    pub timestamp: u64,
    pub agent_id: String,
    /// "user" or "assistant"
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_length: Option<usize>,
    /// Entry type: None for normal entries, Some("compaction") for summaries.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_type: Option<String>,
    /// Total chars accumulated before compaction (only set on compaction entries).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chars_before: Option<u64>,
}

/// Get the transcript file path for today.
pub fn transcript_file_for_today(transcripts_dir: &Path) -> PathBuf {
    let today = Utc::now().format("%Y-%m-%d").to_string();
    transcripts_dir.join(format!("{}.jsonl", today))
}

/// Append a transcript entry to today's JSONL file.
pub fn append_transcript_entry(
    transcripts_dir: &Path,
    entry: &TranscriptEntry,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(transcripts_dir)?;
    let file_path = transcript_file_for_today(transcripts_dir);

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&file_path)?;

    let json = serde_json::to_string(entry)?;
    writeln!(file, "{}", json)?;
    Ok(())
}

/// Read recent transcript entries up to `max_chars` total content.
/// Reads today's file and yesterday's if needed. Returns a formatted
/// string in chronological order for context injection.
pub fn read_recent_transcript_context(
    transcripts_dir: &Path,
    max_chars: usize,
) -> String {
    if max_chars == 0 || !transcripts_dir.exists() {
        return String::new();
    }

    let today = Utc::now();
    let yesterday = today - chrono::Duration::days(1);

    let today_file = transcripts_dir.join(format!("{}.jsonl", today.format("%Y-%m-%d")));
    let yesterday_file = transcripts_dir.join(format!("{}.jsonl", yesterday.format("%Y-%m-%d")));

    // Collect entries from yesterday + today (chronological order)
    let mut entries: Vec<TranscriptEntry> = Vec::new();

    for file_path in &[yesterday_file, today_file] {
        if !file_path.exists() {
            continue;
        }
        if let Ok(file) = std::fs::File::open(file_path) {
            let reader = BufReader::new(file);
            for line in reader.lines().flatten() {
                if let Ok(entry) = serde_json::from_str::<TranscriptEntry>(&line) {
                    entries.push(entry);
                }
            }
        }
    }

    if entries.is_empty() {
        return String::new();
    }

    // Build from the END (most recent first), stopping at budget
    let mut lines: Vec<String> = Vec::new();
    let mut total_chars = 0;

    for entry in entries.iter().rev() {
        let formatted = format_transcript_entry(entry);
        let entry_chars = formatted.len();

        if total_chars + entry_chars > max_chars && !lines.is_empty() {
            break;
        }

        lines.push(formatted);
        total_chars += entry_chars;
    }

    // Reverse back to chronological order
    lines.reverse();
    lines.join("\n")
}

/// Format a single transcript entry for human-readable context injection.
fn format_transcript_entry(entry: &TranscriptEntry) -> String {
    let timestamp = chrono::DateTime::from_timestamp((entry.timestamp / 1000) as i64, 0)
        .map(|dt| dt.format("%H:%M").to_string())
        .unwrap_or_else(|| "??:??".to_string());

    // Truncate very long content to keep context manageable
    let content = if entry.content.len() > 500 {
        format!(
            "{}... [truncated, {} chars total]",
            &entry.content[..500],
            entry.content.len()
        )
    } else {
        entry.content.clone()
    };

    match entry.role.as_str() {
        "user" => format!("[{}] user: {}", timestamp, content),
        "assistant" => format!("[{}] @{}: {}", timestamp, entry.agent_id, content),
        _ => format!("[{}] {}: {}", timestamp, entry.role, content),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_append_and_read_transcript() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("transcripts");

        let entry = TranscriptEntry {
            timestamp: 1708200000000,
            agent_id: "coder".to_string(),
            role: "user".to_string(),
            content: "Fix the bug".to_string(),
            message_id: Some("msg-123".to_string()),
            channel: Some("discord".to_string()),
            sender: Some("user1".to_string()),
            response_length: None,
            entry_type: None,
            chars_before: None,
        };

        append_transcript_entry(&dir, &entry).unwrap();

        let context = read_recent_transcript_context(&dir, 10000);
        assert!(context.contains("user: Fix the bug"));
    }

    #[test]
    fn test_read_empty_transcripts() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("transcripts");
        let context = read_recent_transcript_context(&dir, 10000);
        assert!(context.is_empty());
    }

    #[test]
    fn test_transcript_char_budget() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("transcripts");

        // Write several entries
        for i in 0..20 {
            let entry = TranscriptEntry {
                timestamp: 1708200000000 + i * 60000,
                agent_id: "coder".to_string(),
                role: if i % 2 == 0 { "user" } else { "assistant" }.to_string(),
                content: format!("Message number {} with some padding text here", i),
                message_id: None,
                channel: None,
                sender: None,
                response_length: None,
                entry_type: None,
                chars_before: None,
            };
            append_transcript_entry(&dir, &entry).unwrap();
        }

        // Read with small budget
        let context = read_recent_transcript_context(&dir, 200);
        assert!(!context.is_empty());
        // Should not contain all 20 entries
        let line_count = context.lines().count();
        assert!(line_count < 20);
    }
}
