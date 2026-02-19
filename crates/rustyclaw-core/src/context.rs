use std::collections::HashMap;
use std::path::Path;

use crate::skills::{self, Skill};
use crate::transcript;
use crate::types::SkillOverride;

/// Maximum characters of transcript history to include in context.
pub const MAX_TRANSCRIPT_CONTEXT_CHARS: usize = 8000;

/// Maximum characters to read from MEMORY.md before truncating.
pub const MAX_MEMORY_FILE_CHARS: usize = 10_000;

/// Read a file if it exists and has non-empty content.
fn read_optional_file(path: &Path) -> Option<String> {
    if !path.exists() {
        return None;
    }
    let content = std::fs::read_to_string(path).ok()?;
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_string())
}

/// Read a file with a maximum character limit.
fn read_optional_file_capped(path: &Path, max_chars: usize) -> Option<String> {
    let content = read_optional_file(path)?;
    if content.len() <= max_chars {
        Some(content)
    } else {
        Some(format!(
            "{}... [truncated, {} chars total]",
            &content[..max_chars],
            content.len()
        ))
    }
}

/// Build the full context preamble from bootstrap files, memory, and transcripts.
///
/// Returns a string to prepend to the user message, or empty string if no files
/// have content. The format uses XML-style tags that all LLM providers understand.
pub fn build_context_preamble(
    agent_dir: &Path,
    _agent_id: &str,
    max_transcript_chars: usize,
    skill_dirs: &[&Path],
    skill_overrides: &HashMap<String, SkillOverride>,
) -> String {
    let rustyclaw_dir = agent_dir.join(".rustyclaw");

    let mut sections: Vec<String> = Vec::new();

    // Bootstrap files
    if let Some(content) = read_optional_file(&rustyclaw_dir.join("IDENTITY.md")) {
        sections.push(format!("<identity>\n{}\n</identity>", content));
    }
    if let Some(content) = read_optional_file(&rustyclaw_dir.join("USER.md")) {
        sections.push(format!("<user>\n{}\n</user>", content));
    }
    if let Some(content) = read_optional_file(&rustyclaw_dir.join("TOOLS.md")) {
        sections.push(format!("<tools>\n{}\n</tools>", content));
    }

    // Long-term memory (capped)
    if let Some(content) =
        read_optional_file_capped(&rustyclaw_dir.join("MEMORY.md"), MAX_MEMORY_FILE_CHARS)
    {
        sections.push(format!("<memory>\n{}\n</memory>", content));
    }

    // Daily notes (today)
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let daily_path = rustyclaw_dir.join("memory").join(format!("{}.md", today));
    if let Some(content) = read_optional_file(&daily_path) {
        sections.push(format!(
            "<daily_notes date=\"{}\">\n{}\n</daily_notes>",
            today, content
        ));
    }

    // Recent transcript history
    let transcript_context = transcript::read_recent_transcript_context(
        &rustyclaw_dir.join("transcripts"),
        max_transcript_chars,
    );
    if !transcript_context.is_empty() {
        sections.push(format!(
            "<recent_history>\n{}\n</recent_history>",
            transcript_context
        ));
    }

    // Skills injection
    if !skill_dirs.is_empty() {
        let all_skills = skills::discover_skills(skill_dirs);
        let eligible: Vec<Skill> = all_skills
            .into_iter()
            .filter(|s| skills::is_skill_eligible(s, skill_overrides))
            .collect();
        let skills_text = skills::format_skills_for_context(&eligible);
        if !skills_text.is_empty() {
            sections.push(format!("<skills>\n{}\n</skills>", skills_text));
        }
    }

    if sections.is_empty() {
        return String::new();
    }

    format!("<context>\n{}\n</context>\n\n", sections.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_empty_agent_dir() {
        let tmp = TempDir::new().unwrap();
        let result = build_context_preamble(tmp.path(), "test-agent", 8000, &[], &HashMap::new());
        assert!(result.is_empty());
    }

    #[test]
    fn test_with_identity_file() {
        let tmp = TempDir::new().unwrap();
        let rustyclaw = tmp.path().join(".rustyclaw");
        std::fs::create_dir_all(&rustyclaw).unwrap();
        std::fs::write(rustyclaw.join("IDENTITY.md"), "I am a test agent").unwrap();

        let result = build_context_preamble(tmp.path(), "test-agent", 8000, &[], &HashMap::new());
        assert!(result.contains("<context>"));
        assert!(result.contains("<identity>"));
        assert!(result.contains("I am a test agent"));
        assert!(result.contains("</identity>"));
        assert!(result.contains("</context>"));
    }

    #[test]
    fn test_skips_empty_files() {
        let tmp = TempDir::new().unwrap();
        let rustyclaw = tmp.path().join(".rustyclaw");
        std::fs::create_dir_all(&rustyclaw).unwrap();
        std::fs::write(rustyclaw.join("IDENTITY.md"), "").unwrap();
        std::fs::write(rustyclaw.join("USER.md"), "  \n  ").unwrap();

        let result = build_context_preamble(tmp.path(), "test-agent", 8000, &[], &HashMap::new());
        assert!(result.is_empty());
    }

    #[test]
    fn test_memory_truncation() {
        let tmp = TempDir::new().unwrap();
        let rustyclaw = tmp.path().join(".rustyclaw");
        std::fs::create_dir_all(&rustyclaw).unwrap();

        // Write a large memory file
        let large_content = "x".repeat(20_000);
        std::fs::write(rustyclaw.join("MEMORY.md"), &large_content).unwrap();

        let result = build_context_preamble(tmp.path(), "test-agent", 8000, &[], &HashMap::new());
        assert!(result.contains("<memory>"));
        assert!(result.contains("[truncated, 20000 chars total]"));
    }

    #[test]
    fn test_all_sections_present() {
        let tmp = TempDir::new().unwrap();
        let rustyclaw = tmp.path().join(".rustyclaw");
        std::fs::create_dir_all(&rustyclaw).unwrap();
        std::fs::create_dir_all(rustyclaw.join("memory")).unwrap();

        std::fs::write(rustyclaw.join("IDENTITY.md"), "identity content").unwrap();
        std::fs::write(rustyclaw.join("USER.md"), "user content").unwrap();
        std::fs::write(rustyclaw.join("TOOLS.md"), "tools content").unwrap();
        std::fs::write(rustyclaw.join("MEMORY.md"), "memory content").unwrap();

        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        std::fs::write(
            rustyclaw.join("memory").join(format!("{}.md", today)),
            "daily notes",
        )
        .unwrap();

        let result = build_context_preamble(tmp.path(), "test-agent", 8000, &[], &HashMap::new());
        assert!(result.contains("<identity>"));
        assert!(result.contains("<user>"));
        assert!(result.contains("<tools>"));
        assert!(result.contains("<memory>"));
        assert!(result.contains("<daily_notes"));
    }
}
