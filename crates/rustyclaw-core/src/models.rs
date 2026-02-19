use once_cell::sync::Lazy;
use std::collections::HashMap;

/// Claude (Anthropic) model ID mappings
pub static CLAUDE_MODEL_IDS: Lazy<HashMap<&str, &str>> = Lazy::new(|| {
    let mut m = HashMap::new();
    m.insert("sonnet", "claude-sonnet-4-5");
    m.insert("opus", "claude-opus-4-6");
    m.insert("claude-sonnet-4-5", "claude-sonnet-4-5");
    m.insert("claude-opus-4-6", "claude-opus-4-6");
    m
});

/// Codex (OpenAI) model ID mappings
pub static CODEX_MODEL_IDS: Lazy<HashMap<&str, &str>> = Lazy::new(|| {
    let mut m = HashMap::new();
    m.insert("gpt-5.2", "gpt-5.2");
    m.insert("gpt-5.3-codex", "gpt-5.3-codex");
    m
});

/// OpenCode model ID mappings (provider/model format)
pub static OPENCODE_MODEL_IDS: Lazy<HashMap<&str, &str>> = Lazy::new(|| {
    let mut m = HashMap::new();
    m.insert("opencode/claude-opus-4-6", "opencode/claude-opus-4-6");
    m.insert("opencode/claude-sonnet-4-5", "opencode/claude-sonnet-4-5");
    m.insert("opencode/gemini-3-flash", "opencode/gemini-3-flash");
    m.insert("opencode/gemini-3-pro", "opencode/gemini-3-pro");
    m.insert("opencode/glm-5", "opencode/glm-5");
    m.insert("opencode/kimi-k2.5", "opencode/kimi-k2.5");
    m.insert("opencode/kimi-k2.5-free", "opencode/kimi-k2.5-free");
    m.insert("opencode/minimax-m2.5", "opencode/minimax-m2.5");
    m.insert("opencode/minimax-m2.5-free", "opencode/minimax-m2.5-free");
    m.insert("anthropic/claude-opus-4-6", "anthropic/claude-opus-4-6");
    m.insert(
        "anthropic/claude-sonnet-4-5",
        "anthropic/claude-sonnet-4-5",
    );
    m.insert("openai/gpt-5.2", "openai/gpt-5.2");
    m.insert("openai/gpt-5.3-codex", "openai/gpt-5.3-codex");
    m.insert(
        "openai/gpt-5.3-codex-spark",
        "openai/gpt-5.3-codex-spark",
    );
    // Shorthand aliases
    m.insert("sonnet", "opencode/claude-sonnet-4-5");
    m.insert("opus", "opencode/claude-opus-4-6");
    m
});

/// Resolve the model ID for Claude (Anthropic).
/// Falls back to the raw model string if no mapping found.
pub fn resolve_claude_model(model: &str) -> String {
    CLAUDE_MODEL_IDS
        .get(model)
        .map(|s| s.to_string())
        .unwrap_or_else(|| model.to_string())
}

/// Resolve the model ID for Codex (OpenAI).
/// Falls back to the raw model string if no mapping found.
pub fn resolve_codex_model(model: &str) -> String {
    CODEX_MODEL_IDS
        .get(model)
        .map(|s| s.to_string())
        .unwrap_or_else(|| model.to_string())
}

/// Resolve the model ID for OpenCode.
/// Falls back to the raw model string if no mapping found.
pub fn resolve_opencode_model(model: &str) -> String {
    OPENCODE_MODEL_IDS
        .get(model)
        .map(|s| s.to_string())
        .unwrap_or_else(|| model.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_claude_shortnames() {
        assert_eq!(resolve_claude_model("sonnet"), "claude-sonnet-4-5");
        assert_eq!(resolve_claude_model("opus"), "claude-opus-4-6");
    }

    #[test]
    fn test_resolve_claude_full_ids() {
        assert_eq!(
            resolve_claude_model("claude-sonnet-4-5"),
            "claude-sonnet-4-5"
        );
        assert_eq!(
            resolve_claude_model("claude-opus-4-6"),
            "claude-opus-4-6"
        );
    }

    #[test]
    fn test_resolve_claude_unknown_passthrough() {
        assert_eq!(resolve_claude_model("custom-model"), "custom-model");
    }

    #[test]
    fn test_resolve_codex() {
        assert_eq!(resolve_codex_model("gpt-5.2"), "gpt-5.2");
        assert_eq!(resolve_codex_model("gpt-5.3-codex"), "gpt-5.3-codex");
        assert_eq!(resolve_codex_model("unknown"), "unknown");
    }

    #[test]
    fn test_resolve_opencode() {
        assert_eq!(
            resolve_opencode_model("sonnet"),
            "opencode/claude-sonnet-4-5"
        );
        assert_eq!(
            resolve_opencode_model("opencode/gemini-3-flash"),
            "opencode/gemini-3-flash"
        );
        assert_eq!(resolve_opencode_model("custom"), "custom");
    }
}
