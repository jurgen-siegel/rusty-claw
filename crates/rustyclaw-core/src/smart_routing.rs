use std::collections::HashMap;

use regex::Regex;

use crate::types::AgentConfig;

/// Match an agent by content keywords.
/// Returns the agent_id of the best-matching agent, or None if no patterns match.
///
/// For each agent with `route_patterns`, checks if any pattern matches the message
/// (case-insensitive, word boundary). If multiple agents match, the one with the
/// highest `route_priority` wins. On tie, returns None (falls back to default).
pub fn match_agent_by_content(
    message: &str,
    agents: &HashMap<String, AgentConfig>,
) -> Option<String> {
    let message_lower = message.to_lowercase();
    let mut best_match: Option<(String, u32, usize)> = None; // (agent_id, priority, match_count)

    for (agent_id, config) in agents {
        let patterns = match &config.route_patterns {
            Some(p) if !p.is_empty() => p,
            _ => continue,
        };

        let mut match_count = 0;
        for pattern in patterns {
            let pattern_lower = pattern.to_lowercase();
            // Word boundary matching: check if the pattern appears as a word
            let escaped = regex::escape(&pattern_lower);
            let re_str = format!(r"(?i)\b{}\b", escaped);
            if let Ok(re) = Regex::new(&re_str) {
                if re.is_match(&message_lower) {
                    match_count += 1;
                }
            }
        }

        if match_count == 0 {
            continue;
        }

        let priority = config.route_priority;
        match &best_match {
            None => {
                best_match = Some((agent_id.clone(), priority, match_count));
            }
            Some((_, best_priority, best_count)) => {
                // Higher priority wins; on tie, more matches wins; on further tie, skip (ambiguous)
                if priority > *best_priority
                    || (priority == *best_priority && match_count > *best_count)
                {
                    best_match = Some((agent_id.clone(), priority, match_count));
                } else if priority == *best_priority && match_count == *best_count {
                    // Ambiguous — return None to fall back to default
                    return None;
                }
            }
        }
    }

    best_match.map(|(id, _, _)| id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_agent(name: &str, patterns: Option<Vec<&str>>, priority: u32) -> AgentConfig {
        AgentConfig {
            name: name.to_string(),
            provider: "anthropic".to_string(),
            model: "sonnet".to_string(),
            working_directory: String::new(),
            reset_policy: String::new(),
            reset_hour: None,
            idle_timeout_minutes: None,
            context_window: None,
            fallbacks: None,
            cross_team_handoffs: true,
            route_patterns: patterns.map(|v| v.into_iter().map(|s| s.to_string()).collect()),
            route_priority: priority,
        }
    }

    #[test]
    fn test_no_patterns_returns_none() {
        let mut agents = HashMap::new();
        agents.insert("coder".to_string(), make_agent("Coder", None, 0));
        assert_eq!(match_agent_by_content("fix the bug", &agents), None);
    }

    #[test]
    fn test_single_agent_match() {
        let mut agents = HashMap::new();
        agents.insert(
            "coder".to_string(),
            make_agent("Coder", Some(vec!["code", "bug", "fix"]), 0),
        );
        agents.insert(
            "writer".to_string(),
            make_agent("Writer", Some(vec!["write", "blog", "article"]), 0),
        );
        assert_eq!(
            match_agent_by_content("please fix the bug", &agents),
            Some("coder".to_string())
        );
    }

    #[test]
    fn test_writer_match() {
        let mut agents = HashMap::new();
        agents.insert(
            "coder".to_string(),
            make_agent("Coder", Some(vec!["code", "bug", "fix"]), 0),
        );
        agents.insert(
            "writer".to_string(),
            make_agent("Writer", Some(vec!["write", "blog", "article"]), 0),
        );
        assert_eq!(
            match_agent_by_content("write a blog article", &agents),
            Some("writer".to_string())
        );
    }

    #[test]
    fn test_no_match() {
        let mut agents = HashMap::new();
        agents.insert(
            "coder".to_string(),
            make_agent("Coder", Some(vec!["code", "bug"]), 0),
        );
        assert_eq!(match_agent_by_content("hello world", &agents), None);
    }

    #[test]
    fn test_priority_wins() {
        let mut agents = HashMap::new();
        agents.insert(
            "coder".to_string(),
            make_agent("Coder", Some(vec!["deploy"]), 10),
        );
        agents.insert(
            "devops".to_string(),
            make_agent("DevOps", Some(vec!["deploy"]), 20),
        );
        assert_eq!(
            match_agent_by_content("deploy the app", &agents),
            Some("devops".to_string())
        );
    }

    #[test]
    fn test_tie_returns_none() {
        let mut agents = HashMap::new();
        agents.insert(
            "coder".to_string(),
            make_agent("Coder", Some(vec!["deploy"]), 5),
        );
        agents.insert(
            "devops".to_string(),
            make_agent("DevOps", Some(vec!["deploy"]), 5),
        );
        assert_eq!(match_agent_by_content("deploy the app", &agents), None);
    }

    #[test]
    fn test_case_insensitive() {
        let mut agents = HashMap::new();
        agents.insert(
            "coder".to_string(),
            make_agent("Coder", Some(vec!["Bug", "FIX"]), 0),
        );
        assert_eq!(
            match_agent_by_content("please fix the BUG", &agents),
            Some("coder".to_string())
        );
    }

    #[test]
    fn test_word_boundary() {
        let mut agents = HashMap::new();
        agents.insert(
            "coder".to_string(),
            make_agent("Coder", Some(vec!["code"]), 0),
        );
        // "encode" contains "code" but not at a word boundary
        assert_eq!(match_agent_by_content("encode the data", &agents), None);
        // "code" as a standalone word
        assert_eq!(
            match_agent_by_content("write some code", &agents),
            Some("coder".to_string())
        );
    }

    #[test]
    fn test_more_matches_wins_same_priority() {
        let mut agents = HashMap::new();
        agents.insert(
            "coder".to_string(),
            make_agent("Coder", Some(vec!["code", "fix", "debug"]), 0),
        );
        agents.insert(
            "reviewer".to_string(),
            make_agent("Reviewer", Some(vec!["code"]), 0),
        );
        // "fix the code and debug" matches 3 for coder, 1 for reviewer
        assert_eq!(
            match_agent_by_content("fix the code and debug", &agents),
            Some("coder".to_string())
        );
    }
}
