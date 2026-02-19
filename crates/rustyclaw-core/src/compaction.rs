/// Approximate characters per token for estimation purposes.
pub const CHARS_PER_TOKEN: u64 = 4;

/// Default context window size in tokens.
pub const DEFAULT_CONTEXT_WINDOW: u64 = 200_000;

/// Default reserve tokens (headroom for system prompt + response).
pub const DEFAULT_RESERVE_TOKENS: u64 = 40_000;

/// Check whether a session should be compacted based on accumulated character count.
pub fn should_compact(total_chars: u64, context_window: u64, reserve_tokens: u64) -> bool {
    total_chars > compaction_threshold_chars(context_window, reserve_tokens)
}

/// Calculate the character threshold at which compaction should trigger.
pub fn compaction_threshold_chars(context_window: u64, reserve_tokens: u64) -> u64 {
    let usable_tokens = context_window.saturating_sub(reserve_tokens);
    usable_tokens * CHARS_PER_TOKEN
}

/// Build the prompt sent to an agent to summarize its conversation.
pub fn build_compaction_prompt() -> String {
    "Please summarize the key points, decisions, and context from our conversation so far. \
     Focus on: active tasks, important decisions made, user preferences learned, and any \
     open questions. Keep it under 2000 characters. Output only the summary, no preamble."
        .to_string()
}

/// Resolve the effective context window for an agent.
pub fn resolve_context_window(agent_context_window: Option<u64>) -> u64 {
    agent_context_window.unwrap_or(DEFAULT_CONTEXT_WINDOW)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_threshold_calculation() {
        // 200k tokens - 40k reserve = 160k usable * 4 chars/token = 640k chars
        let threshold = compaction_threshold_chars(200_000, 40_000);
        assert_eq!(threshold, 640_000);
    }

    #[test]
    fn test_should_compact_below_threshold() {
        assert!(!should_compact(500_000, 200_000, 40_000));
    }

    #[test]
    fn test_should_compact_above_threshold() {
        assert!(should_compact(700_000, 200_000, 40_000));
    }

    #[test]
    fn test_should_compact_at_threshold() {
        // Exactly at threshold — not compacted (must exceed)
        assert!(!should_compact(640_000, 200_000, 40_000));
    }

    #[test]
    fn test_small_context_window() {
        // context_window=1000 tokens, reserve=200, usable=800 * 4 = 3200 chars
        assert!(should_compact(4000, 1000, 200));
        assert!(!should_compact(3000, 1000, 200));
    }

    #[test]
    fn test_resolve_context_window() {
        assert_eq!(resolve_context_window(Some(100_000)), 100_000);
        assert_eq!(resolve_context_window(None), DEFAULT_CONTEXT_WINDOW);
    }

    #[test]
    fn test_compaction_prompt_is_nonempty() {
        let prompt = build_compaction_prompt();
        assert!(!prompt.is_empty());
        assert!(prompt.contains("summarize"));
    }
}
